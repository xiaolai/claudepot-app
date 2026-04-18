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
        // Audit H3: send a graceful WM_CLOSE first via taskkill without
        // /F — this asks Claude.exe to close cleanly, matching the
        // macOS AppleScript quit. Only if graceful close doesn't land
        // within the timeout do we fall back to /F.
        //
        // Graceful close lets Electron flush IndexedDB / Local Storage
        // / Session Storage writes in progress; a hard kill can leave
        // partially-written Chromium profile state that breaks the
        // profile on next launch.
        let _ = tokio::process::Command::new("taskkill")
            .args(["/IM", "Claude.exe", "/T"])
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;

        // Poll for graceful exit.
        let graceful_deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < graceful_deadline {
            if !self.is_running().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Graceful quit didn't land in 8s. Escalate to /F as a last
        // resort — still better than force-killing immediately, which
        // was the prior behaviour.
        tracing::warn!("graceful taskkill didn't land in 8s; escalating to /F");
        let _ = tokio::process::Command::new("taskkill")
            .args(["/IM", "Claude.exe", "/T", "/F"])
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;

        // Poll for forced exit.
        let force_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < force_deadline {
            if !self.is_running().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err(DesktopSwapError::DesktopStillRunning)
    }

    async fn launch(&self) -> Result<(), DesktopSwapError> {
        const AUMID: &str = "Claude_pzs8sxrjxfjjc!Claude";
        // Audit M8: check exit status. explorer.exe shell:AppsFolder\...
        // returns non-zero if the AUMID doesn't resolve (Claude not
        // installed, MSIX package name changed, permission issue) —
        // silently dropping that made launch() return Ok even when
        // nothing was launched, and the switch reported success.
        let out = tokio::process::Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{AUMID}"))
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;
        // Explorer commonly returns 1 even on success for shell:
        // protocol activations. Accept exit codes 0 and 1 as success;
        // fail on anything higher (known-fatal codes on Windows).
        let code = out.status.code().unwrap_or(0);
        if code > 1 {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(DesktopSwapError::Io(std::io::Error::other(format!(
                "explorer shell:AppsFolder launch exited {code}: {}",
                stderr.trim()
            ))));
        }
        Ok(())
    }
}
