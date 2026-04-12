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
