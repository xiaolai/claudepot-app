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

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl crate::error_code::ErrorCode for SessionIndexError {
    fn code(&self) -> &'static str {
        match self {
            SessionIndexError::Sql(_) => "session_index.sql",
            SessionIndexError::Io(_) => "session_index.io",
            SessionIndexError::Session(_) => "session_index.session",
            SessionIndexError::Json(_) => "session_index.json",
            SessionIndexError::MigrationValidationFailed { .. } => {
                "session_index.migration_validation_failed"
            }
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            SessionIndexError::Sql(e) => serde_json::json!({ "detail": e.to_string() }),
            SessionIndexError::Io(e) => serde_json::json!({ "detail": e.to_string() }),
            // Its own code rather than the inner `SessionError`'s: the
            // variant prefixes the English with "session scan: ", so a
            // localized sentence has framing of its own to carry.
            SessionIndexError::Session(e) => serde_json::json!({ "detail": e.to_string() }),
            SessionIndexError::Json(e) => serde_json::json!({ "detail": e.to_string() }),
            SessionIndexError::MigrationValidationFailed {
                target_version,
                expected,
                found,
                missing,
            } => serde_json::json!({
                "target_version": target_version,
                "expected": expected,
                "found": found,
                // Joined exactly as the English message joins it —
                // a catalog entry interpolates one string, and
                // i18next cannot join an array.
                "missing": missing.join(", "),
            }),
        }
    }
}
