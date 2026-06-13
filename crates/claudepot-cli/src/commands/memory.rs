//! `claudepot memory …` verb-group module — list / view / log
//! per-project memory artifacts.
//!
//! Read-only by design (per the v1 plan) — no edit verbs. The display
//! mirrors what the GUI's MemoryPane shows so users get the same data
//! whether they're looking through the desktop app or the terminal.
//!
//! Per the commands.md rule for nouns with ≥3 verbs, each verb lives
//! one-per-file under `commands/memory/<verb>.rs`. This entry point
//! holds the shared imports, the path/log resolution helpers consumed
//! by multiple verbs, the submodule declarations, and the `pub use`
//! re-exports `main.rs` depends on.

use crate::AppContext;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, Utc};
use claudepot_core::memory_log::{ChangeQuery, DiffOmitReason, MemoryLog};
use claudepot_core::memory_view::{
    enumerate_project_memory, read_memory_content, MemoryFileRole, MemoryFileSummary,
    ProjectMemoryAnchor,
};
use claudepot_core::paths::claudepot_data_dir;
use claudepot_core::project_helpers::resolve_path;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Resolve `--project <PATH>` (or default to the current working dir)
/// to an absolute, OS-canonical path. Used by every memory verb.
fn resolve_project(project: Option<&str>) -> Result<PathBuf> {
    let raw = match project {
        Some(p) => p.to_string(),
        None => std::env::current_dir()?.to_string_lossy().into_owned(),
    };
    let resolved =
        resolve_path(&raw).map_err(|e| anyhow!("resolve project path {}: {}", raw, e))?;
    Ok(PathBuf::from(resolved))
}

fn open_log() -> Result<MemoryLog> {
    let path = claudepot_data_dir().join("memory_changes.db");
    MemoryLog::open(&path).with_context(|| format!("open memory log at {}", path.display()))
}

fn role_label(role: MemoryFileRole) -> &'static str {
    match role {
        MemoryFileRole::ClaudeMdProject => "project · CLAUDE.md",
        MemoryFileRole::ClaudeMdProjectLocal => "project · .claude/CLAUDE.md",
        MemoryFileRole::AutoMemoryIndex => "auto-memory · MEMORY.md",
        MemoryFileRole::AutoMemoryTopic => "auto-memory · topic",
        MemoryFileRole::KairosLog => "auto-memory · daily log",
        MemoryFileRole::ClaudeMdGlobal => "global · CLAUDE.md",
    }
}

fn format_ns_relative(ns: i64) -> String {
    let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let delta_ns = (now_ns - ns).max(0);
    let secs = delta_ns / 1_000_000_000;
    if secs < 60 {
        return format!("{}s ago", secs);
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{}m ago", mins);
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("{}h ago", hours);
    }
    let days = hours / 24;
    if days < 30 {
        return format!("{}d ago", days);
    }
    // Fall back to absolute date for older entries.
    let secs = ns / 1_000_000_000;
    let nsec = (ns % 1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nsec)
        .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn resolve_memory_file(project_root: &Path, file: &str) -> Result<PathBuf> {
    let p = Path::new(file);
    if p.is_absolute() && p.exists() {
        return Ok(p.to_path_buf());
    }
    let anchor = ProjectMemoryAnchor::for_project(project_root);
    let candidates: Vec<PathBuf> = vec![
        project_root.join(file),
        project_root.join(".claude").join(file),
        anchor.auto_memory_dir.join(file),
        claudepot_core::paths::claude_config_dir().join(file),
    ];
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(anyhow!(
        "memory file not found: {}. Try `claudepot memory list` to see available files.",
        file
    ))
}

// Submodule declarations. Verb implementations live one-per-file
// under `commands/memory/<verb>.rs`; helpers above are visible to
// each submodule via `use super::*;` (child modules reach the
// parent's private items in Rust).
mod list;
mod log;
mod view;

// Re-exports — main.rs's match block dispatches on these names.
pub use list::list;
pub use log::log;
pub use view::view;
