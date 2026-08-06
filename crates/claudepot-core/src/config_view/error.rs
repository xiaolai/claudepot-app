//! Boundary error for the `config_view` module, per rust-conventions
//! ("one enum per module boundary"). Public `config_view` functions
//! that can fail return this instead of stringly `Result<_, String>`,
//! so callers can distinguish I/O from parse from query errors
//! without string-matching.

use crate::error_code::ErrorCode;
use serde_json::{json, Value};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigViewError {
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// Search query rejected — the regex (or `(?i)`-wrapped plain
    /// needle) failed to compile.
    #[error("bad search query: {0}")]
    BadQuery(#[from] regex::Error),

    /// No `plugin.json` / `.claude-plugin/plugin.json` under the
    /// plugin root.
    #[error("no plugin manifest in {0} (looked for plugin.json / .claude-plugin/plugin.json)")]
    ManifestNotFound(PathBuf),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl ErrorCode for ConfigViewError {
    fn code(&self) -> &'static str {
        match self {
            ConfigViewError::Io { .. } => "config_view.io",
            ConfigViewError::Parse { .. } => "config_view.parse",
            ConfigViewError::BadQuery(_) => "config_view.bad_query",
            ConfigViewError::ManifestNotFound(_) => "config_view.manifest_not_found",
        }
    }

    fn params(&self) -> Value {
        // Paths, a regex-compile diagnostic, and serde/io failure text.
        // `config_view` reads settings files but never quotes a value
        // into an error, so nothing here can carry a token.
        match self {
            ConfigViewError::Io { path, source } => {
                json!({ "path": path.display().to_string(), "detail": source.to_string() })
            }
            ConfigViewError::Parse { path, source } => {
                json!({ "path": path.display().to_string(), "detail": source.to_string() })
            }
            ConfigViewError::BadQuery(e) => json!({ "detail": e.to_string() }),
            ConfigViewError::ManifestNotFound(path) => {
                json!({ "path": path.display().to_string() })
            }
        }
    }
}
