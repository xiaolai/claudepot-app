#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

pub mod crypto;
pub mod error;
pub mod swap;
pub mod token_cache;

pub use error::DesktopSwapError;

use std::path::PathBuf;

#[async_trait::async_trait]
pub trait DesktopPlatform: Send + Sync {
    fn data_dir(&self) -> Option<PathBuf>;
    fn session_items(&self) -> &[&str];
    async fn is_running(&self) -> bool;
    async fn quit(&self) -> Result<(), DesktopSwapError>;
    async fn launch(&self) -> Result<(), DesktopSwapError>;

    /// Whether the Claude Desktop app is installed on this machine.
    ///
    /// Distinct from "has a data_dir" — a fresh install has no data_dir
    /// until first launch, and a user who manually cleared
    /// `~/Library/Application Support/Claude/` still has the app
    /// installed. `app_status.desktop_installed` currently collapses
    /// both questions into one disk check; this accessor lets callers
    /// disambiguate.
    ///
    /// macOS: `/Applications/Claude.app` bundle exists.
    /// Windows: the MSIX package is registered (best-effort probe;
    /// falls back to data-dir existence when AppX APIs aren't
    /// reachable from the current process).
    fn is_installed(&self) -> bool;

    /// Fetch the OS-scoped encryption secret Electron's safeStorage
    /// was keyed against. Feeds directly into `crypto::decrypt`.
    ///
    /// macOS: value of `Claude Safe Storage / Claude Key` keychain
    /// item (retrieved via `/usr/bin/security find-generic-password`).
    /// Windows: 32-byte master key produced by DPAPI-unprotecting the
    /// `encrypted_key` field of `Local State`.
    ///
    /// Consumers must treat the returned bytes as SENSITIVE — never
    /// log, never forward across IPC, never serialize.
    async fn safe_storage_secret(&self) -> Result<Vec<u8>, DesktopKeyError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DesktopKeyError {
    #[error("macOS keychain lookup failed: {0}")]
    KeychainRead(String),
    #[error("Windows DPAPI unprotect failed: {0}")]
    DpapiFailed(String),
    #[error("Windows Local State missing or unreadable: {0}")]
    LocalState(String),
    #[error("platform does not implement Desktop safeStorage")]
    Unsupported,
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// Segment is `desktop_key`, not `desktop_backend` — see the note on
/// [`error::DesktopSwapError`]'s impl for why this module's four enums
/// are namespaced by concern.
impl crate::error_code::ErrorCode for DesktopKeyError {
    fn code(&self) -> &'static str {
        match self {
            DesktopKeyError::KeychainRead(_) => "desktop_key.keychain_read",
            DesktopKeyError::DpapiFailed(_) => "desktop_key.dpapi_failed",
            DesktopKeyError::LocalState(_) => "desktop_key.local_state",
            DesktopKeyError::Unsupported => "desktop_key.unsupported",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            // These carry `security`/DPAPI failure text and filesystem
            // messages. The safeStorage secret itself is returned on the
            // Ok path and never reaches an error string — keep it that
            // way when adding a variant.
            DesktopKeyError::KeychainRead(detail) => serde_json::json!({ "detail": detail }),
            DesktopKeyError::DpapiFailed(detail) => serde_json::json!({ "detail": detail }),
            DesktopKeyError::LocalState(detail) => serde_json::json!({ "detail": detail }),
            DesktopKeyError::Unsupported => serde_json::json!({}),
        }
    }
}

pub fn create_platform() -> Option<Box<dyn DesktopPlatform>> {
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(macos::MacosDesktop))
    }
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(windows::WindowsDesktop))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}
