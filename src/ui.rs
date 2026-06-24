//! Rendering of `msync status` (colorized table + JSON).

use crate::heartbeat::{self, Phase};
use crate::registry::{Entry, Registry, State};
use anyhow::Result;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use owo_colors::OwoColorize;
use std::io::IsTerminal;

/// Live, derived status of a single entry (registry + heartbeat overlay).
struct Row {
    entry: Entry,
    phase: Option<Phase>,
    last_sync_at: Option<u64>,
    files: u64,
    last_error: Option<String>,
    alive: bool,
}

fn build_rows(reg: &Registry) -> Vec<Row> {
    reg.entries
        .iter()
        .map(|e| {
            let hb = heartbeat::read(e.id);
            Row {
                entry: e.clone(),
                phase: hb.as_ref().map(|h| h.phase),
                last_sync_at: hb.as_ref().and_then(|h| h.last_sync_at),
                files: hb.as_ref().map_or(0, |h| h.files),
                last_error: hb.and_then(|h| h.last_error),
                alive: e.is_alive(),
            }
        })
        .collect()
}

pub fn render(reg: &Registry, all: bool, json: bool) -> Result<()> {
    if json {
        return render_json(reg);
    }
    let color = use_color();
    let rows = build_rows(reg);

    if rows.is_empty() {
        println!("No syncs. Start one with `msync start <host[:path]>` or `msync start <name>`.");
        return Ok(());
    }

    let live = rows
        .iter()
        .filter(|r| r.alive && r.entry.state == State::Running)
        .count();
    let paused = rows
        .iter()
        .filter(|r| r.entry.state == State::Paused)
        .count();
    let summary = format!("{} total · {} live · {} paused", rows.len(), live, paused);
    if color {
        println!("{}", summary.bold());
    } else {
        println!("{summary}");
    }

    let mut table = Table::new();
    table
        .load_preset(comfy_table::presets::UTF8_BORDERS_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic);

    let mut header = vec!["#", "STATE", "NAME", "SOURCE", "TARGETS", "LAST SYNC"];
    if all {
        header.extend_from_slice(&["PID", "UPTIME", "FILES", "LOG"]);
    }
    table.set_header(header);

    for r in &rows {
        let (glyph, label, scolor) = state_display(r);
        let state_cell = if color {
            Cell::new(format!("{glyph} {label}")).fg(scolor)
        } else {
            Cell::new(format!("{glyph} {label}"))
        };
        let name = r.entry.name.clone().unwrap_or_else(|| "—".to_string());
        let targets = r
            .entry
            .targets
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let last = match r.last_sync_at {
            Some(t) => ago(t),
            None if r.phase == Some(Phase::Syncing) => "syncing…".to_string(),
            None => "—".to_string(),
        };

        let mut cells = vec![
            Cell::new(r.entry.id),
            state_cell,
            Cell::new(name),
            Cell::new(shorten_home(&r.entry.dir.to_string_lossy())),
            Cell::new(targets),
            Cell::new(last),
        ];
        if all {
            cells.push(Cell::new(
                r.entry.pid.map_or_else(|| "—".into(), |p| p.to_string()),
            ));
            cells.push(Cell::new(if r.alive {
                ago(r.entry.started_at)
            } else {
                "—".into()
            }));
            cells.push(Cell::new(r.files));
            cells.push(Cell::new(r.entry.log.to_string_lossy()));
        }
        table.add_row(cells);
    }

    println!("{table}");

    // Surface the most recent error, if any.
    for r in &rows {
        if let Some(err) = &r.last_error {
            let msg = format!("#{} last error: {}", r.entry.id, err);
            if color {
                eprintln!("{}", msg.red());
            } else {
                eprintln!("{msg}");
            }
        }
    }
    Ok(())
}

fn render_json(reg: &Registry) -> Result<()> {
    let rows = build_rows(reg);
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.entry.id,
                "name": r.entry.name,
                "dir": r.entry.dir,
                "targets": r.entry.targets,
                "state": r.entry.state,
                "alive": r.alive,
                "pid": r.entry.pid,
                "phase": r.phase,
                "last_sync_at": r.last_sync_at,
                "files": r.files,
                "started_at": r.entry.started_at,
                "log": r.entry.log,
                "last_error": r.last_error,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&arr)?);
    Ok(())
}

fn state_display(r: &Row) -> (&'static str, &'static str, Color) {
    if r.entry.state == State::Paused {
        return ("⏸", "paused", Color::Yellow);
    }
    if !r.alive {
        return ("✗", "dead", Color::Red);
    }
    match r.phase {
        Some(Phase::Syncing) => ("⟳", "syncing", Color::Cyan),
        _ => ("●", "live", Color::Green),
    }
}

fn use_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

/// Human "N ago" for an epoch-seconds timestamp.
pub fn ago(then: u64) -> String {
    let now = crate::registry::now_secs();
    if then == 0 || then > now {
        return "just now".to_string();
    }
    let secs = now - then;
    if secs < 1 {
        return "just now".to_string();
    }
    let d = humantime::format_duration(std::time::Duration::from_secs(coarse(secs)));
    format!("{d} ago")
}

/// Round a duration to a single coarse unit for compact display.
fn coarse(secs: u64) -> u64 {
    match secs {
        s if s < 60 => s,
        s if s < 3600 => (s / 60) * 60,
        s if s < 86400 => (s / 3600) * 3600,
        s => (s / 86400) * 86400,
    }
}

fn shorten_home(p: &str) -> String {
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()) {
        if let Some(rest) = p.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    p.to_string()
}
