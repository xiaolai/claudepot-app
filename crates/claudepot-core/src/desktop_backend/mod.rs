#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

pub mod crypto;
pub mod swap;
pub mod token_cache;

use crate::error::DesktopSwapError;
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
