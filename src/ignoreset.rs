//! File-set enumeration honoring nested `.gitignore` (via the `ignore` crate) plus
//! force-sync ("un-ignore") patterns from `.msyncignore`/`.ldignore` and the
//! `# nomsyncignore` / `# noldignore` markers.

use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::WalkBuilder;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Names treated as additional ignore files (same syntax as `.gitignore`).
const CUSTOM_IGNORE_FILES: &[&str] = &[".msyncignore", ".ldignore"];

/// Marker comments after which the remaining patterns are *un-ignored* (force-synced).
const FORCE_MARKERS: &[&str] = &["nomsyncignore", "noldignore"];

/// Files scanned (at the root) for force-sync patterns.
const FORCE_PATTERN_FILES: &[&str] = &[".gitignore", ".msyncignore", ".ldignore"];

/// Enumerate the set of files (paths relative to `root`) that should be synced.
///
/// This is the union of two passes:
///   1. an `ignore`-crate walk that respects nested `.gitignore`, `.msyncignore`,
///      `.ldignore`, `.git/info/exclude` and the global gitignore, plus the
///      `extra` patterns from the config (anchored at `root`);
///   2. a plain walk whose results are filtered to only the force-sync patterns.
///
/// The union guarantees the force markers *reverse the ignoring* of matching files
/// (adding them back in) without turning into a whitelist that would exclude
/// everything else. It also means an explicit force-sync pattern beats the
/// config-level `extra` patterns.
pub fn enumerate(root: &Path, extra: &[String]) -> Result<Vec<PathBuf>> {
    let mut set: BTreeSet<PathBuf> = BTreeSet::new();

    // Pass 1: gitignore-respecting walk.
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) // include dotfiles and the .git directory
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .parents(true);
    for name in CUSTOM_IGNORE_FILES {
        builder.add_custom_ignore_filename(name);
    }
    if let Some(extra) = build_extra_matcher(root, extra)? {
        // `filter_entry` prunes whole subtrees, so a directory pattern such as
        // `.git/rebase-merge` skips everything beneath it. Depth 0 is the root
        // itself, which must never be filtered out.
        builder.filter_entry(move |dent| {
            if dent.depth() == 0 {
                return true;
            }
            let is_dir = dent.file_type().is_some_and(|t| t.is_dir());
            !extra.matched(dent.path(), is_dir).is_ignore()
        });
    }
    let walk = builder.build();
    for dent in walk {
        let Ok(dent) = dent else {
            continue;
        };
        if dent.file_type().is_none_or(|t| t.is_dir()) {
            continue;
        }
        let path = dent.path();
        if let Ok(rel) = path.strip_prefix(root) {
            set.insert(rel.to_path_buf());
        }
    }

    // Pass 2: force-sync patterns re-include otherwise-ignored files.
    if let Some(force) = build_force_matcher(root)? {
        let mut plain = WalkBuilder::new(root);
        plain
            .standard_filters(false)
            .hidden(false)
            .require_git(false);
        for dent in plain.build() {
            let Ok(dent) = dent else {
                continue;
            };
            let is_dir = dent.file_type().is_some_and(|t| t.is_dir());
            if is_dir {
                continue;
            }
            let path = dent.path();
            if force.matched(path, false).is_ignore() {
                if let Ok(rel) = path.strip_prefix(root) {
                    set.insert(rel.to_path_buf());
                }
            }
        }
    }

    Ok(set.into_iter().collect())
}

/// Build a matcher for the config-supplied ignore patterns, anchored at `root`.
fn build_extra_matcher(root: &Path, patterns: &[String]) -> Result<Option<Gitignore>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut b = GitignoreBuilder::new(root);
    for p in patterns {
        b.add_line(None, p)
            .with_context(|| format!("invalid ignore pattern {p:?}"))?;
    }
    Ok(Some(b.build()?))
}

/// Build a matcher of the force-sync ("un-ignore") patterns, or `None` if there are
/// none. Patterns come from:
///   * `.msyncignore` lines starting with `!`, and
///   * any patterns following a `# nomsyncignore` / `# noldignore` marker in the
///     root-level `.gitignore`, `.msyncignore` or `.ldignore`.
fn build_force_matcher(root: &Path) -> Result<Option<Gitignore>> {
    let patterns = collect_force_patterns(root);
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut b = GitignoreBuilder::new(root);
    for p in &patterns {
        // Added as ignore patterns so a match reports `.is_ignore()`.
        b.add_line(None, p)
            .with_context(|| format!("invalid force-sync pattern {p:?}"))?;
    }
    Ok(Some(b.build()?))
}

fn collect_force_patterns(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for fname in FORCE_PATTERN_FILES {
        let path = root.join(fname);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let is_msyncignore = *fname == ".msyncignore";
        let mut after_marker = false;
        for raw in content.lines() {
            let line = raw.trim();
            if is_marker(line) {
                after_marker = true;
                continue;
            }
            if after_marker {
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                out.push(strip_negation(line));
            } else if is_msyncignore {
                if let Some(rest) = line.strip_prefix('!') {
                    let rest = rest.trim();
                    if !rest.is_empty() {
                        out.push(rest.to_string());
                    }
                }
            }
        }
    }
    out
}

fn strip_negation(line: &str) -> String {
    line.strip_prefix('!').unwrap_or(line).trim().to_string()
}

fn is_marker(line: &str) -> bool {
    if !line.starts_with('#') {
        return false;
    }
    FORCE_MARKERS.iter().any(|m| line.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, s: &str) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, s).unwrap();
    }

    fn names(root: &Path) -> Vec<String> {
        names_with(root, &[])
    }

    fn names_with(root: &Path, extra: &[&str]) -> Vec<String> {
        let extra: Vec<String> = extra.iter().map(|s| (*s).to_string()).collect();
        let mut v: Vec<String> = enumerate(root, &extra)
            .unwrap()
            .into_iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn nested_gitignore_applies_at_depth() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("keep.txt"), "x");
        write(&root.join("sub/log.log"), "x");
        write(&root.join("sub/code.rs"), "x");
        write(&root.join("sub/.gitignore"), "*.log\n");
        let n = names(root);
        assert!(n.contains(&"keep.txt".to_string()));
        assert!(n.contains(&"sub/code.rs".to_string()));
        assert!(
            !n.contains(&"sub/log.log".to_string()),
            "nested ignore failed: {n:?}"
        );
    }

    #[test]
    fn git_dir_is_synced() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("a.txt"), "x");
        write(&root.join(".git/config"), "x");
        let n = names(root);
        assert!(n.contains(&"a.txt".to_string()), "{n:?}");
        assert!(n.contains(&".git/config".to_string()), "{n:?}");
    }

    #[test]
    fn marker_unignores_without_whitelisting() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("app.js"), "x");
        write(&root.join("dist/bundle.js"), "x");
        write(&root.join("secret.key"), "x");
        write(
            &root.join(".gitignore"),
            "dist/\nsecret.key\n# nomsyncignore\ndist/bundle.js\n",
        );
        let n = names(root);
        // Normal files still included (not a whitelist).
        assert!(n.contains(&"app.js".to_string()), "{n:?}");
        // Force-synced file inside an ignored dir is re-included.
        assert!(n.contains(&"dist/bundle.js".to_string()), "{n:?}");
        // Other ignored file stays ignored.
        assert!(!n.contains(&"secret.key".to_string()), "{n:?}");
    }

    #[test]
    fn msyncignore_negation_forces_sync() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("build/out.bin"), "x");
        write(&root.join(".gitignore"), "build/\n");
        write(&root.join(".msyncignore"), "!build/out.bin\n");
        let n = names(root);
        assert!(n.contains(&"build/out.bin".to_string()), "{n:?}");
    }

    #[test]
    fn config_patterns_exclude_files() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("keep.txt"), "x");
        write(&root.join(".direnv/bin/tool"), "x");
        let n = names_with(root, &[".direnv"]);
        assert!(n.contains(&"keep.txt".to_string()), "{n:?}");
        assert!(!n.contains(&".direnv/bin/tool".to_string()), "{n:?}");
    }

    #[test]
    fn config_patterns_prune_git_transients() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join(".git/config"), "x");
        write(&root.join(".git/index.lock"), "x");
        write(&root.join(".git/ORIG_HEAD"), "x");
        write(&root.join(".git/MERGE_MSG"), "x");
        write(&root.join(".git/refs/heads/main.lock"), "x");
        write(&root.join(".git/rebase-merge/head-name"), "x");
        write(&root.join(".git/BISECT_LOG"), "x");
        let n = names_with(
            root,
            &[
                ".git/*.lock",
                ".git/**/*.lock",
                ".git/*_HEAD",
                ".git/MERGE_MSG",
                ".git/BISECT_*",
                ".git/rebase-merge",
            ],
        );
        assert!(n.contains(&".git/config".to_string()), "{n:?}");
        for gone in [
            ".git/index.lock",
            ".git/ORIG_HEAD",
            ".git/MERGE_MSG",
            ".git/refs/heads/main.lock",
            ".git/rebase-merge/head-name",
            ".git/BISECT_LOG",
        ] {
            assert!(!n.contains(&gone.to_string()), "{gone} not ignored: {n:?}");
        }
    }

    #[test]
    fn force_sync_overrides_config_pattern() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join(".direnv/bin/tool"), "x");
        write(&root.join(".direnv/other"), "x");
        write(&root.join(".msyncignore"), "!.direnv/bin/tool\n");
        let n = names_with(root, &[".direnv"]);
        assert!(n.contains(&".direnv/bin/tool".to_string()), "{n:?}");
        assert!(!n.contains(&".direnv/other".to_string()), "{n:?}");
    }

    #[test]
    fn later_config_pattern_can_negate_an_earlier_one() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("logs/a.log"), "x");
        write(&root.join("logs/keep.log"), "x");
        let n = names_with(root, &["logs/*.log", "!logs/keep.log"]);
        assert!(n.contains(&"logs/keep.log".to_string()), "{n:?}");
        assert!(!n.contains(&"logs/a.log".to_string()), "{n:?}");
    }

    #[test]
    fn invalid_pattern_is_reported() {
        let tmp = tempdir();
        let root = tmp.path();
        write(&root.join("a.txt"), "x");
        assert!(enumerate(root, &["a{b,c".to_string()]).is_err());
    }

    // Minimal tempdir without external crates.
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TempDir {
        let mut p = std::env::temp_dir();
        let n = format!(
            "msync-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        p.push(n);
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
