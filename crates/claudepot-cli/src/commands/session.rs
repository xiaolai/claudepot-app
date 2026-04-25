//! `claudepot session …` verb-group module.
//!
//! Per the commands.md rule for nouns with ≥3 verbs, the session
//! handlers live in submodules under `commands/session/<verb>.rs`.
//! This entry point holds the imports, the shared formatting helpers
//! consumed by multiple verbs, the submodule declarations, and the
//! `pub use` re-exports `main.rs`'s match block depends on.

//! CC session transcript management — move, list-orphans, adopt-orphan,
//! inspect (view/chunks/tools/classify/subagents/phases/context),
//! export, search, worktree grouping.
//!
//! All handlers are thin wrappers around `claudepot_core`. No business
//! logic lives here (per `.claude/rules/architecture.md`).

use crate::AppContext;
use anyhow::{bail, Context, Result};
use claudepot_core::paths;
use claudepot_core::session::{read_session_detail, read_session_detail_at_path, SessionDetail};
use claudepot_core::session_move::{
    adopt_orphan_project, detect_orphaned_projects, move_session, AdoptReport, MoveSessionOpts,
    MoveSessionReport, OrphanedProject,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// CC stores `.claude.json` at `$HOME/.claude.json` — a sibling of
/// `~/.claude/`, not inside. Central accessor so CLI and Tauri agree.

fn claude_json_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// List projects whose internal `cwd` no longer exists on disk. These
/// are the adoption candidates — typically sessions orphaned by
/// `git worktree remove`.

fn truncate_start(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    // Keep the tail (more informative for paths). Prefix with "…".
    let skip = s.chars().count() - (max - 1);
    let kept: String = s.chars().skip(skip).collect();
    format!("…{kept}")
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.2} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}


fn print_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("json serialization failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Prune + trash
// ---------------------------------------------------------------------------


fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GiB", b / GB)
    } else if b >= MB {
        format!("{:.1} MiB", b / MB)
    } else if b >= KB {
        format!("{:.1} KiB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_ts_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn atty_like() -> bool {
    // Used by `trash empty` to refuse without `--yes`. On a non-TTY
    // (pipe, CI, test harness) we don't demand the confirmation.
    std::io::IsTerminal::is_terminal(&std::io::stdin())
}

fn parse_duration(s: &str) -> Result<std::time::Duration> {
    let t = s.trim();
    if t.is_empty() {
        bail!("empty duration");
    }
    let (num_part, unit) = t.split_at(
        t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()),
    );
    let n: u64 = num_part
        .parse()
        .with_context(|| format!("invalid duration: {s}"))?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => bail!("unknown duration unit in {s:?} (use s/m/h/d)"),
    };
    Ok(std::time::Duration::from_secs(secs))
}

fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_uppercase();
    if t.is_empty() {
        bail!("empty size");
    }
    let (num_part, unit) = t.split_at(
        t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()),
    );
    let n: u64 = num_part
        .parse()
        .with_context(|| format!("invalid size: {s}"))?;
    let mult: u64 = match unit {
        "" | "B" => 1,
        "KB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "KIB" => 1024,
        "MIB" => 1024 * 1024,
        "GIB" => 1024 * 1024 * 1024,
        _ => bail!("unknown size unit in {s:?} (use B/KB/MB/GB/KiB/MiB/GiB)"),
    };
    Ok(n.saturating_mul(mult))
}

// Submodule declarations. Verb implementations live one-per-file
// under `commands/session/<verb>.rs`; helpers above are visible to
// each submodule via `use super::*;` (child modules reach the
// parent's private items in Rust).
mod export;
mod inspect;
mod orphan;
mod prune;
mod search;
mod slim;
mod trash;

// Re-exports — main.rs's match block dispatches on these names.
pub use export::export_cmd;
pub use inspect::view_cmd;
pub use orphan::{adopt_orphan_cmd, list_orphans, move_cmd, rebuild_index_cmd};
pub use prune::prune_cmd;
pub use search::{search_cmd, worktrees_cmd};
pub use slim::slim_cmd;
pub use trash::{trash_empty_cmd, trash_list_cmd, trash_restore_cmd};
