//! Apply pipeline — structured ops, deny-by-default validation,
//! shell-free execution.
//!
//! See `dev-docs/templates-implementation-plan.md` §3.2 and §9.
//!
//! Threat model:
//!
//! - The LLM emits `pending-changes.json` next to its run output.
//! - The user reviews each item and selects which to apply.
//! - The executor accepts only typed `Operation`s — no raw shell.
//!   Even an LLM that hallucinates `rm -rf /` cannot reach the
//!   filesystem, because the schema has no field to deserialize
//!   that into.
//! - Every operation is validated against the blueprint's
//!   `apply.scope` (allowed_paths globs, allowed_operations
//!   whitelist) before execution. Canonicalize-then-check
//!   defeats `..` traversal; symlink target is also resolved.
//!
//! The executor is `async` and uses `tokio::fs` directly — no
//! shell process is spawned, ever.

pub mod executor;
pub mod ops;
pub mod sidecar;
pub mod validator;

pub use executor::{apply_selected, ApplyOutcome, ApplyReceipt, ItemOutcome};
pub use ops::{Operation, PendingChanges, PendingGroup, PendingItem};
pub use validator::validate_item;
