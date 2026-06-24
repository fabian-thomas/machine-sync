//! Per-worker heartbeat files (ephemeral, lock-free, single-writer).
//!
//! Each worker owns `$XDG_RUNTIME_DIR/machine-sync/<id>.json` and updates it via
//! atomic temp+rename on phase transitions only. `status` overlays this on the
//! durable registry to show live "syncing / last sync" information.

use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Syncing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub phase: Phase,
    #[serde(default)]
    pub last_sync_at: Option<u64>,
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl Heartbeat {
    pub fn write(&self, id: u64) -> Result<()> {
        let path = paths::heartbeat_file(id)?;
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string(self)?;
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.flush()?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Read a worker's heartbeat, if present and parseable.
pub fn read(id: u64) -> Option<Heartbeat> {
    let path = paths::heartbeat_file(id).ok()?;
    let buf = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&buf).ok()
}

/// Remove a worker's heartbeat file (best effort).
pub fn remove(id: u64) {
    if let Ok(path) = paths::heartbeat_file(id) {
        let _ = std::fs::remove_file(path);
    }
}
