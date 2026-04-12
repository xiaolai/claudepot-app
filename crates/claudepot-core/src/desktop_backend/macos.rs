use crate::error::DesktopSwapError;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
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
        Some(dirs::data_dir().expect("no data dir").join("Claude"))
    }

    fn session_items(&self) -> &[&str] {
        SESSION_ITEMS
    }

    async fn is_running(&self) -> bool {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.processes().values().any(|p| {
            let exe = p.exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            exe.contains("/Applications/Claude.app/")
        })
    }

    async fn quit(&self) -> Result<(), DesktopSwapError> {
        // Graceful quit via AppleScript
        let output = tokio::process::Command::new("osascript")
            .args(["-e", "tell application \"Claude\" to quit"])
            .output()
            .await
            .map_err(|e| DesktopSwapError::Io(e))?;

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
        tokio::process::Command::new("open")
            .args(["-a", "Claude"])
            .output()
            .await
            .map_err(|e| DesktopSwapError::Io(e))?;
        Ok(())
    }
}
