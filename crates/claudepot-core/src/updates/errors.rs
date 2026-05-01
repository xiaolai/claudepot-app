//! Error type for the updates module.

use thiserror::Error;

/// All failure modes for update operations. Wraps subprocess failures
/// (the `claude update` and `brew upgrade` shell-outs), HTTP failures
/// from the version-check endpoints, and the few hand-rolled failure
/// modes we need to distinguish (signature mismatch, refused-because-
/// running, unsupported-platform).
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Free-form parse failure — e.g., a version-check endpoint returned
    /// an HTML error page where we expected a `MAJOR.MINOR.PATCH` string.
    /// Use sparingly; prefer typed variants where possible.
    #[error("parse: {0}")]
    Parse(String),

    /// Operation is meaningful in principle but refused under current
    /// state — e.g., Desktop is running, or `DISABLE_UPDATES=1` is set
    /// and we were asked to run `claude update`.
    #[error("refused: {0}")]
    Refused(String),

    /// Subprocess (`claude update`, `brew upgrade --cask`, `ditto`,
    /// `codesign`, `unzip`) terminated with a non-zero status.
    /// `stderr` is the captured child stderr (may be empty).
    #[error("subprocess `{cmd}` exited {status}: {stderr}")]
    Subprocess {
        cmd: String,
        status: i32,
        stderr: String,
    },

    /// Code signature on a downloaded artifact didn't match the
    /// expected authority (`Anthropic PBC`).
    #[error("signature: {0}")]
    Signature(String),

    /// Caller asked for an action that's not implemented on the
    /// current OS (e.g., direct-DMG install on Linux). Surfaces in
    /// the UI as "managed elsewhere" / "not supported here".
    #[error("unsupported on this platform")]
    UnsupportedPlatform,

    /// Required tool not found in PATH (`brew`, `pgrep`, `codesign`,
    /// `ditto`). Distinct from `Io::NotFound` on the binary path
    /// itself so the UI can surface "install Homebrew" rather than
    /// "binary missing".
    #[error("tool not found: {0}")]
    ToolMissing(String),
}

pub type Result<T> = std::result::Result<T, UpdateError>;
