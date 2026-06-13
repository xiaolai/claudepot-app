//! `claudepot cli …` verb-group module.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **status** — show the active CLI account
//! - **use** (`use_account`) — switch the active CLI account
//! - **clear** — clear CC credentials (log out)
//! - **run** — launch a command with a specific account's token
//!
//! Per the commands.md rule for nouns with ≥3 verbs, each verb lives
//! one-per-file under `commands/cli_ops/<verb>.rs`. This entry point
//! holds the shared imports, the submodule declarations, and the
//! `pub use` re-exports `main.rs` depends on. All handlers are thin
//! wrappers around `claudepot_core` — no business logic here, per
//! `.claude/rules/architecture.md`.

use crate::AppContext;
use anyhow::Result;

// Submodule declarations. Verb implementations live one-per-file
// under `commands/cli_ops/<verb>.rs`; the imports above are visible
// to each submodule via `use super::*;` (child modules reach the
// parent's private items in Rust).
mod clear;
mod run;
mod status;
mod use_account;

// Re-exports — main.rs's match block dispatches on these names.
pub use clear::clear;
pub use run::run;
pub use status::status;
pub use use_account::use_account;
