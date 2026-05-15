//! Cross-harness shared memory (Claude Code + Codex).
//!
//! Owns the durable tables that live alongside the existing
//! `sessions` cache in `sessions.db`: `exchanges`, `tool_calls`,
//! `exchange_fts`, plus the user/agent-authored `memories`,
//! `decisions`, `evidence_records`, `memory_links`. The
//! transcript-derived rows can be rebuilt from disk; the durable
//! rows are authoritative.
//!
//! This module is intentionally minimal in v1. WI-002 lands the
//! schema and the migration; later WIs (WI-003..WI-009) flesh out
//! the indexer, search, locator read, durable CRUD, MCP server,
//! and installer.
//!
//! See `dev-docs/codex-plans/20260515-1130-shared-memory.md` for
//! the full plan.

pub mod schema;

#[cfg(test)]
pub mod migration_tests;
