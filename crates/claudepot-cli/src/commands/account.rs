//! `claudepot account …` verb-group module.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **list** — table of registered accounts with live usage
//! - **add** — register from current CC creds / refresh token / browser
//! - **remove** — delete a registered account (with confirmation)
//! - **inspect** — detailed account view incl. token health + usage
//! - **verify** — per-account blob identity check against /profile
//!
//! Per the commands.md rule for nouns with ≥3 verbs, each verb lives
//! one-per-file under `commands/account/<verb>.rs`. This entry point
//! holds the shared imports, the helpers consumed by multiple verbs,
//! the submodule declarations, and the `pub use` re-exports `main.rs`
//! depends on. All handlers are thin wrappers around `claudepot_core`
//! — no business logic here, per `.claude/rules/architecture.md`.

use crate::output;
use crate::AppContext;
use anyhow::Result;

/// Capitalize the first character (plan names: "max" → "Max").
/// Shared by `add` (register line) and `inspect` (plan row).
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

// Submodule declarations. Verb implementations live one-per-file
// under `commands/account/<verb>.rs`; helpers above are visible to
// each submodule via `use super::*;` (child modules reach the
// parent's private items in Rust).
mod add;
mod inspect;
mod list;
mod remove;
mod verify;

// Re-exports — main.rs's match block dispatches on these names.
pub use add::add;
pub use inspect::inspect;
pub use list::list;
pub use remove::remove;
pub use verify::verify;
