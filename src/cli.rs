//! Command-line interface definition (clap derive).

use clap::{Args, Parser, Subcommand};
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};
use std::path::PathBuf;

/// Dynamic completion candidates for sync selectors (ids and names of registered
/// syncs), each annotated with its source → targets for the shell's help column.
fn sync_candidates() -> Vec<CompletionCandidate> {
    let reg = crate::registry::read_unlocked().unwrap_or_default();
    let mut out = Vec::new();
    for e in &reg.entries {
        let targets = e
            .targets
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let help = format!("{} → {}", e.dir.display(), targets);
        out.push(CompletionCandidate::new(e.id.to_string()).help(Some(help.clone().into())));
        if let Some(name) = &e.name {
            out.push(CompletionCandidate::new(name.clone()).help(Some(help.into())));
        }
    }
    out
}

/// Completion candidates for `start`: config sync and group names.
fn start_candidates() -> Vec<CompletionCandidate> {
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let Ok(cfg) = crate::config::Config::load(&cwd) else {
        return Vec::new();
    };
    cfg.completion_names()
        .into_iter()
        .map(|(name, help)| CompletionCandidate::new(name).help(Some(help.into())))
        .collect()
}

#[derive(Debug, Parser)]
#[command(
    name = "msync",
    version,
    about = "Live-sync a directory to one or more machines over rsync/ssh.",
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List syncs and their status (default command).
    Status(StatusArgs),
    /// Start a sync from config name/group, an ad-hoc target, or the project default.
    Start(StartArgs),
    /// Stop one or more syncs (removes them from the registry).
    Stop(SelectArgs),
    /// Pause one or more syncs (keeps them in the registry).
    Pause(SelectArgs),
    /// Resume paused syncs; with no arguments, resume everything previously running.
    Resume(SelectArgs),
    /// Restart one or more syncs.
    Restart(SelectArgs),
    /// Show (or follow) a sync's log.
    Logs(LogsArgs),
    /// Print shell completion setup instructions.
    Completions(CompletionsArgs),
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Show extra columns (pid, uptime, files, log path).
    #[arg(short, long)]
    pub all: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // independent CLI flags, not a state machine
pub struct StartArgs {
    /// Config sync/group names, or ad-hoc targets (`host[:path]`). Empty starts the
    /// project default sync.
    #[arg(add = ArgValueCandidates::new(start_candidates))]
    pub names: Vec<String>,

    /// Start every sync defined in the resolved config.
    #[arg(long)]
    pub all: bool,

    /// Source directory to sync (defaults to the current directory) for ad-hoc syncs.
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,

    /// Label for an ad-hoc sync.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Do an initial sync and exit (no watching).
    #[arg(long, alias = "no-watch")]
    pub once: bool,

    /// Pass -n to rsync (no changes made).
    #[arg(long)]
    pub dry_run: bool,

    /// Mirror local deletions to the remote (off by default).
    #[arg(long)]
    pub delete: bool,

    /// Comma-separated notification backends (desktop,osc,wsl). Default: auto-detect.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub notify: Vec<String>,

    /// Debounce window in milliseconds for coalescing change events (default: 700).
    #[arg(long, value_name = "MS")]
    pub debounce: Option<u64>,

    /// Run in the foreground instead of daemonizing (for debugging).
    #[arg(long, alias = "foreground")]
    pub debug: bool,

    /// Extra arguments passed straight to rsync (after `--`).
    #[arg(last = true)]
    pub rsync_args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SelectArgs {
    /// Sync ids or names to act on.
    #[arg(add = ArgValueCandidates::new(sync_candidates))]
    pub selectors: Vec<String>,
    /// Act on all syncs.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Sync id or name.
    #[arg(add = ArgValueCandidates::new(sync_candidates))]
    pub selector: String,
    /// Follow the log (like `tail -f`).
    #[arg(short, long)]
    pub follow: bool,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}
