//! Activity cards — extract anomalies and milestones from session
//! JSONLs. See `dev-docs/activity-cards-design.md` for the full
//! design rationale.
//!
//! Phase 1–4 surface (current):
//! - `card.rs`       — `Card`, `CardKind`, `Severity`, `HelpRef`,
//!                     `SourceRef`, `ConfigScope`, `plugin`
//!                     attribution field. Public types that cross
//!                     the crate boundary.
//! - `templates.rs`  — 11-template help catalog: hook.plugin_missing,
//!                     hook.json_invalid, tool.{read_required,
//!                     parallel_cancelled, ssh_timeout, no_such_file,
//!                     edit_drift, user_rejected, bash_cmd_not_found},
//!                     agent.{no_return, error_return}.
//! - `classifier.rs` — pure JSONL line → `Card` function with
//!                     per-session `ClassifierState` (open Agent
//!                     episodes, last-seen model). Routes attachment
//!                     records, user `tool_result` blocks, and
//!                     assistant `tool_use`/`model` fields.
//!                     `finalize_session` drains stranded episodes.
//! - `index.rs`      — SQLite read/write for `activity_cards`,
//!                     `activity_meta` (last_seen cursor). Idempotent
//!                     on `(session_path, event_uuid)`. Plugin column
//!                     for attribution filtering.
//! - `backfill.rs`   — walk all `~/.claude/projects/*/*.jsonl` and
//!                     populate the index. Per-session
//!                     delete-then-replay rebuild semantics.
//!
//! LiveRuntime integration: when an `ActivityIndex` is enabled via
//! `LiveRuntime::enable_activity`, the per-tick tail loop also runs
//! each new line through the classifier and persists emitted cards.
//! On session end (PID gone), `activity_finalize` drains open Agent
//! episodes into AgentStranded cards.
//!
//! Deferred to later phases (see design doc):
//! - SmallVec<[Card; 1]> optimization (current `Vec<Card>` is fine
//!   at v1 scale; revisit if benches justify the dep).
//! - Source-ref line numbers (settings parser byte-offset tracking).
//! - GUI surface (Activity two-pane, click-through, Trends).
//! - Notifier coalescing layer (consumes the same card stream).
//! - Attach-time backfill seed for the classifier (workaround:
//!   `claudepot activity reindex` covers historical sessions).
//! - 5-minute idle finalization (workaround: PID-removal path
//!   covers the common case; reindex catches everything else).

pub mod backfill;
pub mod card;
pub mod classifier;
pub mod index;
pub mod templates;

pub use card::{Card, CardKind, ConfigScope, HelpRef, Severity, SourceRef};
pub use classifier::{classify, finalize_session, ClassifierState, SessionMeta};
pub use index::{ActivityIndex, ActivityIndexError, RecentQuery};
pub use templates::render as render_help;
