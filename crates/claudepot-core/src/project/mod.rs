//! Project — a CC project session directory (one of the five domain
//! nouns; see `.claude/rules/architecture.md`).
//!
//! This directory module folds together what used to be ~15 flat
//! `project_*` modules at the crate root. The noun's core API
//! (`ProjectError`, `list_projects`, `move_project`, …) lives in
//! [`core`] and is re-exported here so `crate::project::Foo` resolves
//! exactly as it did when the family was a single `project.rs`. The
//! surface-specific helpers each get their own submodule:
//!
//! - [`config_rewrite`] / [`rewrite`] — format-preserving edits to a
//!   project's CC config and `.claude.json` surfaces.
//! - [`display`] — human-facing formatting (`format_size`).
//! - [`dry_run_service`] — plan-only move preview.
//! - [`helpers`] — shared path/IO helpers for the noun.
//! - [`journal`] — append-only rename/move journal.
//! - [`lock`] — per-project advisory lock.
//! - [`memory`] — CLAUDE.md / memory-file discovery.
//! - [`progress`] — progress-sink plumbing shared across long ops.
//! - [`remove`] — project removal (slug dir + `.claude.json` + history).
//! - [`repair`] — resume/rollback of an interrupted move.
//! - [`sanitize`] — `sanitize_path` / `unsanitize_path` (CC parity).
//! - [`trash`] — soft-delete to the Claudepot trash.
//! - [`types`] — shared DTOs (`ProjectInfo`, `MoveArgs`, …).
//!
//! Nothing here performs Tauri I/O; this is pure `claudepot-core`.
//! The old crate-root paths (`crate::project_sanitize`, …) are kept
//! alive by `pub use` shims in `lib.rs` so CLI and Tauri call sites
//! compile unchanged.

pub(crate) mod config_rewrite;
pub mod core;
pub(crate) mod display;
pub mod dry_run_service;
pub mod helpers;
pub mod journal;
pub(crate) mod lock;
pub(crate) mod memory;
pub mod progress;
pub mod remove;
pub mod repair;
pub(crate) mod rewrite;
pub mod sanitize;
pub mod trash;
pub mod types;

// The core module *is* the noun's public API; re-export it flat so
// `crate::project::ProjectError`, `crate::project::move_project`, …
// resolve exactly as they did when `project.rs` was a single file.
pub use core::*;

// The plugin-registry health check is a read-only surface CLI/GUI call
// (the rest of `config_rewrite` stays crate-private, invoked only by the
// move phases).
pub use config_rewrite::{detect_stale_plugin_bindings, StalePluginBinding};
