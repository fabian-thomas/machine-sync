//! The durable, flock-guarded registry of syncs.
//!
//! The registry holds only configuration and lifecycle state and is written solely
//! on lifecycle transitions (start/stop/pause/resume). Transient "currently syncing"
//! information lives in per-worker heartbeat files (see [`crate::heartbeat`]).

use crate::paths;
use crate::spec::SyncSpec;
use crate::target::Target;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Lifecycle state of a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Paused,
}

/// A single persisted sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub name: Option<String>,
    pub dir: PathBuf,
    pub targets: Vec<Target>,
    pub state: State,
    pub pid: Option<i32>,
    pub started_at: u64,
    pub log: PathBuf,
    #[serde(default)]
    pub tty: Option<String>,
    #[serde(default)]
    pub delete: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub extra_rsync: Vec<String>,
    #[serde(default)]
    pub notify: Vec<String>,
    #[serde(default)]
    pub debounce_ms: Option<u64>,
}

impl Entry {
    /// Reconstruct the runnable spec from this entry.
    pub fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: self.name.clone(),
            dir: self.dir.clone(),
            targets: self.targets.clone(),
            delete: self.delete,
            dry_run: self.dry_run,
            extra_rsync: self.extra_rsync.clone(),
            notify: self.notify.clone(),
            debounce_ms: self.debounce_ms,
        }
    }

    /// Is the worker process for this entry currently alive?
    pub fn is_alive(&self) -> bool {
        match self.pid {
            Some(pid) => pid_alive(pid),
            None => false,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Registry {
    pub fn find(&self, sel: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| matches_selector(e, sel))
    }
}

fn matches_selector(e: &Entry, sel: &str) -> bool {
    if let Ok(id) = sel.parse::<u64>() {
        if e.id == id {
            return true;
        }
    }
    e.name.as_deref() == Some(sel)
}

/// A handle holding the exclusive registry lock for the duration of a mutation.
pub struct RegistryGuard {
    _lock: File,
    file: File,
    pub registry: Registry,
}

impl RegistryGuard {
    /// Acquire the exclusive lock and load the current registry.
    pub fn lock() -> Result<RegistryGuard> {
        let lock_path = paths::registry_lock_file()?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("opening lock file {}", lock_path.display()))?;
        lock.lock_exclusive().context("acquiring registry lock")?;

        let reg_path = paths::registry_file()?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&reg_path)
            .with_context(|| format!("opening registry {}", reg_path.display()))?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        let registry: Registry = if buf.trim().is_empty() {
            Registry::default()
        } else {
            serde_json::from_str(&buf).context("parsing registry json")?
        };
        Ok(RegistryGuard {
            _lock: lock,
            file,
            registry,
        })
    }

    /// Allocate the next sync id.
    pub fn alloc_id(&mut self) -> u64 {
        self.registry.next_id += 1;
        self.registry.next_id
    }

    /// Persist the in-memory registry back to disk (atomic-ish truncate+write).
    pub fn save(&mut self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.registry)?;
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(json.as_bytes())?;
        self.file.flush()?;
        Ok(())
    }
}

/// Read the registry without holding the lock (for read-only views like `status`).
pub fn read_unlocked() -> Result<Registry> {
    let reg_path = paths::registry_file()?;
    let buf = match std::fs::read_to_string(&reg_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(e).context("reading registry"),
    };
    if buf.trim().is_empty() {
        return Ok(Registry::default());
    }
    serde_json::from_str(&buf).context("parsing registry json")
}

/// Is a process with this pid alive? Uses `kill(pid, 0)`.
pub fn pid_alive(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    if pid <= 0 {
        return false;
    }
    matches!(
        kill(Pid::from_raw(pid), None),
        Ok(()) | Err(nix::errno::Errno::EPERM)
    )
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
