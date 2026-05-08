//! Account auto-rotation — rules engine.
//!
//! See `dev-docs/auto-rotation.md` for the full design. This module is
//! pure-Rust, no Tauri dependency. It provides:
//!
//! - [`rules`] — typed rule schema + serde + validation.
//! - [`store`] — atomic load/save of `~/.claudepot/rotation-rules.json`.
//! - [`audit`] — ring-buffer log of every swap attempt at
//!   `~/.claudepot/rotation-audit.json`.
//! - [`eval`] — pure evaluator that maps `(rules, snapshot, active,
//!   audit, now)` to a list of pending swaps.
//!
//! The orchestrator (Tauri's `usage_watcher`) loads rules + audit,
//! calls [`eval::evaluate`], then dispatches swaps based on each
//! pending entry's mode. Nothing in this module performs network or
//! account-mutation I/O; that lives behind `cli_backend::swap`.

pub mod audit;
pub mod eval;
pub mod rules;
pub mod store;

pub use audit::{
    RotationAuditEntry, RotationAuditLog, RotationOutcome, RotationTriggerSummary,
    MAX_AUDIT_ENTRIES,
};
pub use eval::{evaluate, NoCandidateReason, PendingSwap, SkipReason};
pub use rules::{
    Action, RotationGuards, RotationMode, RotationRule, RotationRulesFile, Selector, Trigger,
    SCHEMA_VERSION,
};
pub use store::{load, save, RotationStoreError};
