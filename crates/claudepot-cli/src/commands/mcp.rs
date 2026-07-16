//! Claudepot MCP memory server (WI-008).
//!
//! `claudepot mcp memory-server` starts a stdio MCP server exposing
//! thirteen tools backed by the shared_memory module:
//!
//! * `claudepot_search_memory`
//! * `claudepot_read_conversation`
//! * `claudepot_remember`
//! * `claudepot_archive_memory`
//! * `claudepot_log_decision`
//! * `claudepot_archive_decision`
//! * `claudepot_submit_evidence`
//! * `claudepot_list_memories`
//! * `claudepot_list_decisions`
//! * `claudepot_list_evidence`
//! * `claudepot_memory_links`
//! * `claudepot_list_sessions`
//! * `claudepot_list_projects`
//!
//! All emission paths run through `claudepot_core::redaction::apply`
//! before crossing the MCP boundary. Server logs go to stderr only;
//! stdout is reserved for JSON-RPC frames.
//!
//! Spike verdict in `dev-docs/reports/rmcp-spike-2026-05-15.md`.
//!
//! Per the commands.md rule for nouns with ≥3 verbs, the verbs live
//! in submodules: `server.rs` (the memory-server itself — tool
//! structs, router, emission helpers) and `snippet.rs` (the
//! print-snippet / install-snippet pair). This entry point holds the
//! shared imports, the `default_db_path` helper the codex noun also
//! consumes, the submodule declarations, and the `pub use`
//! re-exports `main.rs` depends on.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Default DB path: `<claudepot_data_dir>/sessions.db`. Honors the
/// `CLAUDEPOT_DATA_DIR` override so the MCP server reads the same
/// index as the rest of the CLI and the GUI (split-brain guard).
/// Shared with the codex verbs.
pub(crate) fn default_db_path() -> PathBuf {
    claudepot_core::paths::claudepot_data_dir().join("sessions.db")
}

// The canonical snippet body lives in `claudepot_core::mcp_snippet`
// so the CLI installer and the Tauri Settings → MCP pane both emit
// the same bytes. Re-export the public surface here to keep the
// CLI's call sites short.

pub use claudepot_core::mcp_snippet::snippet_body;

// Submodule declarations. The imports above are visible to each
// submodule via `use super::*;` (child modules reach the parent's
// private items in Rust).
mod server;
mod snippet;

// Re-exports — main.rs's match block dispatches on these names.
pub use server::run;
pub use snippet::{install_snippet, print_snippet};
