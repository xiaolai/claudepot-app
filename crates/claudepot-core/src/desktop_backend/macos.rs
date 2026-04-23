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

    fn is_installed(&self) -> bool {
        // Authoritative on macOS: the app bundle lives at a fixed
        // path. `/Applications/Claude.app` existing is sufficient —
        // we don't need to launch it, load Info.plist, or touch
        // LaunchServices. A user-installed copy under
        // ~/Applications is also accepted, mirroring how macOS
        // treats per-user installs.
        std::path::Path::new("/Applications/Claude.app").is_dir()
            || dirs::home_dir()
                .map(|h| h.join("Applications/Claude.app").is_dir())
                .unwrap_or(false)
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

    async fn safe_storage_secret(&self) -> Result<Vec<u8>, super::DesktopKeyError> {
        // Subprocess-only per .claude/rules/architecture.md — the
        // `keyring` crate is reserved for Claudepot's own secrets.
        // `-s` names the service, `-a` the account, `-w` prints the
        // password to stdout. The trailing newline must be stripped
        // before PBKDF2 so derive_key sees the exact byte string
        // Chromium used.
        let out = tokio::process::Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Safe Storage",
                "-a",
                "Claude Key",
                "-w",
            ])
            .output()
            .await
            .map_err(|e| super::DesktopKeyError::KeychainRead(e.to_string()))?;

        if !out.status.success() {
            let code = out.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(super::DesktopKeyError::KeychainRead(format!(
                "security exited {code}: {}",
                stderr.trim()
            )));
        }

        let mut pw = out.stdout;
        // `security -w` appends exactly one trailing newline on
        // success. Trim only the trailing newline — the password
        // itself could contain whitespace.
        if pw.last() == Some(&b'\n') {
            pw.pop();
        }
        if pw.is_empty() {
            return Err(super::DesktopKeyError::KeychainRead(
                "keychain returned empty password".into(),
            ));
        }
        Ok(pw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_backend::DesktopPlatform;

    #[test]
    fn test_is_installed_returns_bool() {
        // We can't assume Claude.app is present on every CI host, but
        // we can assert the call is well-formed and stable. The result
        // must match whichever of the two candidate paths exists.
        let p = MacosDesktop;
        let system = std::path::Path::new("/Applications/Claude.app").is_dir();
        let user = dirs::home_dir()
            .map(|h| h.join("Applications/Claude.app").is_dir())
            .unwrap_or(false);
        assert_eq!(p.is_installed(), system || user);
    }
}
