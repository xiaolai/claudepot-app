use crate::error::DesktopSwapError;
use std::path::PathBuf;

pub struct WindowsDesktop;

#[async_trait::async_trait]
impl super::DesktopPlatform for WindowsDesktop {
    fn data_dir(&self) -> Option<PathBuf> {
        // MSIX-virtualized path
        dirs::data_local_dir().map(|d| {
            d.join("Packages")
                .join("Claude_pzs8sxrjxfjjc")
                .join("LocalCache")
                .join("Roaming")
                .join("Claude")
        })
    }

    fn session_items(&self) -> &[&str] {
        // Windows adjustments from reference.md §II.3:
        // Cookies under Network/, no fcache, add git-worktrees.json
        &[
            "config.json",
            "Network/Cookies",
            "Network/Cookies-journal",
            "Network/Network Persistent State",
            "DIPS",
            "DIPS-wal",
            "Preferences",
            "ant-did",
            "git-worktrees.json",
            "Local Storage",
            "Session Storage",
            "IndexedDB",
        ]
    }

    async fn is_running(&self) -> bool {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.processes()
            .values()
            .any(|p| p.name().to_string_lossy() == "Claude.exe")
    }

    async fn quit(&self) -> Result<(), DesktopSwapError> {
        let status = tokio::process::Command::new("taskkill")
            .args(["/IM", "Claude.exe", "/T", "/F"])
            .output()
            .await
            .map_err(|e| DesktopSwapError::Io(e))?;

        // taskkill returns 0 even if process not found
        let _ = status;

        // Poll for exit
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if !self.is_running().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Err(DesktopSwapError::DesktopStillRunning)
    }

    async fn launch(&self) -> Result<(), DesktopSwapError> {
        const AUMID: &str = "Claude_pzs8sxrjxfjjc!Claude";
        tokio::process::Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{AUMID}"))
            .output()
            .await
            .map_err(|e| DesktopSwapError::Io(e))?;
        Ok(())
    }
}
