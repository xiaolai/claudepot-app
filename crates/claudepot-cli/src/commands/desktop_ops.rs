//! `claudepot desktop …` verb-group module.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **status** — show the active Desktop account and running state
//! - **use** (`use_account`) — switch the active Desktop account
//! - **identity** — probe the live Desktop session identity
//! - **reconcile** — align profile flags with on-disk truth
//! - **adopt** — capture the live session into an account snapshot
//! - **clear** — sign Desktop out (snapshot kept by default)
//! - **launch** / **quit** — Desktop process control (grouped in
//!   `process.rs`)
//!
//! Per the commands.md rule for nouns with ≥3 verbs, verbs live in
//! submodules under `commands/desktop_ops/<verb>.rs` (one-per-file,
//! except the launch/quit process-control pair). This entry point
//! holds the shared imports, the submodule declarations, and the
//! `pub use` re-exports `main.rs` depends on. All handlers are thin
//! wrappers around `claudepot_core` — no business logic here, per
//! `.claude/rules/architecture.md`.

use crate::AppContext;
use anyhow::Result;

// Submodule declarations. The imports above are visible to each
// submodule via `use super::*;` (child modules reach the parent's
// private items in Rust).
mod adopt;
mod clear;
mod identity;
mod process;
mod reconcile;
mod status;
mod use_account;

// Re-exports — main.rs's match block dispatches on these names.
pub use adopt::adopt;
pub use clear::clear;
pub use identity::identity;
pub use process::{launch, quit};
pub use reconcile::reconcile;
pub use status::status;
pub use use_account::use_account;
