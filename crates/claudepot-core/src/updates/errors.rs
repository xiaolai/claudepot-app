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

    /// An install step failed AND moving the backup copy back into
    /// place also failed — the app at the install path may be missing
    /// or partial. `cause` is the original install failure; the
    /// message names the backup so the user can restore it by hand.
    #[error(
        "{cause}; restoring the backup also failed ({restore_err}) — \
         the previous app is preserved at {} and must be moved back \
         manually",
        .backup.display()
    )]
    RestoreFailed {
        backup: std::path::PathBuf,
        restore_err: std::io::Error,
        #[source]
        cause: Box<UpdateError>,
    },
}

/// `settings_bridge` writes `~/.claude/settings.json` through the shared
/// mutation boundary, which has its own error type. Map it onto the variants
/// this module already surfaces rather than adding a parallel one — every
/// failure here is still an I/O failure, a parse failure, or a refusal to
/// clobber a file we could not read.
impl From<crate::settings_mutex::SettingsMutexError> for UpdateError {
    fn from(e: crate::settings_mutex::SettingsMutexError) -> Self {
        use crate::settings_mutex::SettingsMutexError as S;
        match e {
            S::Io(e) => Self::Io(e),
            S::JsonParse(e) => Self::Json(e),
            S::NotAJsonObject(p) => Self::Parse(format!("{} root is not an object", p.display())),
            S::Contended { path, attempts } => Self::Refused(format!(
                "{} is being written by something else (gave up after {attempts} attempts)",
                path.display()
            )),
        }
    }
}

pub type Result<T> = std::result::Result<T, UpdateError>;
