//! Boundary error for the `desktop_backend` module (the Desktop
//! profile slot). Historically lived in the crate-root `error.rs`;
//! relocated next to its boundary per rust-conventions ("one enum per
//! module boundary"). `crate::error::DesktopSwapError` remains a
//! re-export.

use crate::error_code::ErrorCode;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DesktopSwapError {
    #[error("Claude Desktop is still running after quit timeout")]
    DesktopStillRunning,

    #[error("no desktop profile stored for account {0}")]
    NoStoredProfile(uuid::Uuid),

    #[error("file copy failed: {0}")]
    FileCopyFailed(String),

    #[error("desktop not installed on this platform")]
    NotInstalled,

    /// Windows-only. Detected at pre-restore by
    /// `desktop_service::check_profile_dpapi_valid`. Means the
    /// stored profile's ciphertext was encrypted under a different
    /// DPAPI master key than the one this Windows session currently
    /// holds, so Chromium on next launch would reject the cookies /
    /// tokens as corrupt. Surfaced to the user as "re-sign in to
    /// Claude Desktop on this machine; Claudepot will re-bind the
    /// fresh session." Never fires on macOS.
    #[error(
        "Desktop profile encrypted under different Windows credentials \
         (different machine, different user, or password reset) — \
         sign in to Claude Desktop fresh, then re-bind."
    )]
    DpapiInvalidated,

    /// Failure to acquire or open the Desktop operation lock. Carries
    /// the underlying [`crate::desktop_lock::DesktopLockError`] so
    /// callers can distinguish "already held" (retry) from
    /// "open failed" (I/O) without string-matching on the message.
    #[error("desktop lock: {0}")]
    Lock(#[from] crate::desktop_lock::DesktopLockError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// `desktop_backend` owns four error enums rather than the one
/// rust-conventions asks for, so only this one — the module's boundary
/// error, the `desktop` noun's swap failures — takes the bare
/// `desktop_backend` segment. The other three are namespaced by the
/// concern they carry: `desktop_key`, `desktop_crypto`,
/// `desktop_token_cache`.
impl ErrorCode for DesktopSwapError {
    fn code(&self) -> &'static str {
        match self {
            DesktopSwapError::DesktopStillRunning => "desktop_backend.desktop_still_running",
            DesktopSwapError::NoStoredProfile(_) => "desktop_backend.no_stored_profile",
            DesktopSwapError::FileCopyFailed(_) => "desktop_backend.file_copy_failed",
            DesktopSwapError::NotInstalled => "desktop_backend.not_installed",
            DesktopSwapError::DpapiInvalidated => "desktop_backend.dpapi_invalidated",
            DesktopSwapError::Lock(_) => "desktop_backend.lock",
            DesktopSwapError::Io(_) => "desktop_backend.io",
        }
    }

    fn params(&self) -> Value {
        match self {
            DesktopSwapError::DesktopStillRunning => json!({}),
            // The account row's uuid, never a profile secret.
            DesktopSwapError::NoStoredProfile(uuid) => json!({ "uuid": uuid.to_string() }),
            DesktopSwapError::FileCopyFailed(detail) => json!({ "detail": detail }),
            DesktopSwapError::NotInstalled => json!({}),
            DesktopSwapError::DpapiInvalidated => json!({}),
            // The inner lock error's own English. A GUI that wants to
            // branch on "already held" vs "open failed" should match on
            // `DesktopLockError` before it reaches this boundary.
            DesktopSwapError::Lock(e) => json!({ "detail": e.to_string() }),
            DesktopSwapError::Io(e) => json!({ "detail": e.to_string() }),
        }
    }
}
