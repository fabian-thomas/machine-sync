//! The sync worker: daemonizes (unless foreground), performs an initial full sync,
//! then watches the source directory and syncs changes incrementally.

use crate::heartbeat::{Heartbeat, Phase};
use crate::spec::SyncSpec;
use crate::transport::{self, source_basename};
use crate::{ignoreset, notify, registry};
use anyhow::{Context, Result};
use notify_debouncer_full::new_debouncer;
use notify_debouncer_full::notify::{RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

/// Maximum size of a worker log file before it is rotated in place.
const LOG_CAP_BYTES: u64 = 5 * 1024 * 1024;
/// Amount of the most recent log retained when rotating.
const LOG_KEEP_BYTES: usize = 1024 * 1024;

/// When set (daemonized workers), `log()` writes to this file with size-capped
/// rotation. When unset (foreground), `log()` prints to stdout.
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Result of attempting to daemonize.
enum Daemonized {
    Parent,
    Child,
}

/// Double-fork into the background and detach from the controlling terminal.
fn daemonize() -> Result<Daemonized> {
    use nix::unistd::{fork, setsid, ForkResult};
    match unsafe { fork() }.context("first fork failed")? {
        ForkResult::Parent { .. } => return Ok(Daemonized::Parent),
        ForkResult::Child => {}
    }
    setsid().context("setsid failed")?;
    match unsafe { fork() }.context("second fork failed")? {
        ForkResult::Parent { .. } => std::process::exit(0),
        ForkResult::Child => {}
    }
    Ok(Daemonized::Child)
}

/// Redirect stdio: stdin from /dev/null, stdout/stderr appended to the log file.
fn redirect_io(log: &PathBuf) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let devnull = std::fs::OpenOptions::new().read(true).open("/dev/null")?;
    let logf = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .with_context(|| format!("opening log file {}", log.display()))?;
    nix::unistd::dup2(devnull.as_raw_fd(), 0).ok();
    nix::unistd::dup2(logf.as_raw_fd(), 1).ok();
    nix::unistd::dup2(logf.as_raw_fd(), 2).ok();
    Ok(())
}

/// Entry point for a worker. When `foreground` is false this daemonizes and the
/// original (parent) process returns immediately; only the detached child runs
/// the sync loop (and never returns until terminated).
pub fn run(id: u64, spec: &SyncSpec, foreground: bool) -> Result<()> {
    if !foreground {
        match daemonize()? {
            Daemonized::Parent => return Ok(()),
            Daemonized::Child => {}
        }
        let log = crate::paths::log_file(id)?;
        redirect_io(&log)?;
        let _ = LOG_PATH.set(log);
    }
    // Record our real pid now that we are the worker process. A PID always fits in
    // an i32 on Linux, so the cast cannot wrap.
    #[allow(clippy::cast_possible_wrap)]
    update_pid(id, std::process::id() as i32)?;
    run_loop(id, spec)?;
    Ok(())
}

fn update_pid(id: u64, pid: i32) -> Result<()> {
    let mut guard = registry::RegistryGuard::lock()?;
    if let Some(e) = guard.registry.entries.iter_mut().find(|e| e.id == id) {
        e.pid = Some(pid);
    }
    guard.save()
}

fn run_loop(id: u64, spec: &SyncSpec) -> Result<()> {
    let root = spec
        .dir
        .canonicalize()
        .with_context(|| format!("source directory {} not found", spec.dir.display()))?;
    let tty = entry_tty(id);
    let basename = source_basename(&root);

    log(&format!(
        "starting sync #{id} {} -> {}",
        root.display(),
        spec.targets
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ));

    // Initial full sync.
    let included = ignoreset::enumerate(&root)?;
    do_sync(id, spec, tty.as_deref(), &basename, &included, true);

    // Watch for changes.
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(spec.debounce_ms()), None, tx)
        .context("creating file watcher")?;
    debouncer
        .watcher()
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", root.display()))?;

    for res in rx {
        let events = match res {
            Ok(evs) => evs,
            Err(errs) => {
                for e in errs {
                    log(&format!("watch error: {e}"));
                }
                continue;
            }
        };
        // Gather candidate changed paths (absolute).
        let mut candidates: HashSet<PathBuf> = HashSet::new();
        for ev in events {
            for p in &ev.paths {
                candidates.insert(p.clone());
            }
        }
        if candidates.is_empty() {
            continue;
        }
        // Refresh the included set and intersect (drops ignored & deleted files).
        let included: HashSet<PathBuf> = ignoreset::enumerate(&root)?.into_iter().collect();
        let mut changed: Vec<PathBuf> = candidates
            .iter()
            .filter_map(|abs| {
                abs.strip_prefix(&root)
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
            .filter(|rel| included.contains(rel))
            .collect();
        changed.sort();
        changed.dedup();
        if changed.is_empty() {
            continue;
        }
        do_sync(id, spec, tty.as_deref(), &basename, &changed, false);
    }
    Ok(())
}

fn do_sync(
    id: u64,
    spec: &SyncSpec,
    tty: Option<&str>,
    basename: &str,
    files: &[PathBuf],
    initial: bool,
) {
    if files.is_empty() {
        let _ = Heartbeat {
            phase: Phase::Idle,
            last_sync_at: Some(registry::now_secs()),
            files: 0,
            bytes: 0,
            last_error: None,
        }
        .write(id);
        return;
    }
    let _ = Heartbeat {
        phase: Phase::Syncing,
        last_sync_at: None,
        files: files.len() as u64,
        bytes: 0,
        last_error: None,
    }
    .write(id);

    match transport::sync_files(spec, files) {
        Ok(results) => {
            let failed: Vec<String> = results
                .iter()
                .filter(|r| !r.success)
                .map(|r| format!("{}: {}", r.target, r.message))
                .collect();
            let ok_targets: Vec<String> = results
                .iter()
                .filter(|r| r.success)
                .map(|r| r.target.to_string())
                .collect();

            if failed.is_empty() {
                log(&format!(
                    "{} synced {} file(s) -> {}",
                    if initial { "initial" } else { "change" },
                    files.len(),
                    ok_targets.join(", ")
                ));
            } else {
                for f in &failed {
                    log(&format!("FAILED {f}"));
                }
            }

            let title = format!("{} → {}", basename, spec.targets_display());
            let body = file_list_body(files);
            notify::notify(spec, tty, &title, &body);

            let _ = Heartbeat {
                phase: Phase::Idle,
                last_sync_at: Some(registry::now_secs()),
                files: files.len() as u64,
                bytes: 0,
                last_error: failed.first().cloned(),
            }
            .write(id);
        }
        Err(e) => {
            log(&format!("sync error: {e:#}"));
            let _ = Heartbeat {
                phase: Phase::Idle,
                last_sync_at: Some(registry::now_secs()),
                files: 0,
                bytes: 0,
                last_error: Some(e.to_string()),
            }
            .write(id);
        }
    }
}

fn file_list_body(files: &[PathBuf]) -> String {
    const MAX: usize = 12;
    let mut lines: Vec<String> = files
        .iter()
        .take(MAX)
        .map(|f| f.to_string_lossy().into_owned())
        .collect();
    if files.len() > MAX {
        lines.push(format!("… and {} more", files.len() - MAX));
    }
    lines.join("\n")
}

fn entry_tty(id: u64) -> Option<String> {
    registry::read_unlocked()
        .ok()?
        .entries
        .into_iter()
        .find(|e| e.id == id)
        .and_then(|e| e.tty)
}

fn log(msg: &str) {
    let now = registry::now_secs();
    if let Some(path) = LOG_PATH.get() {
        rotate_if_needed(path);
    }
    println!("[{now}] {msg}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// Rotate the log in place (keeping the most recent tail) when it grows past the
/// cap. Truncating in place preserves the inode, so the daemon's redirected
/// stdout/stderr file descriptors stay valid and keep appending.
fn rotate_if_needed(path: &std::path::Path) {
    let len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if len <= LOG_CAP_BYTES {
        return;
    }
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let out = truncate_tail(&data, LOG_KEEP_BYTES);
    let _ = std::fs::write(path, &out);
}

/// Keep the most recent `keep` bytes of `data`, trimmed to a line boundary, with a
/// truncation marker prepended.
fn truncate_tail(data: &[u8], keep: usize) -> Vec<u8> {
    let start = data.len().saturating_sub(keep);
    // Advance to the next line boundary so we don't keep a partial line.
    let start = data[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(start, |p| start + p + 1);
    let mut out = Vec::with_capacity(data.len() - start + 16);
    out.extend_from_slice(b"[log truncated]\n");
    out.extend_from_slice(&data[start..]);
    out
}

impl SyncSpec {
    fn targets_display(&self) -> String {
        self.targets
            .iter()
            .map(|t| t.host.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_tail;

    #[test]
    fn keeps_recent_whole_lines_with_marker() {
        let mut data = Vec::new();
        for i in 0..1000 {
            data.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let out = truncate_tail(&data, 50);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("[log truncated]\n"));
        // Smaller than the original and ends with the most recent line.
        assert!(s.len() < data.len());
        assert!(s.trim_end().ends_with("line 999"), "got: {s:?}");
        // No partial first content line (every content line is intact).
        for line in s.lines().skip(1) {
            assert!(line.starts_with("line "), "partial line: {line:?}");
        }
    }
}
