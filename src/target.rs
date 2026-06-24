//! Parsing and rendering of sync targets (`host`, `host:/abs`, `host:~/rel`, `host:rel`).

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A sync destination: an ssh `host` and an optional remote `path`.
///
/// When `path` is `None` the destination defaults to `host:~/<basename>` where
/// `<basename>` is the final component of the synced directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub host: String,
    /// Remote path. `None` means "default to the source directory's basename".
    pub path: Option<String>,
}

impl Target {
    /// Parse a target string.
    ///
    /// * `host`        -> host with default path (`~/basename`)
    /// * `host:`       -> host with default path (trailing colon forces host parsing)
    /// * `host:/abs`   -> absolute remote path
    /// * `host:~/rel`  -> path relative to remote home (shell-expanded remotely)
    /// * `host:rel`    -> path relative to remote login dir (rsync default)
    pub fn parse(s: &str) -> Result<Target> {
        if s.is_empty() {
            bail!("empty target");
        }
        let (host, path) = match s.split_once(':') {
            Some((h, p)) => {
                let path = if p.is_empty() {
                    None
                } else {
                    Some(p.to_string())
                };
                (h.to_string(), path)
            }
            None => (s.to_string(), None),
        };
        if host.is_empty() {
            bail!("target {s:?} has an empty host");
        }
        if host.contains('/') {
            bail!("target host {host:?} must not contain '/'");
        }
        Ok(Target { host, path })
    }

    /// The remote rsync destination string for a given source basename, including a
    /// trailing slash so rsync treats it as a directory.
    pub fn rsync_dest(&self, basename: &str) -> String {
        let path = self.path.clone().unwrap_or_else(|| basename.to_string());
        let path = path.trim_end_matches('/');
        format!("{}:{}/", self.host, path)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(p) => write!(f, "{}:{}", self.host, p),
            None => write!(f, "{}", self.host),
        }
    }
}

/// Heuristic: does this CLI argument look like an explicit target rather than a
/// config sync/group name? An argument containing a `:` is always a target.
pub fn looks_like_target(arg: &str) -> bool {
    arg.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_host_defaults_to_basename() {
        let t = Target::parse("laptop").unwrap();
        assert_eq!(t.host, "laptop");
        assert_eq!(t.path, None);
        assert_eq!(t.rsync_dest("project"), "laptop:project/");
    }

    #[test]
    fn trailing_colon_is_default_path() {
        let t = Target::parse("laptop:").unwrap();
        assert_eq!(t.path, None);
        assert_eq!(t.rsync_dest("proj"), "laptop:proj/");
    }

    #[test]
    fn absolute_and_relative_paths() {
        assert_eq!(
            Target::parse("srv:/srv/site").unwrap().rsync_dest("x"),
            "srv:/srv/site/"
        );
        assert_eq!(
            Target::parse("srv:~/site").unwrap().rsync_dest("x"),
            "srv:~/site/"
        );
        assert_eq!(
            Target::parse("srv:site").unwrap().rsync_dest("x"),
            "srv:site/"
        );
    }

    #[test]
    fn rejects_bad_hosts() {
        assert!(Target::parse("").is_err());
        assert!(Target::parse("a/b:/x").is_err());
        assert!(Target::parse(":/x").is_err());
    }

    #[test]
    fn target_detection() {
        assert!(looks_like_target("host:/path"));
        assert!(looks_like_target("host:"));
        assert!(!looks_like_target("myname"));
    }
}
