//! Session — read-only introspection over CC's `.jsonl` transcripts,
//! plus the surgical move/prune/slim/export operations layered on top.
//!
//! This directory module folds together what used to be ~19 flat
//! `session_*` modules at the crate root. The shared transcript model
//! (`SessionRow`, `SessionEvent`, `SessionDetail`, `SessionError`,
//! `TokenUsage`, the parsers) lives in [`core`] and is re-exported here
//! so `crate::session::Foo` resolves exactly as it did when the family
//! was a single `session.rs`. Each surface gets its own submodule:
//!
//! - [`chunks`] — turn-windowed transcript chunking.
//! - [`classify`] — per-session activity classification.
//! - [`context`] — context-window reconstruction.
//! - [`export`] / [`export_delivery`] — transcript export + delivery.
//! - [`move_`] — relocate a transcript between project cwds (the
//!   JSONL rewriters, slug helpers, and types are private inside it).
//! - [`phases`] — phase segmentation of a session.
//! - [`prune`] — bulk transcript pruning.
//! - [`search`] — cross-session full-text search (ranking is private
//!   inside it).
//! - [`share`] — shareable-bundle assembly.
//! - [`slim`] — transcript slimming (image strip, etc.).
//! - [`subagents`] — subagent transcript linkage.
//! - [`tool_link`] — tool-use → result linkage.
//! - [`worktree`] — git-worktree resolution for a session.
//!
//! Note: `session_index/` and `session_live/` are *separate*
//! crate-root directory modules and intentionally NOT folded in here —
//! the index is the persistent `sessions.db` cache and `live` is the
//! running-session metrics path; both predate this consolidation and
//! keep their own boundaries.
//!
//! The old crate-root paths (`crate::session_export`,
//! `crate::session_move`, …) are kept alive by `pub use` shims in
//! `lib.rs` so CLI and Tauri call sites compile unchanged.

pub mod chunks;
pub mod classify;
pub mod context;
pub mod core;
pub mod export;
pub mod export_delivery;
pub mod move_;
pub mod phases;
pub mod prune;
pub mod redact;
pub mod search;
pub mod share;
pub mod slim;
pub mod subagents;
pub mod tool_link;
pub mod worktree;

// The core module *is* the noun's shared model; re-export it flat so
// `crate::session::SessionRow`, `crate::session::parse_events_public`,
// … resolve exactly as they did when `session.rs` was a single file.
pub use core::*;
