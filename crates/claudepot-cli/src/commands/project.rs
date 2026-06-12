//! `claudepot project …` verb-group module.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **list** / **show** — read-only browsing (grouped in `list.rs`)
//! - **move** (`move_project`) — rename a project and migrate CC
//!   state (in `rename.rs`; `move` is a Rust keyword)
//! - **clean** — bulk-remove orphaned project directories
//! - **remove** — single-target trash with manifest snapshot
//! - **trash** — list / restore / empty the recoverable trash
//! - **repair** — resolve pending / failed rename journals
//!
//! Per the commands.md rule for nouns with ≥3 verbs, verbs live in
//! submodules under `commands/project/<group>.rs`. This entry point
//! holds the imports, the journal-gate helpers and shared formatters
//! consumed by multiple verbs, the submodule declarations, and the
//! `pub use` re-exports `main.rs` depends on. All handlers are thin
//! wrappers around `claudepot_core` — no business logic here, per
//! `.claude/rules/architecture.md`.
//!
//! The migrate verbs (export / import / inspect / undo) live in the
//! sibling `commands/project_migrate.rs` module — see its header.

use crate::output::format_size;
use crate::AppContext;
use anyhow::{Context as _, Result};
use claudepot_core::paths;
use claudepot_core::project;
use claudepot_core::project_journal;
use std::time::SystemTime;

fn journals_dir() -> std::path::PathBuf {
    paths::claudepot_repair_dir().join("journals")
}

fn locks_dir() -> std::path::PathBuf {
    paths::claudepot_repair_dir().join("locks")
}

fn snapshots_dir() -> std::path::PathBuf {
    paths::claudepot_repair_dir().join("snapshots")
}

/// Print a one-line banner if any pending journals exist. Used by
/// read-only subcommands (list, show) per spec §6 (gate rules).
fn warn_pending_journals_banner() {
    let Ok(active) = project_journal::list_active_pending(&journals_dir()) else {
        return;
    };
    if !active.is_empty() {
        eprintln!(
            "\u{26a0}  {} pending rename journal(s). Run `claudepot project repair` to resolve.",
            active.len()
        );
    }
}

/// Hard gate for mutating subcommands. Returns an error if any journal
/// is pending (and not abandoned) unless the caller explicitly opts
/// out via `ignore_pending_journals`.
fn gate_on_pending_journals(ignore: bool) -> Result<()> {
    let pending = project_journal::list_active_pending(&journals_dir())?;
    if pending.is_empty() || ignore {
        if ignore && !pending.is_empty() {
            eprintln!(
                "\u{26a0}  Ignoring {} pending rename journal(s) at your request.",
                pending.len()
            );
        }
        return Ok(());
    }
    eprintln!("Pending rename journals detected:");
    for (path, j) in &pending {
        eprintln!(
            "  {} — {} \u{2192} {} (started {}, phases: [{}])",
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            j.old_path,
            j.new_path,
            j.started_at,
            j.phases_completed.join(", ")
        );
    }
    anyhow::bail!(
        "refusing to proceed with pending rename journals. \
         Run `claudepot project repair` or pass --ignore-pending-journals."
    );
}

fn format_relative_time(time: SystemTime) -> String {
    let elapsed = time.elapsed().unwrap_or_default();
    let secs = elapsed.as_secs();

    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

// Submodule declarations. Verb groups live under
// `commands/project/<group>.rs`; helpers above are visible to each
// submodule via `use super::*;` (child modules reach the parent's
// private items in Rust).
mod clean;
mod list;
mod remove;
mod rename;
mod repair;
mod trash;

// Re-exports — main.rs's match block and clap variants depend on
// these names.
pub use clean::clean;
pub use list::{list, show};
pub use remove::remove;
pub use rename::{move_project, MoveArgs};
pub use repair::{repair, RepairArgs};
pub use trash::{trash_empty, trash_list, trash_restore};
