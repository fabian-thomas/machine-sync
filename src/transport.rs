//! rsync-over-ssh transport: fan out a file list to one or more targets.

use crate::spec::SyncSpec;
use crate::target::Target;
use anyhow::{Context, Result};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Outcome of an rsync run to a single target.
pub struct TargetResult {
    pub target: Target,
    pub success: bool,
    pub message: String,
}

/// Sync the given relative `files` from `spec.dir` to every target. An empty
/// `files` slice is a no-op (returns an empty result set). Targets are synced
/// concurrently (one thread per target).
pub fn sync_files(spec: &SyncSpec, files: &[PathBuf]) -> Result<Vec<TargetResult>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let basename = source_basename(&spec.dir);

    // Build a single NUL-separated buffer shared by every target.
    let mut list: Vec<u8> = Vec::new();
    for f in files {
        list.extend_from_slice(f.as_os_str().as_bytes());
        list.push(0);
    }
    let list = std::sync::Arc::new(list);

    // One target is the common case; avoid spawning a thread for it.
    if spec.targets.len() == 1 {
        let target = &spec.targets[0];
        let dest = target.rsync_dest(&basename);
        let (success, message) = run_rsync(&spec.dir, &dest, &list, spec)
            .with_context(|| format!("running rsync to {target}"))?;
        return Ok(vec![TargetResult {
            target: target.clone(),
            success,
            message,
        }]);
    }

    std::thread::scope(|scope| {
        let handles: Vec<_> = spec
            .targets
            .iter()
            .map(|target| {
                let dest = target.rsync_dest(&basename);
                let list = std::sync::Arc::clone(&list);
                let source = &spec.dir;
                let spec_ref = spec;
                scope.spawn(move || match run_rsync(source, &dest, &list, spec_ref) {
                    Ok((success, message)) => TargetResult {
                        target: target.clone(),
                        success,
                        message,
                    },
                    Err(e) => TargetResult {
                        target: target.clone(),
                        success: false,
                        message: e.to_string(),
                    },
                })
            })
            .collect();
        Ok(handles.into_iter().map(|h| h.join().unwrap()).collect())
    })
}

fn run_rsync(source: &Path, dest: &str, list: &[u8], spec: &SyncSpec) -> Result<(bool, String)> {
    let mut cmd = Command::new("rsync");
    cmd.arg("-a").arg("--files-from=-").arg("--from0");
    if spec.dry_run {
        cmd.arg("-n");
    }
    if spec.delete {
        cmd.arg("--delete");
    }
    for a in &spec.extra_rsync {
        cmd.arg(a);
    }
    // Source must end with a slash so files-from paths are relative to its contents.
    let mut src = source.as_os_str().to_os_string();
    src.push("/");
    cmd.arg(src).arg(dest);

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .context("failed to spawn rsync (is it installed?)")?;
    child
        .stdin
        .take()
        .context("rsync stdin unavailable")?
        .write_all(list)
        .context("writing file list to rsync")?;
    let out = child.wait_with_output().context("waiting for rsync")?;
    let success = out.status.success();
    let message = if success {
        String::new()
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        stderr
            .trim()
            .lines()
            .last()
            .unwrap_or("rsync failed")
            .to_string()
    };
    Ok((success, message))
}

/// The final path component of the source directory, used as the default remote dir.
pub fn source_basename(dir: &Path) -> String {
    dir.file_name()
        .map_or_else(|| "sync".to_string(), |n| n.to_string_lossy().into_owned())
}
