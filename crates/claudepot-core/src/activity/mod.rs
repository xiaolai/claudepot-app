//! Activity cards — extract anomalies and milestones from session
//! JSONLs. See `dev-docs/activity-cards-design.md` for the full
//! design rationale; v1 implements Phase 1 of that plan.
//!
//! Layout:
//! - `card.rs`       — `Card`, `CardKind`, `Severity`, `HelpRef`,
//!                     `SourceRef`, `ConfigScope`. Public types that
//!                     cross the crate boundary.
//! - `templates.rs`  — help template catalog. v1 ships
//!                     `hook.plugin_missing` only.
//! - `classifier.rs` — pure JSONL line → `Card` function. Stateful
//!                     only via `ClassifierState`, which the caller
//!                     owns per-session.
//! - `index.rs`      — SQLite read/write surface for `activity_cards`.
//! - `backfill.rs`   — walk all `~/.claude/projects/*/*.jsonl` and
//!                     populate the index. Phase 1 ingest path.
//!
//! The LiveRuntime integration (mid-tail classification + bus
//! emission) is Phase 2 — kept out of v1 to keep this slice small
//! and the runtime's active-session machinery untouched.

pub mod backfill;
pub mod card;
pub mod classifier;
pub mod index;
pub mod templates;

pub use card::{Card, CardKind, ConfigScope, HelpRef, Severity, SourceRef};
pub use classifier::{classify, ClassifierState, SessionMeta};
pub use index::{ActivityIndex, ActivityIndexError, RecentQuery};
pub use templates::render as render_help;
