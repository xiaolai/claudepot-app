//! Boundary error for the `config_view` module, per rust-conventions
//! ("one enum per module boundary"). Public `config_view` functions
//! that can fail return this instead of stringly `Result<_, String>`,
//! so callers can distinguish I/O from parse from query errors
//! without string-matching.

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
