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
