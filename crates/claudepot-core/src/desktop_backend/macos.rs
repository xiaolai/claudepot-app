use crate::error::DesktopSwapError;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const QUIT_TIMEOUT: Duration = Duration::from_secs(10);

/// macOS session items — the 12 items Kannon swaps per account.
/// See reference.md §II.6.
pub const SESSION_ITEMS: &[&str] = &[
    "config.json",
    "Cookies",
    "Cookies-journal",
    "DIPS",
    "DIPS-wal",
    "Preferences",
    "ant-did",
    "Network Persistent State",
    "fcache",
    "Local Storage",
    "Session Storage",
    "IndexedDB",
];

pub struct MacosDesktop;

#[async_trait::async_trait]
impl super::DesktopPlatform for MacosDesktop {
    fn data_dir(&self) -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("Claude"))
    }

    fn session_items(&self) -> &[&str] {
        SESSION_ITEMS
    }

    async fn is_running(&self) -> bool {
        // Use pgrep instead of sysinfo — sysinfo's exe() returns None
        // for some processes on macOS when running over SSH.
        tokio::process::Command::new("pgrep")
            .args(["-f", "/Applications/Claude.app/"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn quit(&self) -> Result<(), DesktopSwapError> {
        // Graceful quit via AppleScript
        let output = tokio::process::Command::new("osascript")
            .args(["-e", "tell application \"Claude\" to quit"])
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;

        if !output.status.success() {
            tracing::warn!("osascript quit returned non-zero");
        }

        // Poll for exit
        let deadline = Instant::now() + QUIT_TIMEOUT;
        while Instant::now() < deadline {
            if !self.is_running().await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(DesktopSwapError::DesktopStillRunning)
    }

    async fn launch(&self) -> Result<(), DesktopSwapError> {
        // Audit M8: check exit status. `open -a Claude` returns
        // non-zero if the app isn't installed / the bundle can't be
        // resolved. Previously we returned Ok regardless, so the
        // caller recorded a successful switch even when Claude
        // never launched.
        let out = tokio::process::Command::new("open")
            .args(["-a", "Claude"])
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(DesktopSwapError::Io(std::io::Error::other(format!(
                "open -a Claude failed ({}): {}",
                out.status,
                stderr.trim()
            ))));
        }
        Ok(())
    }
}
