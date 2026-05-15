//! Cross-harness shared memory (Claude Code + Codex).
//!
//! Owns the durable tables that live alongside the existing
//! `sessions` cache in `sessions.db`: `exchanges`, `tool_calls`,
//! `exchange_fts`, plus the user/agent-authored `memories`,
//! `decisions`, `evidence_records`, `memory_links`. The
//! transcript-derived rows can be rebuilt from disk; the durable
//! rows are authoritative.
//!
//! ## Submodules
//!
//! * [`schema`] — DDL for the v4 tables and FTS5 maintenance
//!   triggers, plus the `V4_TABLE_NAMES` / `V4_TRIGGER_NAMES`
//!   constants used by `session_index::apply_schema`'s post-write
//!   validator.
//! * [`indexer`] — Codex transcript backfill / incremental
//!   indexer. Walks `$CODEX_HOME/sessions/`, parses via
//!   [`crate::codex_session`], writes to `sessions` (with
//!   `source_kind='codex'`), `exchanges`, `tool_calls`. Each
//!   per-file write is wrapped in a SAVEPOINT for isolation.
//! * [`search`] — Exchange-level FTS5 search. Phrase-escapes user
//!   input, applies redaction at emission, supports source /
//!   project / git_branch / model / date-range filters.
//! * [`read`] — Read-by-locator. Containment-checks the
//!   `file_path` against `sessions`, resolves line bounds (with
//!   exchange-id constrained to that file), enforces a byte cap.
//! * [`durable`] — CRUD over the user/agent-authored tables:
//!   `create_memory`, `archive_memory`, `log_decision`,
//!   `supersede_decision`, `archive_decision`, `submit_evidence`,
//!   `link`. All writers go through the `with_tx` helper.
//!
//! See `dev-docs/codex-plans/20260515-1130-shared-memory.md` for
//! the full plan and `grill-report-2026-05-15.md` for the audit
//! history that produced the current shape.

pub mod durable;
pub mod indexer;
pub mod read;
pub mod schema;
pub mod search;

#[cfg(test)]
pub mod migration_tests;
