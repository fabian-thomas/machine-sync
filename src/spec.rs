//! The resolved description of a single sync (source, targets, options).

use crate::target::Target;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A fully resolved sync, ready to be run by a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSpec {
    /// Optional name (set when started from a config entry).
    pub name: Option<String>,
    /// Absolute source directory.
    pub dir: PathBuf,
    /// One or more destinations.
    pub targets: Vec<Target>,
    /// Mirror deletions to the remote (off by default).
    #[serde(default)]
    pub delete: bool,
    /// Pass `-n` to rsync.
    #[serde(default)]
    pub dry_run: bool,
    /// Extra raw rsync arguments.
    #[serde(default)]
    pub extra_rsync: Vec<String>,
    /// Notification backends to use; empty means "auto-detect".
    #[serde(default)]
    pub notify: Vec<String>,
    /// Debounce window in milliseconds for coalescing change events. `None` uses
    /// [`DEFAULT_DEBOUNCE_MS`].
    #[serde(default)]
    pub debounce_ms: Option<u64>,
}

/// Default debounce window for coalescing filesystem change events.
pub const DEFAULT_DEBOUNCE_MS: u64 = 700;

impl SyncSpec {
    /// The effective debounce window.
    pub fn debounce_ms(&self) -> u64 {
        self.debounce_ms.unwrap_or(DEFAULT_DEBOUNCE_MS)
    }
}
