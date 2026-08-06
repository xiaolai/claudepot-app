//! Error type for the `codex_session` module.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodexError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The file was readable but contained no `session_meta`
    /// record. This is fatal for `parse_head` and
    /// `parse_codex_rollout_jsonl`; `iter_events` does not raise
    /// it (callers that don't need head metadata can stream
    /// regardless).
    #[error("no session_meta record found in {path}")]
    MissingSessionMeta { path: PathBuf },

    /// The `session_meta` record was present but lacked a
    /// resolvable session id (`payload.id`). Surfaces drift in the
    /// Codex rollout schema, not a per-line malformation.
    #[error("session_meta in {path} is missing payload.id")]
    MissingSessionId { path: PathBuf },
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// Every variant names the rollout file it failed on, so `path` is
/// carried raw (`Path::display()`, no normalization) on all three.
impl crate::error_code::ErrorCode for CodexError {
    fn code(&self) -> &'static str {
        match self {
            CodexError::Io { .. } => "codex_session.io",
            CodexError::MissingSessionMeta { .. } => "codex_session.missing_session_meta",
            CodexError::MissingSessionId { .. } => "codex_session.missing_session_id",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            CodexError::Io { path, source } => serde_json::json!({
                "path": path.display().to_string(),
                "detail": source.to_string(),
            }),
            CodexError::MissingSessionMeta { path } => serde_json::json!({
                "path": path.display().to_string(),
            }),
            CodexError::MissingSessionId { path } => serde_json::json!({
                "path": path.display().to_string(),
            }),
        }
    }
}
