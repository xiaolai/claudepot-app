//! `claudepot session …` verb-group module.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **orphan** — list-orphans, move, adopt-orphan, rebuild-index
//! - **inspect** — view (chunks / tools / classify / subagents /
//!   phases / context)
//! - **search** — search, worktrees
//! - **export** — write a transcript to file (markdown / html / json)
//! - **prune** — wholesale-delete sessions matching a filter
//! - **trash** — list / restore / empty the prune trash
//! - **slim** — strip noisy events from a session in place
//!
//! Per the commands.md rule for nouns with ≥3 verbs, related verbs
//! live in submodules under `commands/session/<group>.rs`. This entry
//! point holds the imports, the shared formatting helpers consumed
//! by multiple verbs, the submodule declarations, and the `pub use`
//! re-exports `main.rs` depends on. All handlers are thin wrappers
//! around `claudepot_core` — no business logic here, per
//! `.claude/rules/architecture.md`.

use crate::output::{format_size, format_ts_ms, print_json, truncate_start};
use crate::AppContext;
use anyhow::{bail, Context, Result};
use claudepot_core::paths;
use claudepot_core::paths::claude_json_path;
use claudepot_core::session::{read_session_detail, read_session_detail_at_path, SessionDetail};
use claudepot_core::session_move::{
    adopt_orphan_project, detect_orphaned_projects, move_session, AdoptReport, MoveSessionOpts,
    MoveSessionReport, OrphanedProject,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn parse_duration(s: &str) -> Result<std::time::Duration> {
    let t = s.trim();
    if t.is_empty() {
        bail!("empty duration");
    }
    let (num_part, unit) = t.split_at(t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()));
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
    let (num_part, unit) = t.split_at(t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()));
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
pub use export::{export_cmd, ExportArgs};
pub use inspect::view_cmd;
pub use orphan::{
    adopt_orphan_cmd, backfill_exchanges_cmd, list_orphans, move_cmd, rebuild_index_cmd,
};
pub use prune::{prune_cmd, PruneArgs};
pub use search::{search_cmd, worktrees_cmd};
pub use slim::{slim_cmd, SlimArgs};
pub use trash::{trash_empty_cmd, trash_list_cmd, trash_restore_cmd};
