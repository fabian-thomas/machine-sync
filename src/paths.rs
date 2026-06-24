//! XDG path helpers for config, durable state, runtime (heartbeat) and logs.

use anyhow::{Context, Result};
use std::path::PathBuf;

const APP: &str = "machine-sync";

/// Directory for durable state (the registry). `$XDG_STATE_HOME/machine-sync`.
pub fn state_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs_home().map(|h| h.join(".local/state")))
        .context("could not determine a state directory (set XDG_STATE_HOME or HOME)")?;
    let dir = base.join(APP);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating state dir {}", dir.display()))?;
    Ok(dir)
}

/// Directory for ephemeral runtime files (heartbeats). Prefers `$XDG_RUNTIME_DIR`
/// (tmpfs); falls back to the state dir if it is unset.
pub fn runtime_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let dir = match base {
        Some(b) => b.join(APP),
        None => state_dir()?.join("run"),
    };
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating runtime dir {}", dir.display()))?;
    Ok(dir)
}

/// Directory for per-sync log files.
pub fn log_dir() -> Result<PathBuf> {
    let dir = state_dir()?.join("logs");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating log dir {}", dir.display()))?;
    Ok(dir)
}

/// The global config file path: `$XDG_CONFIG_HOME/machine-sync/config.toml`.
pub fn global_config_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs_home().map(|h| h.join(".config")))?;
    Some(base.join(APP).join("config.toml"))
}

/// The registry file inside the state dir.
pub fn registry_file() -> Result<PathBuf> {
    Ok(state_dir()?.join("registry.json"))
}

/// The registry lock file inside the state dir.
pub fn registry_lock_file() -> Result<PathBuf> {
    Ok(state_dir()?.join("registry.lock"))
}

/// The heartbeat file for a given sync id.
pub fn heartbeat_file(id: u64) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("{id}.json")))
}

/// The log file for a given sync id.
pub fn log_file(id: u64) -> Result<PathBuf> {
    Ok(log_dir()?.join(format!("{id}.log")))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}
