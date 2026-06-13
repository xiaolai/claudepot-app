//! `claudepot codex …` verb-group module — populate `sessions.db`
//! with Codex rollouts so the cross-harness MCP memory tools can
//! surface them (H4 of the grill fixing plan).
//!
//! Without `codex index`, `backfill_codex` had no production caller
//! and `claudepot_search_memory` would silently return empty
//! results in production.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **index** — walk the Codex sessions tree into `sessions.db`
//! - **rebuild** — set the `_pending_rescan` marker
//! - **forget** — wipe Shared Memory rows (destructive; `--yes`)
//!
//! Per the commands.md rule for nouns with ≥3 verbs, each verb lives
//! one-per-file under `commands/codex/<verb>.rs`. This entry point
//! holds the shared imports, the submodule declarations, and the
//! `pub use` re-exports `main.rs` depends on.

use std::path::PathBuf;

use anyhow::{Context, Result};

// Shared with the MCP verb so both honor `CLAUDEPOT_DATA_DIR`
// (split-brain guard).
use super::mcp::default_db_path;

// Submodule declarations. Verb implementations live one-per-file
// under `commands/codex/<verb>.rs`; the imports above are visible to
// each submodule via `use super::*;` (child modules reach the
// parent's private items in Rust).
mod forget;
mod index;
mod rebuild;

// Re-exports — main.rs's match block dispatches on these names.
pub use forget::forget;
pub use index::index;
pub use rebuild::rebuild;
