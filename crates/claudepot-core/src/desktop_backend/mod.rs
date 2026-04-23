#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

pub mod swap;

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
