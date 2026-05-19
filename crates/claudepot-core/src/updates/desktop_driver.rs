//! Drive Claude Desktop updates: brew when brew-managed, direct .zip
//! download otherwise.
//!
//! Both paths refuse if Desktop is currently running. The .zip path
//! verifies SHA256 against the value the Homebrew Cask publishes,
//! then verifies the extracted .app's code signature names
//! "Anthropic" before touching `/Applications`. Backup-and-replace
//! via `ditto` so a botched copy is recoverable.

// Imports split by platform so Linux clippy's -D warnings doesn't
// flag them as unused:
//
//   - `DesktopSource` is the routing enum, used in both the macOS
//     `match install.source { Homebrew => brew_upgrade_cask, ... }`
//     and the Windows `match install.source { Homebrew =>
//     winget_upgrade_cask, ... }` arms. Gated to `any(macos, windows)`.
//   - `fetch_desktop_latest` is only called from the macOS arm
//     (the Windows path routes through `winget_upgrade_cask` which
//     never needs the version metadata). Gated to `macos` only.
//
// Linux falls into the explicit `cfg(target_os = "linux")` no-op
// branch in `install_desktop_latest`; neither symbol is needed there.
#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::updates::detect::DesktopSource;
use crate::updates::detect::{detect_desktop_install, is_desktop_running, DesktopInstall};
use crate::updates::errors::{Result, UpdateError};
#[cfg(target_os = "macos")]
use crate::updates::version::fetch_desktop_latest;
use crate::updates::version::DesktopRelease;
use std::path::Path;
// Process / timeout machinery is only used by `brew_upgrade_cask`
// (macOS), `winget_upgrade_cask` (windows), and the macOS
// `install_via_zip` — i.e. the platforms with a real updater path.
// Linux falls into the no-op branch and never touches a subprocess.
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Stdio;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tokio::process::Command;
#[cfg(any(target_os = "macos", target_os = "windows"))]
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
#[allow(dead_code)]
async fn install_via_zip(
    _release: &DesktopRelease,
    _install: &DesktopInstall,
) -> Result<DesktopUpdateOutcome> {
    Err(UpdateError::UnsupportedPlatform)
}

/// Outcome of a startup orphan-backup recovery sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanRecovery {
    /// No recovery needed — target exists OR no orphan backup was found.
    NoOp,
    /// Restored from `backup_path` → `target_path`.
    Restored {
        backup_path: std::path::PathBuf,
        target_path: std::path::PathBuf,
    },
    /// Recovery candidate found but the restore itself failed.
    /// Surfaces as a logged warning at the call site; included as a
    /// distinct variant for tests.
    Failed {
        backup_path: std::path::PathBuf,
        target_path: std::path::PathBuf,
        error: String,
    },
}

/// Recover a Claude.app that went missing because `install_via_zip`
/// was interrupted between the `fs::rename(target, &backup)` and the
/// `ditto`-restore branch (SIGKILL, OOM, hard reboot mid-install).
///
/// Scans `target.parent()` for `Claude.app.bak-<unix-ts>` siblings,
/// picks the highest timestamp (most recent backup), and renames it
/// back into place. Idempotent: a no-op when the target exists or no
/// orphan backup is found. Both renames happen on the same filesystem
/// — atomic; no risk of leaving partial state if THIS call is itself
/// interrupted (the source still exists under its bak-<ts> name).
///
/// Call once at startup, before `detect_desktop_install` runs in any
/// UI surface that asks "is Claude.app installed?" — otherwise the
/// user sees a phantom "Desktop not installed" state while a usable
/// backup sits next to where it should be.
///
/// Calls the platform-default verifier (`verify_codesign` on macOS,
/// no-op elsewhere) post-restore. For test injection, use
/// [`recover_orphan_backup_with_verifier`].
pub fn recover_orphan_backup(target: &Path) -> OrphanRecovery {
    recover_orphan_backup_with_verifier(target, default_post_restore_verifier)
}

/// Default post-restore verifier: re-checks the codesign of the
/// restored bundle on macOS, no-op elsewhere. Pulled out so the
/// startup path uses the real verifier and tests inject a no-op
/// without compiling-out the verify branch.
fn default_post_restore_verifier(target: &Path) -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        verify_codesign(target).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = target;
        Ok(())
    }
}

/// Test-injectable variant of [`recover_orphan_backup`]. The
/// `verifier` runs post-rename against the restored target path;
/// returning `Err(msg)` triggers the quarantine-aside branch. The
/// public `recover_orphan_backup` passes [`default_post_restore_verifier`].
pub fn recover_orphan_backup_with_verifier(
    target: &Path,
    verifier: fn(&Path) -> std::result::Result<(), String>,
) -> OrphanRecovery {
    if target.exists() {
        return OrphanRecovery::NoOp;
    }
    let Some(parent) = target.parent() else {
        return OrphanRecovery::NoOp;
    };
    let Some(target_name) = target.file_name().and_then(|s| s.to_str()) else {
        return OrphanRecovery::NoOp;
    };
    // The backup filename shape comes from `install_via_zip`:
    //     {target_name}.bak-{unix_ts}
    // See the `fs::rename(target, &backup)` site above.
    let prefix = format!("{target_name}.bak-");

    // Find the highest-timestamp backup.
    let Ok(entries) = std::fs::read_dir(parent) else {
        return OrphanRecovery::NoOp;
    };
    let best = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?.to_string();
            let suffix = name.strip_prefix(&prefix)?;
            // Reject anything that's already trashed / partial — the
            // suffix must be a positive integer (`unix_ts` from
            // `chrono::Utc::now().timestamp()`).
            let ts: i64 = suffix.parse().ok()?;
            Some((ts, path))
        })
        .max_by_key(|(ts, _)| *ts);

    let Some((ts, backup_path)) = best else {
        return OrphanRecovery::NoOp;
    };

    match std::fs::rename(&backup_path, target) {
        Ok(()) => {
            // Re-verify the codesign of the restored bundle. The
            // backup was produced by `install_via_zip`'s
            // `fs::rename(target, &backup)`, which already happened
            // on a code-signed install — so a clean recovery should
            // re-verify identically. But the filename timestamp is
            // forgeable: anyone with write access to `target.parent()`
            // (notably `~/Applications/` on user-local installs)
            // can plant a hostile `Claude.app.bak-<future_ts>` and
            // delete the real bundle to trigger orphan recovery on
            // next launch. The attacker still needs user-level code
            // execution to delete the bundle, so this is
            // defense-in-depth rather than a fresh hole — but the
            // cost is one `codesign --verify` per restore, which is
            // cheap and matches the discipline `install_via_zip`
            // uses for the install path.
            //
            // On failure, rename the suspicious bundle aside to a
            // quarantined name so the user can inspect it manually
            // (and we don't auto-recover the same suspicious backup
            // on the next launch). Surface as `OrphanRecovery::Failed`.
            if let Err(verify_err) = verifier(target) {
                let quarantine_path = target.with_file_name(format!(
                    "{target_name}.bak-restored-but-unverified-{ts}"
                ));
                let quarantine_attempted = std::fs::rename(target, &quarantine_path);
                tracing::error!(
                    target = %target.display(),
                    backup = %backup_path.display(),
                    verify_error = %verify_err,
                    quarantine = ?quarantine_attempted.as_ref().ok().map(|_| quarantine_path.display().to_string()),
                    "orphan-backup recovery: restored bundle failed codesign verification; quarantining"
                );
                return OrphanRecovery::Failed {
                    backup_path,
                    target_path: target.to_path_buf(),
                    error: format!(
                        "restored bundle failed codesign verification: {verify_err}"
                    ),
                };
            }

            tracing::warn!(
                target = %target.display(),
                backup = %backup_path.display(),
                backup_ts = ts,
                "recovered orphan Claude.app backup left over from an interrupted update"
            );
            OrphanRecovery::Restored {
                backup_path,
                target_path: target.to_path_buf(),
            }
        }
        Err(e) => {
            tracing::error!(
                target = %target.display(),
                backup = %backup_path.display(),
                error = %e,
                "orphan-backup recovery: rename failed; backup left in place for manual recovery"
            );
            OrphanRecovery::Failed {
                backup_path,
                target_path: target.to_path_buf(),
                error: e.to_string(),
            }
        }
    }
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

/// Run [`recover_orphan_backup`] against every standard Claude.app
/// install location on the current platform. Wired into `lib.rs::run`
/// as a one-shot startup sweep so an interrupted update doesn't leave
/// the user staring at a "Desktop not installed" surface when a
/// recoverable backup is sitting right next to it.
///
/// macOS: `/Applications/Claude.app` (Homebrew + direct DMG) and
/// `~/Applications/Claude.app` (user-local). Other platforms: no-op
/// — Squirrel-Windows handles its own rollback, Linux has no Desktop
/// build to recover.
#[cfg(target_os = "macos")]
pub fn recover_orphan_backups_at_startup() -> Vec<OrphanRecovery> {
    let mut out = Vec::new();
    let system = std::path::PathBuf::from("/Applications/Claude.app");
    out.push(recover_orphan_backup(&system));
    if let Some(home) = dirs::home_dir() {
        let user = home.join("Applications/Claude.app");
        out.push(recover_orphan_backup(&user));
    }
    out
}

#[cfg(not(target_os = "macos"))]
pub fn recover_orphan_backups_at_startup() -> Vec<OrphanRecovery> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test verifier: pretends every restored bundle is codesign-clean.
    /// The real verifier shells out to `/usr/bin/codesign` which would
    /// reject the empty `Claude.app` test fixtures these tests use.
    /// Coverage of the "verifier rejects → quarantine" branch lives in
    /// the dedicated `verifier_failure_*` test below.
    fn ok_verifier(_: &Path) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Test verifier that always rejects. Used by the quarantine tests
    /// to exercise the "restored bundle failed codesign verification"
    /// branch without depending on a real signed fixture.
    fn fail_verifier(_: &Path) -> std::result::Result<(), String> {
        Err("simulated codesign rejection".into())
    }

    /// Set up `<tempdir>/Applications/Claude.app` (the "target") plus
    /// zero or more bak-<ts> siblings. Returns the simulated target
    /// path so tests can pass it to `recover_orphan_backup`.
    fn setup_install_dir(
        td: &TempDir,
        target_exists: bool,
        backups: &[(i64, &str)],
    ) -> std::path::PathBuf {
        let apps = td.path().join("Applications");
        fs::create_dir_all(&apps).unwrap();
        let target = apps.join("Claude.app");
        if target_exists {
            fs::create_dir(&target).unwrap();
            // .app bundles always carry a Contents/ directory; a
            // bare empty directory is enough for these tests.
            fs::create_dir(target.join("Contents")).unwrap();
        }
        for (ts, marker) in backups {
            let bak = apps.join(format!("Claude.app.bak-{ts}"));
            fs::create_dir(&bak).unwrap();
            fs::write(bak.join("marker.txt"), marker).unwrap();
        }
        target
    }

    #[test]
    fn recover_orphan_backup_noop_when_target_exists() {
        let td = TempDir::new().unwrap();
        let target = setup_install_dir(&td, true, &[(1, "old")]);
        let result = recover_orphan_backup_with_verifier(&target, ok_verifier);
        assert_eq!(result, OrphanRecovery::NoOp);
        // Target still in place, backup untouched.
        assert!(target.exists());
        assert!(target.parent().unwrap().join("Claude.app.bak-1").exists());
    }

    #[test]
    fn recover_orphan_backup_noop_when_no_backup_present() {
        let td = TempDir::new().unwrap();
        let target = setup_install_dir(&td, false, &[]);
        let result = recover_orphan_backup_with_verifier(&target, ok_verifier);
        assert_eq!(result, OrphanRecovery::NoOp);
        // Still missing — no restore happened because there was
        // nothing to restore from.
        assert!(!target.exists());
    }

    #[test]
    fn recover_orphan_backup_restores_when_target_missing_and_backup_present() {
        let td = TempDir::new().unwrap();
        let target = setup_install_dir(&td, false, &[(1700000000, "only-bak")]);
        assert!(!target.exists());
        let result = recover_orphan_backup_with_verifier(&target, ok_verifier);
        match result {
            OrphanRecovery::Restored {
                backup_path,
                target_path,
            } => {
                assert_eq!(target_path, target);
                assert_eq!(
                    backup_path.file_name().unwrap().to_str().unwrap(),
                    "Claude.app.bak-1700000000"
                );
            }
            other => panic!("expected Restored, got {other:?}"),
        }
        // Target restored from backup; backup name no longer present.
        assert!(target.exists());
        assert!(!target.parent().unwrap().join("Claude.app.bak-1700000000").exists());
        // The content from the backup followed the rename.
        let marker = fs::read_to_string(target.join("marker.txt")).unwrap();
        assert_eq!(marker, "only-bak");
    }

    #[test]
    fn recover_orphan_backup_picks_highest_timestamp() {
        let td = TempDir::new().unwrap();
        let target = setup_install_dir(
            &td,
            false,
            &[
                (1700000000, "oldest"),
                (1800000000, "newest"),
                (1750000000, "middle"),
            ],
        );
        let result = recover_orphan_backup_with_verifier(&target, ok_verifier);
        assert!(matches!(result, OrphanRecovery::Restored { .. }));
        assert!(target.exists());
        // The newest backup was restored; the other two remain.
        let marker = fs::read_to_string(target.join("marker.txt")).unwrap();
        assert_eq!(marker, "newest");
        assert!(target.parent().unwrap().join("Claude.app.bak-1700000000").exists());
        assert!(target.parent().unwrap().join("Claude.app.bak-1750000000").exists());
        assert!(!target.parent().unwrap().join("Claude.app.bak-1800000000").exists());
    }

    #[test]
    fn recover_orphan_backup_ignores_non_timestamp_suffixes() {
        let td = TempDir::new().unwrap();
        let apps = td.path().join("Applications");
        fs::create_dir_all(&apps).unwrap();
        let target = apps.join("Claude.app");
        // A garbage-named sibling we must NOT pick up — only properly
        // formatted bak-<unix_ts> entries are candidates.
        let bogus = apps.join("Claude.app.bak-NOT-A-TIMESTAMP");
        fs::create_dir(&bogus).unwrap();
        fs::write(bogus.join("marker.txt"), "should-be-ignored").unwrap();

        let result = recover_orphan_backup_with_verifier(&target, ok_verifier);
        assert_eq!(result, OrphanRecovery::NoOp);
        // Target stayed missing; the garbage directory wasn't promoted.
        assert!(!target.exists());
        assert!(bogus.exists());
    }

    /// Verifier rejects the restored bundle (simulating a planted
    /// hostile backup whose codesign doesn't match). The function
    /// must rename the suspicious target aside to a quarantined name
    /// so we don't auto-restore the same hostile bundle on the next
    /// launch, and report `OrphanRecovery::Failed`.
    #[test]
    fn recover_orphan_backup_quarantines_when_verifier_rejects() {
        let td = TempDir::new().unwrap();
        let target = setup_install_dir(&td, false, &[(1900000000, "tainted")]);
        assert!(!target.exists());

        let result = recover_orphan_backup_with_verifier(&target, fail_verifier);
        match result {
            OrphanRecovery::Failed {
                backup_path,
                target_path,
                error,
            } => {
                assert_eq!(target_path, target);
                assert_eq!(
                    backup_path.file_name().unwrap().to_str().unwrap(),
                    "Claude.app.bak-1900000000"
                );
                assert!(
                    error.contains("simulated codesign rejection"),
                    "error must carry the verifier's message: {error}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        // The original target slot is empty (the restore was undone
        // via the quarantine rename) and the suspicious bundle now
        // lives under the quarantined name.
        assert!(!target.exists());
        let quarantine = target
            .parent()
            .unwrap()
            .join("Claude.app.bak-restored-but-unverified-1900000000");
        assert!(
            quarantine.exists(),
            "suspicious bundle must be renamed aside for manual inspection"
        );
        let marker = fs::read_to_string(quarantine.join("marker.txt")).unwrap();
        assert_eq!(marker, "tainted");
        // The original bak-<ts> name is gone because the rename moved
        // it to target first, then to the quarantine name.
        assert!(!target.parent().unwrap().join("Claude.app.bak-1900000000").exists());
    }
}
