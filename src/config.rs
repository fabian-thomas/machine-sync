//! TOML configuration: a global file plus an optional per-project `.msync.toml`.
//!
//! ```toml
//! # default = "project"        # optional override of the default sync
//!
//! [sync.dotfiles]
//! dir = "~/dotfiles"
//! targets = ["laptop", "server:/etc/dots"]
//!
//! [group.work]
//! members = ["dotfiles", "project"]
//! ```

use crate::spec::SyncSpec;
use crate::target::Target;
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const PROJECT_CONFIG: &str = ".msync.toml";

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    sync: IndexMap<String, SyncEntry>,
    #[serde(default)]
    group: IndexMap<String, GroupEntry>,
}

#[derive(Debug, Deserialize)]
struct SyncEntry {
    dir: String,
    targets: Vec<String>,
    #[serde(default)]
    delete: bool,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    notify: Vec<String>,
    #[serde(default)]
    rsync_args: Vec<String>,
    #[serde(default)]
    debounce_ms: Option<u64>,
    #[serde(default)]
    default: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GroupEntry {
    members: Vec<String>,
}

/// A loaded config file together with the directory it lives in (for resolving
/// relative `dir` paths).
struct Loaded {
    file: ConfigFile,
    base: PathBuf,
}

/// The merged view of project + global configuration.
pub struct Config {
    project: Option<Loaded>,
    global: Option<Loaded>,
}

impl Config {
    /// Load the global config and discover a project `.msync.toml` by walking up
    /// from `start_dir`.
    pub fn load(start_dir: &Path) -> Result<Config> {
        let global = match crate::paths::global_config_file() {
            Some(p) if p.is_file() => Some(load_file(&p)?),
            _ => None,
        };
        let project = find_project_config(start_dir)
            .map(|p| load_file(&p))
            .transpose()?;
        Ok(Config { project, global })
    }

    /// Resolve a single named sync (project takes precedence over global).
    pub fn resolve_sync(&self, name: &str) -> Option<Result<SyncSpec>> {
        for src in [&self.project, &self.global].into_iter().flatten() {
            if let Some(entry) = src.file.sync.get(name) {
                return Some(entry.to_spec(name, &src.base));
            }
        }
        None
    }

    /// Resolve a group to its member sync names (project precedence).
    pub fn resolve_group(&self, name: &str) -> Option<Vec<String>> {
        for src in [&self.project, &self.global].into_iter().flatten() {
            if let Some(g) = src.file.group.get(name) {
                return Some(g.members.clone());
            }
        }
        None
    }

    /// Resolve a name that may be a group or a sync into one or more specs.
    pub fn resolve(&self, name: &str) -> Result<Vec<SyncSpec>> {
        if let Some(members) = self.resolve_group(name) {
            let mut out = Vec::new();
            for m in members {
                out.push(
                    self.resolve_sync(&m).with_context(|| {
                        format!("group {name:?} references unknown sync {m:?}")
                    })??,
                );
            }
            return Ok(out);
        }
        match self.resolve_sync(name) {
            Some(spec) => Ok(vec![spec?]),
            None => bail!("no sync or group named {name:?} in config"),
        }
    }

    /// The default sync to start when `msync start` is given no arguments: an
    /// explicit `default = "name"`, else a per-sync `default = true`, else the
    /// first-defined sync in the project config.
    pub fn default_sync(&self) -> Result<SyncSpec> {
        let proj = self
            .project
            .as_ref()
            .context("no project .msync.toml found in this directory or its parents")?;
        if let Some(name) = &proj.file.default {
            return self
                .resolve_sync(name)
                .with_context(|| format!("default = {name:?} but no such sync"))?;
        }
        if let Some((name, entry)) = proj.file.sync.iter().find(|(_, e)| e.default == Some(true)) {
            return entry.to_spec(name, &proj.base);
        }
        let (name, entry) = proj
            .file
            .sync
            .first()
            .context("project config defines no syncs")?;
        entry.to_spec(name, &proj.base)
    }

    /// All syncs across project + global (project overrides global by name).
    pub fn all_syncs(&self) -> Result<Vec<SyncSpec>> {
        let mut seen: IndexMap<String, SyncSpec> = IndexMap::new();
        for src in [&self.global, &self.project].into_iter().flatten() {
            for (name, entry) in &src.file.sync {
                seen.insert(name.clone(), entry.to_spec(name, &src.base)?);
            }
        }
        if seen.is_empty() {
            bail!("no syncs defined in any config");
        }
        Ok(seen.into_values().collect())
    }

    /// Names of all syncs and groups (for shell completion), each with a short
    /// description. Project entries take precedence over global ones.
    pub fn completion_names(&self) -> Vec<(String, String)> {
        let mut seen: IndexMap<String, String> = IndexMap::new();
        for src in [&self.global, &self.project].into_iter().flatten() {
            for (name, entry) in &src.file.sync {
                let desc = format!("sync → {}", entry.targets.join(", "));
                seen.insert(name.clone(), desc);
            }
            for (name, group) in &src.file.group {
                let desc = format!("group [{}]", group.members.join(", "));
                seen.insert(name.clone(), desc);
            }
        }
        seen.into_iter().collect()
    }
}

impl SyncEntry {
    fn to_spec(&self, name: &str, base: &Path) -> Result<SyncSpec> {
        let dir = expand_dir(&self.dir, base);
        let targets = self
            .targets
            .iter()
            .map(|t| Target::parse(t))
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("in sync {name:?}"))?;
        if targets.is_empty() {
            bail!("sync {name:?} has no targets");
        }
        Ok(SyncSpec {
            name: Some(name.to_string()),
            dir,
            targets,
            delete: self.delete,
            dry_run: self.dry_run,
            extra_rsync: self.rsync_args.clone(),
            notify: self.notify.clone(),
            debounce_ms: self.debounce_ms,
        })
    }
}

fn load_file(path: &Path) -> Result<Loaded> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    let file: ConfigFile =
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok(Loaded { file, base })
}

fn find_project_config(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        let candidate = d.join(PROJECT_CONFIG);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent().map(std::path::Path::to_path_buf);
    }
    None
}

/// Expand a leading `~` and resolve relative paths against `base`.
fn expand_dir(dir: &str, base: &Path) -> PathBuf {
    let expanded = if let Some(rest) = dir.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(rest)
        } else {
            PathBuf::from(dir)
        }
    } else if dir == "~" {
        std::env::var_os("HOME").map_or_else(|| PathBuf::from(dir), PathBuf::from)
    } else {
        PathBuf::from(dir)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    }
}
