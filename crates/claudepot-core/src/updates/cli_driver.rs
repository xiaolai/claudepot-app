//! Drive `claude update` to force a CC CLI update right now.
//!
//! We never touch CC's binaries directly. The native installer's own
//! update routine handles symlink swap + per-version locks + channel
//! + `minimumVersion` semantics. Our job is to spawn the subprocess,
//! capture its output, surface the result, and refuse cleanly if the
//! user has opted out via `DISABLE_UPDATES=1`.

use crate::updates::detect::{detect_cli_installs, CliInstall, CliInstallKind};
use crate::updates::errors::{Result, UpdateError};
use crate::updates::settings_bridge;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Outcome of a `claude update` invocation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CliUpdateOutcome {
    pub stdout: String,
    pub stderr: String,
    /// Version string detected from the active install AFTER the
    /// update completed. None if no install is active or the version
    /// probe failed.
    pub installed_after: Option<String>,
}

/// Run an in-place update against the active install.
///
/// Routing depends on the install kind:
/// - **Native curl / npm**: invoke `<binary> update` (CC's own
///   updater handles symlink swap + locks atomically).
/// - **Homebrew / WinGet / apt / dnf / apk**: refuse with the
///   correct package-manager command. CC's `update` subcommand on
///   these kinds prints package-manager instructions and exits 0,
///   which would silently leave the install unchanged and convince
///   the auto-installer it had succeeded.
/// - **Unknown**: best-effort try `<binary> update`; if the binary
///   actually has its own auto-updater wired in, this works; if not
///   we surface the subprocess error to the user.
///
/// Always refuses if `DISABLE_UPDATES=1` is set in CC's
/// `~/.claude/settings.json`.
pub async fn run_claude_update() -> Result<CliUpdateOutcome> {
    let cc = settings_bridge::read().unwrap_or_default();
    if cc.disable_updates {
        return Err(UpdateError::Refused(
            "DISABLE_UPDATES is set in ~/.claude/settings.json — manual update path is blocked"
                .into(),
        ));
    }

    let installs = detect_cli_installs();
    let active = installs
        .iter()
        .find(|c| c.is_active)
        .cloned()
        .ok_or_else(|| UpdateError::Refused("no active `claude` binary on PATH".into()))?;

    match active.kind {
        CliInstallKind::NativeCurl | CliInstallKind::NpmGlobal | CliInstallKind::Unknown => {
            invoke_claude_update(&active.binary_path).await
        }
        CliInstallKind::HomebrewStable => Err(UpdateError::Refused(
            "active CC install is Homebrew-managed; run `brew upgrade --cask claude-code`".into(),
        )),
        CliInstallKind::HomebrewLatest => Err(UpdateError::Refused(
            "active CC install is Homebrew-managed; run `brew upgrade --cask claude-code@latest`"
                .into(),
        )),
        CliInstallKind::Apt => Err(UpdateError::Refused(
            "active CC install is apt-managed; run `sudo apt update && sudo apt upgrade claude-code`"
                .into(),
        )),
        CliInstallKind::Dnf => Err(UpdateError::Refused(
            "active CC install is dnf-managed; run `sudo dnf upgrade claude-code`".into(),
        )),
        CliInstallKind::Apk => Err(UpdateError::Refused(
            "active CC install is apk-managed; run `apk update && apk upgrade claude-code`".into(),
        )),
        CliInstallKind::WinGet => Err(UpdateError::Refused(
            "active CC install is WinGet-managed; run `winget upgrade Anthropic.ClaudeCode`".into(),
        )),
    }
}

async fn invoke_claude_update(bin: &std::path::Path) -> Result<CliUpdateOutcome> {
    // 5 minute hard timeout. Updates are typically <30s; 5 min covers
    // slow networks without leaving zombie shells around forever.
    let fut = Command::new(bin)
        .arg("update")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let output = timeout(Duration::from_secs(300), fut)
        .await
        .map_err(|_| UpdateError::Refused("`claude update` timed out after 5 minutes".into()))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                UpdateError::ToolMissing(bin.display().to_string())
            } else {
                UpdateError::Io(e)
            }
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(UpdateError::Subprocess {
            cmd: format!("{} update", bin.display()),
            status: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    // Re-detect to capture the post-update version. The active
    // install may have moved (symlink swap), so we re-resolve from
    // scratch instead of trusting the pre-update binary path.
    let installed_after = detect_cli_installs()
        .into_iter()
        .find(|c: &CliInstall| c.is_active)
        .and_then(|c| c.version);

    Ok(CliUpdateOutcome {
        stdout,
        stderr,
        installed_after,
    })
}
