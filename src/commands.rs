//! Implementations of the CLI subcommands.

use crate::cli::{LogsArgs, SelectArgs, StartArgs, StatusArgs};
use crate::config::Config;
use crate::registry::{Entry, RegistryGuard, State};
use crate::spec::SyncSpec;
use crate::target::{looks_like_target, Target};
use crate::worker;
use crate::{heartbeat, ignoreset, paths, registry, transport, ui};
use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use std::io::{IsTerminal, Read, Write};
use std::path::Path;

pub fn status(args: &StatusArgs) -> Result<()> {
    let reg = registry::read_unlocked()?;
    ui::render(&reg, args.all, args.json)
}

pub fn start(args: &StartArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let config = Config::load(&cwd)?;

    // Determine the set of specs to start.
    let mut specs: Vec<SyncSpec> = Vec::new();
    if args.all {
        specs = config.all_syncs()?;
    } else if args.names.is_empty() {
        specs.push(adhoc_or_default(&config, &cwd, args)?);
    } else {
        let mut adhoc_targets: Vec<Target> = Vec::new();
        for name in &args.names {
            if looks_like_target(name) {
                adhoc_targets.push(Target::parse(name)?);
                continue;
            }
            match config.resolve(name) {
                Ok(s) => specs.extend(s),
                Err(_) => adhoc_targets.push(Target::parse(name)?),
            }
        }
        if !adhoc_targets.is_empty() {
            specs.push(adhoc_spec(adhoc_targets, &cwd, args, &config)?);
        }
    }

    if specs.is_empty() {
        bail!("nothing to start");
    }

    // A `--debounce` flag overrides the debounce of every started sync (CLI takes
    // precedence over config and the built-in default).
    if let Some(ms) = args.debounce {
        for spec in &mut specs {
            spec.debounce_ms = Some(ms);
        }
    }

    if args.debug && specs.len() > 1 {
        bail!(
            "--debug runs a single sync in the foreground; you selected {}",
            specs.len()
        );
    }
    if args.once {
        for spec in &specs {
            run_once(spec)?;
        }
        return Ok(());
    }

    for spec in specs {
        start_one(spec, args.debug)?;
    }
    Ok(())
}

/// Build the spec for `msync start` with no positional names: either the project
/// default sync, or (if there is no project config) an error guiding the user.
fn adhoc_or_default(config: &Config, _cwd: &Path, _args: &StartArgs) -> Result<SyncSpec> {
    config.default_sync()
}

fn adhoc_spec(
    targets: Vec<Target>,
    cwd: &Path,
    args: &StartArgs,
    config: &Config,
) -> Result<SyncSpec> {
    let dir = args.dir.clone().unwrap_or_else(|| cwd.to_path_buf());
    let dir = dir
        .canonicalize()
        .with_context(|| format!("source directory {} not found", dir.display()))?;
    Ok(SyncSpec {
        name: args.name.clone(),
        dir,
        targets,
        delete: args.delete,
        dry_run: args.dry_run,
        extra_rsync: args.rsync_args.clone(),
        notify: args.notify.clone(),
        debounce_ms: args.debounce,
        ignore: config.base_ignore(),
    })
}

/// Register and launch a single sync (daemonized unless `foreground`).
fn start_one(mut spec: SyncSpec, foreground: bool) -> Result<()> {
    spec.dir = spec
        .dir
        .canonicalize()
        .with_context(|| format!("source directory {} not found", spec.dir.display()))?;
    let tty = capture_tty();

    let id;
    {
        let mut guard = RegistryGuard::lock()?;
        // Replace any existing sync of the same source directory.
        let stale: Vec<Entry> = guard
            .registry
            .entries
            .iter()
            .filter(|e| e.dir == spec.dir)
            .cloned()
            .collect();
        for e in &stale {
            if let Some(pid) = e.pid {
                kill(pid);
            }
            heartbeat::remove(e.id);
            let _ = std::fs::remove_file(&e.log);
            print_replaced(e);
        }
        guard.registry.entries.retain(|e| e.dir != spec.dir);

        id = guard.alloc_id();
        let log = paths::log_file(id)?;
        guard.registry.entries.push(Entry {
            id,
            name: spec.name.clone(),
            dir: spec.dir.clone(),
            targets: spec.targets.clone(),
            state: State::Running,
            pid: None,
            started_at: registry::now_secs(),
            log,
            tty,
            delete: spec.delete,
            dry_run: spec.dry_run,
            extra_rsync: spec.extra_rsync.clone(),
            notify: spec.notify.clone(),
            debounce_ms: spec.debounce_ms,
            ignore: spec.ignore.clone(),
        });
        guard.save()?;
    }

    print_started(id, &spec, foreground);

    worker::run(id, &spec, foreground)?;
    Ok(())
}

/// A one-shot sync (initial sync only, no registry/daemon).
fn run_once(spec: &SyncSpec) -> Result<()> {
    let root = spec
        .dir
        .canonicalize()
        .with_context(|| format!("source directory {} not found", spec.dir.display()))?;
    let spec = SyncSpec {
        dir: root.clone(),
        ..spec.clone()
    };
    let files = ignoreset::enumerate(&root, &spec.ignore)?;
    let results = transport::sync_files(&spec, &files)?;
    let basename = transport::source_basename(&root);
    let failures: Vec<_> = results.iter().filter(|r| !r.success).collect();
    for r in &results {
        if r.success {
            println!(
                "{} synced {} file(s) -> {}",
                ok_mark(),
                files.len(),
                r.target
            );
        } else {
            eprintln!("{} {} → {}: {}", fail_mark(), basename, r.target, r.message);
        }
    }
    if !failures.is_empty() {
        bail!("{} target(s) failed", failures.len());
    }
    Ok(())
}

pub fn stop(args: &SelectArgs) -> Result<()> {
    let mut guard = RegistryGuard::lock()?;
    let ids = select_ids(&guard, args)?;
    if ids.is_empty() {
        bail!("no matching syncs");
    }
    for id in &ids {
        let desc = guard
            .registry
            .entries
            .iter()
            .find(|e| e.id == *id)
            .map(entry_desc)
            .unwrap_or_default();
        if let Some(e) = guard.registry.entries.iter().find(|e| e.id == *id) {
            if let Some(pid) = e.pid {
                kill(pid);
            }
            let _ = std::fs::remove_file(&e.log);
        }
        heartbeat::remove(*id);
        println!("{} stopped #{id}{desc}", ok_mark());
    }
    guard.registry.entries.retain(|e| !ids.contains(&e.id));
    guard.save()?;
    Ok(())
}

pub fn pause(args: &SelectArgs) -> Result<()> {
    let mut guard = RegistryGuard::lock()?;
    let ids = select_ids(&guard, args)?;
    if ids.is_empty() {
        bail!("no matching syncs");
    }
    for id in &ids {
        let mut desc = String::new();
        if let Some(e) = guard.registry.entries.iter_mut().find(|e| e.id == *id) {
            desc = entry_desc(e);
            if let Some(pid) = e.pid {
                kill(pid);
            }
            e.pid = None;
            e.state = State::Paused;
        }
        heartbeat::remove(*id);
        let mark = if std::io::stdout().is_terminal() {
            "⏸".yellow().to_string()
        } else {
            "⏸".to_string()
        };
        println!("{mark} paused #{id}{desc}  (resume with: msync resume {id})");
    }
    guard.save()?;
    Ok(())
}

pub fn resume(args: &SelectArgs) -> Result<()> {
    // Collect the specs/ids to resume while holding the lock briefly.
    let to_resume: Vec<(u64, SyncSpec)> = {
        let guard = RegistryGuard::lock()?;
        if args.selectors.is_empty() && !args.all {
            // Resume everything that was running but is no longer alive.
            guard
                .registry
                .entries
                .iter()
                .filter(|e| e.state == State::Running && !e.is_alive())
                .map(|e| (e.id, e.spec()))
                .collect()
        } else {
            let ids = select_ids(&guard, args)?;
            guard
                .registry
                .entries
                .iter()
                .filter(|e| ids.contains(&e.id) && !e.is_alive())
                .map(|e| (e.id, e.spec()))
                .collect()
        }
    };

    if to_resume.is_empty() {
        println!("Nothing to resume.");
        return Ok(());
    }

    let tty = capture_tty();
    for (id, spec) in to_resume {
        {
            let mut guard = RegistryGuard::lock()?;
            if let Some(e) = guard.registry.entries.iter_mut().find(|e| e.id == id) {
                e.state = State::Running;
                e.pid = None;
                e.tty.clone_from(&tty);
                e.started_at = registry::now_secs();
            }
            guard.save()?;
        }
        print_started(id, &spec, false);
        worker::run(id, &spec, false)?;
    }
    Ok(())
}

pub fn restart(args: &SelectArgs) -> Result<()> {
    // Stop the workers but keep the entries, then resume them.
    let ids: Vec<u64> = {
        let mut guard = RegistryGuard::lock()?;
        let ids = select_ids(&guard, args)?;
        for id in &ids {
            if let Some(e) = guard.registry.entries.iter_mut().find(|e| e.id == *id) {
                if let Some(pid) = e.pid {
                    kill(pid);
                }
                e.pid = None;
            }
            heartbeat::remove(*id);
        }
        guard.save()?;
        ids
    };
    if ids.is_empty() {
        bail!("no matching syncs");
    }
    let selectors = ids.iter().map(std::string::ToString::to_string).collect();
    resume(&SelectArgs {
        selectors,
        all: false,
    })
}

pub fn logs(args: &LogsArgs) -> Result<()> {
    let reg = registry::read_unlocked()?;
    let entry = reg
        .find(&args.selector)
        .with_context(|| format!("no sync matching {:?}", args.selector))?;
    let path = entry.log.clone();
    if !path.exists() {
        bail!("no log file yet at {}", path.display());
    }
    let mut file =
        std::fs::File::open(&path).with_context(|| format!("opening log {}", path.display()))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    print!("{buf}");
    std::io::stdout().flush().ok();

    if !args.follow {
        return Ok(());
    }
    let mut pos = file.metadata()?.len();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let len = std::fs::metadata(&path)?.len();
        if len > pos {
            use std::io::{Seek, SeekFrom};
            let mut f = std::fs::File::open(&path)?;
            f.seek(SeekFrom::Start(pos))?;
            let mut more = String::new();
            f.read_to_string(&mut more)?;
            print!("{more}");
            std::io::stdout().flush().ok();
            pos = len;
        }
    }
}

// ---- helpers ----

fn select_ids(guard: &RegistryGuard, args: &SelectArgs) -> Result<Vec<u64>> {
    if args.all {
        return Ok(guard.registry.entries.iter().map(|e| e.id).collect());
    }
    if args.selectors.is_empty() {
        bail!("specify one or more ids/names, or use --all");
    }
    let mut ids = Vec::new();
    for sel in &args.selectors {
        match guard.registry.find(sel) {
            Some(e) => ids.push(e.id),
            None => bail!("no sync matching {sel:?}"),
        }
    }
    Ok(ids)
}

fn kill(pid: i32) {
    use nix::sys::signal::{kill as nkill, Signal};
    use nix::unistd::Pid;
    nkill(Pid::from_raw(pid), Signal::SIGTERM).ok();
}

/// Determine the controlling terminal path for OSC notifications.
fn capture_tty() -> Option<String> {
    use std::os::unix::io::AsRawFd;
    for fd in [
        std::io::stderr().as_raw_fd(),
        std::io::stdout().as_raw_fd(),
        0,
    ] {
        if let Ok(path) = nix::unistd::ttyname(fd) {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

fn entry_desc(e: &Entry) -> String {
    let name = e.name.clone().map(|n| format!(" {n}")).unwrap_or_default();
    let targets = e
        .targets
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{name}  {} → {}",
        shorten(&e.dir.to_string_lossy()),
        targets
    )
}

fn shorten(p: &str) -> String {
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()) {
        if let Some(rest) = p.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    p.to_string()
}

fn print_replaced(e: &Entry) {
    let name = e.name.clone().map(|n| format!(" {n}")).unwrap_or_default();
    let targets = e
        .targets
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let msg = format!("↻ replacing #{}{name} (was → {targets})", e.id);
    if std::io::stderr().is_terminal() {
        eprintln!("{}", msg.yellow());
    } else {
        eprintln!("{msg}");
    }
}

fn print_started(id: u64, spec: &SyncSpec, foreground: bool) {
    let name = spec
        .name
        .clone()
        .map(|n| format!(" {n}"))
        .unwrap_or_default();
    let targets = spec
        .targets
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let src = spec.dir.to_string_lossy();
    eprintln!("{} started #{id}{name}  {} → {}", ok_mark(), src, targets);
    if !foreground {
        let hint = format!("  follow with: msync logs {id} -f");
        if std::io::stderr().is_terminal() {
            eprintln!("{}", hint.dimmed());
        } else {
            eprintln!("{hint}");
        }
    }
}

fn ok_mark() -> String {
    if std::io::stderr().is_terminal() {
        "✓".green().to_string()
    } else {
        "✓".to_string()
    }
}

fn fail_mark() -> String {
    if std::io::stderr().is_terminal() {
        "✗".red().to_string()
    } else {
        "✗".to_string()
    }
}
