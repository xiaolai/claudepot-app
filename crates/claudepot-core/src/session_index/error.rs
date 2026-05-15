//! Errors surfaced by the persistent session index.
//!
//! The index is a best-effort cache over `~/.claude/projects/` — most
//! failure modes should be recoverable (wipe and rebuild). Keep the
//! variants narrow so callers can decide what to surface vs. what to
//! swallow.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionIndexError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),

    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),

    #[error("session scan: {0}")]
    Session(#[from] crate::session::SessionError),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Post-write migration validation failed: the v4 schema-apply
    /// transaction produced an incomplete set of tables / triggers /
    /// FTS internal tables. The transaction is rolled back before
    /// this variant returns, so the DB stays at the prior version.
    ///
    /// Distinct from `Sql(QueryReturnedNoRows)` so downstream
    /// recovery logic doesn't conflate a real "no rows" condition
    /// with "your migration produced the wrong table set."
    #[error(
        "migration validation failed at v{target_version}: expected {expected} objects, found {found}; missing: [{missing}]",
        missing = .missing.join(", ")
    )]
    MigrationValidationFailed {
        target_version: String,
        expected: usize,
        found: usize,
        missing: Vec<String>,
    },
}
