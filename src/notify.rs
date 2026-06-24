//! Layered notifications: Linux desktop (notify-send / D-Bus), terminal OSC 9, and
//! WSL -> Windows toast. When `spec.notify` is empty the backends are auto-detected.

use crate::spec::SyncSpec;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Send a notification for a completed sync.
///
/// Backends are resolved synchronously, but the actual delivery (which may spawn
/// external processes such as `notify-send` or `powershell.exe`) runs on a detached
/// thread so a slow backend never stalls the caller's sync loop.
pub fn notify(spec: &SyncSpec, tty: Option<&str>, title: &str, body: &str) {
    let backends = resolve_backends(spec, tty);
    if backends.is_empty() {
        return;
    }
    let title = title.to_string();
    let body = body.to_string();
    let tty = tty.map(str::to_string);
    std::thread::spawn(move || {
        for b in backends {
            match b {
                Backend::Desktop => desktop(&title, &body),
                Backend::Osc => {
                    if let Some(t) = &tty {
                        osc(t, &title, &body);
                    }
                }
                Backend::Wsl => wsl(&title, &body),
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Desktop,
    Osc,
    Wsl,
}

fn resolve_backends(spec: &SyncSpec, tty: Option<&str>) -> Vec<Backend> {
    if !spec.notify.is_empty() {
        return spec
            .notify
            .iter()
            .filter_map(|s| match s.to_ascii_lowercase().as_str() {
                "desktop" | "dbus" | "notify-send" => Some(Backend::Desktop),
                "osc" | "terminal" => Some(Backend::Osc),
                "wsl" | "windows" => Some(Backend::Wsl),
                _ => None,
            })
            .collect();
    }
    // Auto-detect: pick the single best available backend. OSC is only a fallback
    // for headless/remote terminal sessions without a desktop or WSL bridge, to
    // avoid duplicate notifications and stray escape sequences on capable terminals.
    let mut out = Vec::new();
    if is_wsl() {
        out.push(Backend::Wsl);
    } else if have("notify-send") {
        out.push(Backend::Desktop);
    } else if tty.is_some() {
        out.push(Backend::Osc);
    }
    out
}

fn desktop(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=msync")
        .arg("--")
        .arg(title)
        .arg(body)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Emit an OSC 9 desktop-notification escape sequence to the captured terminal.
fn osc(tty: &str, title: &str, body: &str) {
    let mut msg = String::from(title);
    if !body.is_empty() {
        msg.push_str(": ");
        msg.push_str(body);
    }
    let msg = sanitize(&msg);
    // OSC 9 ; <text> BEL
    let seq = format!("\x1b]9;{msg}\x07");
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(Path::new(tty)) {
        let _ = f.write_all(seq.as_bytes());
    }
}

fn wsl(title: &str, body: &str) {
    if have("wsl-notify-send") {
        let _ = Command::new("wsl-notify-send")
            .arg("--category")
            .arg("msync")
            .arg(format!("{title}\n{body}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return;
    }
    // Fall back to PowerShell balloon-style toast.
    let ps = format!(
        "[void][System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms'); \
         $n=New-Object System.Windows.Forms.NotifyIcon; \
         $n.Icon=[System.Drawing.SystemIcons]::Information; $n.Visible=$true; \
         $n.ShowBalloonTip(5000,'{}','{}',[System.Windows.Forms.ToolTipIcon]::Info); \
         Start-Sleep -Seconds 6",
        ps_escape(title),
        ps_escape(body)
    );
    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Detect running under WSL.
pub fn is_wsl() -> bool {
    if std::env::var_os("WSL_DISTRO_NAME").is_some() {
        return true;
    }
    false
}

fn have(bin: &str) -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(bin).is_file() {
                return true;
            }
        }
    }
    false
}

fn sanitize(s: &str) -> String {
    // Strip control characters that could corrupt the terminal.
    s.chars().filter(|c| !c.is_control()).collect()
}

fn ps_escape(s: &str) -> String {
    s.replace('\'', "''")
}
