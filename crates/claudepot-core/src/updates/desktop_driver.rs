//! Drive Claude Desktop updates: brew when brew-managed, direct .zip
//! download otherwise.
//!
//! Both paths refuse if Desktop is currently running. The .zip path
//! verifies SHA256 against the value the Homebrew Cask publishes,
//! then verifies the extracted .app's code signature names
//! "Anthropic" before touching `/Applications`. Backup-and-replace
//! via `ditto` so a botched copy is recoverable.

// Imports split by platform — `DesktopSource`, `DesktopRelease`,
// and `fetch_desktop_latest` are only consumed inside the macOS
// branch of `install_desktop_latest`, so on Linux they are
// "unused import" lints (= errors under CI's -D warnings).
use crate::updates::detect::{detect_desktop_install, is_desktop_running, DesktopInstall};
#[cfg(target_os = "macos")]
use crate::updates::detect::DesktopSource;
use crate::updates::errors::{Result, UpdateError};
use crate::updates::version::DesktopRelease;
#[cfg(target_os = "macos")]
use crate::updates::version::fetch_desktop_latest;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DesktopUpdateOutcome {
    pub method: String,
    pub version_after: Option<String>,
    pub stdout: String,
    pub stderr: String,
}

/// Drive a Desktop update. Picks the right install path based on
/// detected source-of-truth. Refuses if Desktop is currently running.
///
/// Routing:
/// - macOS Homebrew Cask → `brew upgrade --cask claude`
/// - macOS direct DMG → fetch zip + verify codesign + ditto into place
/// - Windows WinGet → `winget upgrade Anthropic.ClaudeCode`
/// - Windows Squirrel direct install → refused (Squirrel-Windows
///   only updates while Desktop is running, which is outside our
///   precondition)
/// - Setapp / Mac App Store / user-local → refused as "managed elsewhere"
pub async fn install_desktop_latest() -> Result<DesktopUpdateOutcome> {
    if is_desktop_running() {
        return Err(UpdateError::Refused(
            "Claude Desktop is currently running — quit it first or wait for the next periodic check"
                .into(),
        ));
    }
    let install = detect_desktop_install()
        .ok_or_else(|| UpdateError::Refused("no Claude Desktop install detected".into()))?;
    if !install.manageable {
        return Err(UpdateError::Refused(format!(
            "Desktop is managed by {} — Claudepot can't drive updates here",
            install.source.label()
        )));
    }
    #[cfg(target_os = "macos")]
    {
        match install.source {
            DesktopSource::Homebrew => brew_upgrade_cask().await,
            DesktopSource::DirectDmg => {
                let release = fetch_desktop_latest().await?;
                install_via_zip(&release, &install).await
            }
            _ => Err(UpdateError::UnsupportedPlatform),
        }
    }
    #[cfg(target_os = "windows")]
    {
        // On Windows, the `Homebrew` source variant is reused as
        // the "package-manager managed" lane (see detect.rs); for
        // Windows it actually means WinGet. Keeping one variant
        // across platforms avoids a parallel enum just to label
        // a routing decision.
        match install.source {
            DesktopSource::Homebrew => winget_upgrade_cask().await,
            _ => Err(UpdateError::Refused(
                "Squirrel-Windows direct installs update themselves — open Claude Desktop to receive updates"
                    .into(),
            )),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = install;
        Err(UpdateError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "windows")]
async fn winget_upgrade_cask() -> Result<DesktopUpdateOutcome> {
    let fut = Command::new("winget")
        .args([
            "upgrade",
            "--exact",
            "--id",
            "Anthropic.ClaudeCode",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let output = timeout(Duration::from_secs(600), fut)
        .await
        .map_err(|_| {
            UpdateError::Refused("`winget upgrade Anthropic.ClaudeCode` timed out".into())
        })?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                UpdateError::ToolMissing("winget".into())
            } else {
                UpdateError::Io(e)
            }
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(UpdateError::Subprocess {
            cmd: "winget upgrade Anthropic.ClaudeCode".into(),
            status: output.status.code().unwrap_or(-1),
            stderr,
        });
    }
    let version_after = detect_desktop_install().and_then(|i| i.version);
    Ok(DesktopUpdateOutcome {
        method: "winget".into(),
        version_after,
        stdout,
        stderr,
    })
}

// Only the macOS routing branch invokes brew_upgrade_cask; on
// Linux the whole `match` arm is cfg'd away, so the helper would
// be dead code without this gate.
#[cfg(target_os = "macos")]
async fn brew_upgrade_cask() -> Result<DesktopUpdateOutcome> {
    let fut = Command::new("brew")
        .args(["upgrade", "--cask", "claude"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let output = timeout(Duration::from_secs(600), fut)
        .await
        .map_err(|_| UpdateError::Refused("`brew upgrade --cask claude` timed out".into()))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                UpdateError::ToolMissing("brew".into())
            } else {
                UpdateError::Io(e)
            }
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(UpdateError::Subprocess {
            cmd: "brew upgrade --cask claude".into(),
            status: output.status.code().unwrap_or(-1),
            stderr,
        });
    }
    let version_after = detect_desktop_install().and_then(|i| i.version);
    Ok(DesktopUpdateOutcome {
        method: "brew".into(),
        version_after,
        stdout,
        stderr,
    })
}

#[cfg(target_os = "macos")]
async fn install_via_zip(
    release: &DesktopRelease,
    install: &DesktopInstall,
) -> Result<DesktopUpdateOutcome> {
    use sha2::{Digest, Sha256};

    let tmp_dir = tempfile::tempdir()?;
    let zip_path = tmp_dir.path().join("Claude.zip");

    // 15 min download timeout; the zip is ~150 MB so a slow
    // connection could legitimately take a while.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(900))
        .user_agent(concat!("Claudepot/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client");
    let bytes = client
        .get(&release.download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    std::fs::write(&zip_path, &bytes)?;

    if let Some(expected) = &release.sha256 {
        let actual = {
            let body = std::fs::read(&zip_path)?;
            let mut hasher = Sha256::new();
            hasher.update(&body);
            hex::encode(hasher.finalize())
        };
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(UpdateError::Signature(format!(
                "SHA256 mismatch on Claude.zip: expected {expected}, got {actual}"
            )));
        }
    }

    // Extract via /usr/bin/unzip (always present on macOS).
    let extract_dir = tmp_dir.path().join("extracted");
    std::fs::create_dir_all(&extract_dir)?;
    let unzip = Command::new("unzip")
        .args([
            "-q",
            zip_path.to_str().unwrap(),
            "-d",
            extract_dir.to_str().unwrap(),
        ])
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                UpdateError::ToolMissing("unzip".into())
            } else {
                UpdateError::Io(e)
            }
        })?;
    if !unzip.status.success() {
        return Err(UpdateError::Subprocess {
            cmd: "unzip".into(),
            status: unzip.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&unzip.stderr).into_owned(),
        });
    }

    let new_app = extract_dir.join("Claude.app");
    if !new_app.exists() {
        return Err(UpdateError::Parse(
            "extracted zip did not contain Claude.app".into(),
        ));
    }

    // Verify the new app's code signature. This catches MITM, server
    // compromise, and malformed cask metadata.
    verify_codesign(&new_app)?;

    // Re-check Desktop isn't running before we touch the install.
    if is_desktop_running() {
        return Err(UpdateError::Refused(
            "Desktop started running mid-install — aborted before replace".into(),
        ));
    }

    // Backup-and-replace. Rename is atomic on a single filesystem;
    // ditto preserves resource forks and metadata. If ditto fails we
    // restore the backup.
    let target = &install.app_path;
    let backup =
        target.with_file_name(format!("Claude.app.bak-{}", chrono::Utc::now().timestamp()));
    if target.exists() {
        std::fs::rename(target, &backup)?;
    }
    let ditto = Command::new("ditto")
        .args([
            "--rsrc",
            new_app.to_str().unwrap(),
            target.to_str().unwrap(),
        ])
        .output()
        .await
        .map_err(|e| {
            // Try to restore backup on infrastructural failure.
            if backup.exists() {
                let _ = std::fs::rename(&backup, target);
            }
            if e.kind() == std::io::ErrorKind::NotFound {
                UpdateError::ToolMissing("ditto".into())
            } else {
                UpdateError::Io(e)
            }
        })?;
    if !ditto.status.success() {
        if backup.exists() {
            let _ = std::fs::rename(&backup, target);
        }
        return Err(UpdateError::Subprocess {
            cmd: "ditto".into(),
            status: ditto.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&ditto.stderr).into_owned(),
        });
    }

    // Strip quarantine attribute so first-launch doesn't show
    // Gatekeeper. Best-effort — failure here doesn't block the
    // update, just means the user sees one extra dialog.
    let _ = Command::new("xattr")
        .args(["-rd", "com.apple.quarantine", target.to_str().unwrap()])
        .output()
        .await;

    // Trash the backup (recoverable via macOS Trash). We don't unlink
    // because `ditto` may have failed silently and the backup is the
    // user's escape hatch for ~24 hours.
    if backup.exists() {
        let _ = trash::delete(&backup);
    }

    let version_after = detect_desktop_install().and_then(|i| i.version);
    Ok(DesktopUpdateOutcome {
        method: "direct-zip".into(),
        version_after,
        stdout: format!(
            "Installed Claude.app {} via direct download",
            release.version
        ),
        stderr: String::new(),
    })
}

#[cfg(not(target_os = "macos"))]
async fn install_via_zip(
    _release: &DesktopRelease,
    _install: &DesktopInstall,
) -> Result<DesktopUpdateOutcome> {
    Err(UpdateError::UnsupportedPlatform)
}

#[cfg(target_os = "macos")]
fn verify_codesign(app: &Path) -> Result<()> {
    // Step 1: signature integrity (--verify --deep --strict).
    let verify = std::process::Command::new("codesign")
        .args(["--verify", "--deep", "--strict", app.to_str().unwrap()])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                UpdateError::ToolMissing("codesign".into())
            } else {
                UpdateError::Io(e)
            }
        })?;
    if !verify.status.success() {
        return Err(UpdateError::Signature(format!(
            "codesign --verify failed: {}",
            String::from_utf8_lossy(&verify.stderr)
        )));
    }
    // Step 2: leaf authority must be exactly "Developer ID Application:
    // Anthropic, PBC". `-dv --verbose=4` writes the authority chain to
    // stderr in the format `Authority=<name>` per line. A naive
    // substring match for "Anthropic" anywhere in the output would
    // pass for any cert whose subject happens to mention Anthropic in
    // an unrelated field; pinning the full leaf-authority line is the
    // tight check.
    const EXPECTED_LEAF_AUTHORITY: &str = "Authority=Developer ID Application: Anthropic, PBC";
    let dv = std::process::Command::new("codesign")
        .args(["-dv", "--verbose=4", app.to_str().unwrap()])
        .output()?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&dv.stdout),
        String::from_utf8_lossy(&dv.stderr)
    );
    let has_leaf = combined
        .lines()
        .any(|l| l.trim() == EXPECTED_LEAF_AUTHORITY);
    if !has_leaf {
        return Err(UpdateError::Signature(format!(
            "codesign leaf authority did not match `{EXPECTED_LEAF_AUTHORITY}`: {}",
            combined.lines().take(8).collect::<Vec<_>>().join(" / ")
        )));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn verify_codesign(_app: &Path) -> Result<()> {
    Err(UpdateError::UnsupportedPlatform)
}
