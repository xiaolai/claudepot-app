//! Claude Desktop session-file swap.
//! See reference.md Part II.

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

pub mod swap;

use crate::error::DesktopSwapError;
use std::path::PathBuf;

/// Platform-specific Desktop operations.
#[async_trait::async_trait]
pub trait DesktopPlatform: Send + Sync {
    fn data_dir(&self) -> Option<PathBuf>;
    fn session_items(&self) -> &[&str];
    async fn is_running(&self) -> bool;
    async fn quit(&self) -> Result<(), DesktopSwapError>;
    async fn launch(&self) -> Result<(), DesktopSwapError>;
}
