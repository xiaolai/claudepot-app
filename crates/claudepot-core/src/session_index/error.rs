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
}
