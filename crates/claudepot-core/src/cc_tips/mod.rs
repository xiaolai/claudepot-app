//! CC tips ledger — read-only introspection over CC's tip registry.
//!
//! Three sources joined at render time:
//! - **CC binary** (`extract`) — Bun-compiled CC embeds the tip
//!   registry as ASCII in its read-only data section. We byte-scan
//!   for `id:"..."` anchors and parse each tip object via a
//!   token-aware brace walker (`walker`).
//! - **`~/.claude.json`** (`history`) — the `tipsHistory` map
//!   `{id → numStartups}` plus `numStartups` itself. Tells us which
//!   tips this user has been shown and at what session count.
//! - **Claudepot snapshots** (`history::snapshot`) — append-only log
//!   of `{ts, numStartups, tipsHistory}` records that converts CC's
//!   count-based ledger into a time-resolved one.
//!
//! Bundled metadata (`categories`, `triggers`) is Claudepot-authored:
//! human-readable category and trigger summaries for the known tip
//! ids. Anthropic's prose stays in the binary; we never bundle it.
//!
//! See `dev-docs/cc-tips-ledger.md` for the design.

pub mod catalog;
pub mod categories;
pub mod error;
pub mod extract;
pub mod history;
pub mod triggers;
pub mod walker;

pub use catalog::{CatalogSnapshot, RenderedTip, TipsRender};
pub use categories::{category_for, Category};
pub use error::TipsError;
pub use extract::{extract_from_binary, resolve_cc_binary, RawTip};
pub use history::{read_tips_history, LastSeen, Snapshot, SnapshotLog};
pub use triggers::{trigger_for, TriggerInfo};
