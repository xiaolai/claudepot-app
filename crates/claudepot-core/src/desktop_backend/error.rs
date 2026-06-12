//! Boundary error for the `desktop_backend` module (the Desktop
//! profile slot). Historically lived in the crate-root `error.rs`;
//! relocated next to its boundary per rust-conventions ("one enum per
//! module boundary"). `crate::error::DesktopSwapError` remains a
//! re-export.

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
