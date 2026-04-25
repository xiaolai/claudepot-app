//! Private credential storage: Keychain on macOS (when available), files elsewhere.
//!
//! On macOS with a valid code-signing identity, credentials are stored in the
//! login Keychain via `/usr/bin/security`. Unsigned/ad-hoc-signed debug builds
//! fall back silently to file storage so `cargo test` works without signing.
//!
//! On Linux/Windows and when CLAUDEPOT_CREDENTIAL_BACKEND=file, blobs live at:
//!   `<claudepot_data_dir>/credentials/<uuid>.json`  (0600 on Unix)
//!
//! Migration: load reads from Keychain first; on miss, if a file exists, it's
//! imported into Keychain and the file is removed.

use crate::error::SwapError;
use uuid::Uuid;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.claudepot.credentials";

/// Backend selector. Reads CLAUDEPOT_CREDENTIAL_BACKEND env var:
/// - "file"    → always file, no Keychain attempts (used by tests)
/// - "keyring" → always Keychain (fail closed if unavailable)
/// - unset/other → auto: Keychain on macOS if it works, else file
#[derive(Copy, Clone, PartialEq)]
enum CredBackend {
    FileOnly,
    KeyringOnly,
    Auto,
}

fn backend() -> CredBackend {
    match std::env::var("CLAUDEPOT_CREDENTIAL_BACKEND")
        .ok()
        .as_deref()
    {
        Some("file") => CredBackend::FileOnly,
        Some("keyring") => CredBackend::KeyringOnly,
        _ => CredBackend::Auto,
    }
}

pub(crate) fn private_path(account_id: Uuid) -> std::path::PathBuf {
    crate::paths::claudepot_data_dir()
        .join("credentials")
        .join(format!("{}.json", account_id))
}

fn save_to_file(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    let path = private_path(account_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or(std::path::Path::new(".")))?;
    std::io::Write::write_all(&mut tmp, blob.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    tmp.persist(&path)
        .map_err(|e| SwapError::WriteFailed(format!("persist failed: {e}")))?;
    // Apply user-only access on the persisted target. Unix already
    // has 0600 from the tempfile; Windows requires an explicit DACL.
    super::secret_file::harden_path(&path)?;
    Ok(())
}

fn load_from_file(account_id: Uuid) -> Result<String, SwapError> {
    let path = private_path(account_id);

    // Verify file permissions / ACL before reading credentials.
    // Cross-platform: 0600 on Unix, user-only DACL on Windows.
    super::secret_file::verify_path(&path)?;

    std::fs::read_to_string(&path).map_err(|_| SwapError::NoStoredCredentials(account_id))
}

fn delete_file(account_id: Uuid) -> Result<(), SwapError> {
    let path = private_path(account_id);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

// Use /usr/bin/security directly — the `keyring` crate's SecItem-based
// approach silently succeeds but writes to an ephemeral per-app keychain
// on Developer ID-signed binaries without a provisioning profile.

/// Reject characters that would let a caller break out of the
/// `security -i` quoted command line. Same allowlist as the CC keychain
/// helper in `keychain.rs::validate_security_input`.
#[cfg(target_os = "macos")]
fn validate_keychain_attr(value: &str) -> std::io::Result<()> {
    if value.contains('"') || value.contains('\n') || value.contains('\r') || value.contains('\\') {
        return Err(std::io::Error::other(
            "keychain attribute contains unsafe characters",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn save_to_keyring(account_id: Uuid, blob: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let account = account_id.to_string();
    validate_keychain_attr(&account)?;
    validate_keychain_attr(KEYCHAIN_SERVICE)?;

    // Idempotent delete; ignore failure (the item may not exist).
    let _ = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-a",
            &account,
            "-s",
            KEYCHAIN_SERVICE,
        ])
        .output();

    // Hardened write path:
    //   1. Drop `-A` — never grant blanket access. The keychain item
    //      defaults to the calling executable having access via the
    //      partition list; that is what we want.
    //   2. Pass blob via `-X <hex>` over stdin to `security -i` so the
    //      blob never appears in argv (argv is world-readable on macOS
    //      via `ps`/`lsof`).
    let hex_value = hex::encode(blob.as_bytes());
    let command_line = format!(
        "add-generic-password -U -a \"{account}\" -s \"{KEYCHAIN_SERVICE}\" -X \"{hex_value}\"\n"
    );

    let mut child = Command::new("/usr/bin/security")
        .args(["-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(command_line.as_bytes())?;
        // Closing stdin signals end-of-input to `security -i`.
        drop(stdin);
    }

    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "security add-generic-password failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    // Read-back verification — `security -i` returns 0 even when the
    // inner command silently fails (TCC denial, ACL gate). Only confirm
    // success once we observe the blob actually landed.
    match load_from_keyring(account_id)? {
        Some(stored) if stored == blob => Ok(()),
        Some(_) => Err(std::io::Error::other(
            "keyring write did not take effect (read-back mismatch)",
        )),
        None => Err(std::io::Error::other(
            "keyring write returned success but item is absent on read-back",
        )),
    }
}

#[cfg(target_os = "macos")]
fn load_from_keyring(account_id: Uuid) -> std::io::Result<Option<String>> {
    use std::process::Command;
    let out = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-a",
            &account_id.to_string(),
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
        ])
        .output()?;
    if out.status.success() {
        let blob = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        Ok(Some(blob))
    } else {
        let code = out.status.code().unwrap_or(-1);
        if code == 44 {
            Ok(None)
        } else if code == 36 {
            Err(std::io::Error::other(
                "macOS login keychain is locked — open Keychain Access and \
                 unlock the \"login\" keychain, then retry",
            ))
        } else {
            Err(std::io::Error::other(format!(
                "security find-generic-password failed (code {code}): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

#[cfg(target_os = "macos")]
fn delete_from_keyring(account_id: Uuid) -> std::io::Result<()> {
    use std::process::Command;
    let out = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-a",
            &account_id.to_string(),
            "-s",
            KEYCHAIN_SERVICE,
        ])
        .output()?;
    if out.status.success() || out.status.code() == Some(44) {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "security delete-generic-password failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

#[cfg(not(target_os = "macos"))]
fn save_to_keyring(_account_id: Uuid, _blob: &str) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "keyring backend is only implemented on macOS",
    ))
}

#[cfg(not(target_os = "macos"))]
fn load_from_keyring(_account_id: Uuid) -> std::io::Result<Option<String>> {
    Err(std::io::Error::other(
        "keyring backend is only implemented on macOS",
    ))
}

#[cfg(not(target_os = "macos"))]
fn delete_from_keyring(_account_id: Uuid) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "keyring backend is only implemented on macOS",
    ))
}

pub fn save(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    match backend() {
        CredBackend::FileOnly => save_to_file(account_id, blob),
        CredBackend::KeyringOnly => save_to_keyring(account_id, blob)
            .map_err(|e| SwapError::WriteFailed(format!("keyring: {e}"))),
        CredBackend::Auto => match save_to_keyring(account_id, blob) {
            Ok(()) => {
                let _ = delete_file(account_id);
                Ok(())
            }
            Err(e) => {
                tracing::warn!("keyring save failed ({e}); falling back to file storage");
                save_to_file(account_id, blob)
            }
        },
    }
}

pub fn load(account_id: Uuid) -> Result<String, SwapError> {
    match backend() {
        CredBackend::FileOnly => load_from_file(account_id),
        CredBackend::KeyringOnly => match load_from_keyring(account_id) {
            Ok(Some(blob)) => Ok(blob),
            Ok(None) => Err(SwapError::NoStoredCredentials(account_id)),
            Err(e) => Err(SwapError::WriteFailed(format!("keyring: {e}"))),
        },
        CredBackend::Auto => match load_from_keyring(account_id) {
            Ok(Some(blob)) => {
                let _ = delete_file(account_id);
                Ok(blob)
            }
            Ok(None) => match load_from_file(account_id) {
                Ok(blob) => {
                    if save_to_keyring(account_id, &blob).is_ok() {
                        let _ = delete_file(account_id);
                    }
                    Ok(blob)
                }
                Err(e) => Err(e),
            },
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("keychain is locked") {
                    return Err(SwapError::KeychainError(msg));
                }
                tracing::warn!("keyring load failed ({e}); trying file storage");
                load_from_file(account_id)
            }
        },
    }
}

pub fn load_opt(account_id: Uuid) -> Option<String> {
    load(account_id).ok()
}

pub fn delete(account_id: Uuid) -> Result<(), SwapError> {
    match backend() {
        CredBackend::FileOnly => delete_file(account_id),
        CredBackend::KeyringOnly => delete_from_keyring(account_id)
            .map_err(|e| SwapError::WriteFailed(format!("keyring: {e}"))),
        CredBackend::Auto => {
            // Try both backends. On macOS the private slot lives in
            // Keychain; on Linux/headless it's a file. A keychain
            // delete can fail if the keychain is locked or the
            // `security` subprocess errors. Previously the Auto path
            // used `let _ = delete_from_keyring(...)` and returned the
            // file-delete result, silently dropping keychain errors —
            // callers believed the secret was gone but it could still
            // live in the keychain slot.
            //
            // New policy: attempt both, accumulate errors, and return
            // Err iff BOTH backends errored. "Entry not found" from
            // either side is treated as success (idempotent delete).
            let keyring_result = delete_from_keyring(account_id);
            let file_result = delete_file(account_id);

            let keyring_ok = keyring_result.is_ok();
            let file_ok = file_result.is_ok();
            if keyring_ok || file_ok {
                // At least one backend reported success. Surface the
                // other's error as a log warning but don't fail the
                // call — the typical case is "this account was stored
                // in only one of the two backends anyway."
                if let Err(e) = &keyring_result {
                    tracing::warn!(
                        account = %account_id,
                        "keyring delete reported error (file: {}): {e}",
                        if file_ok { "deleted" } else { "also failed" }
                    );
                }
                if let Err(e) = &file_result {
                    tracing::debug!(
                        account = %account_id,
                        "file delete reported error (keyring: deleted): {e}"
                    );
                }
                Ok(())
            } else {
                // Both backends errored — propagate. This is the case
                // the old code silently hid: user thought the delete
                // succeeded because only the file-delete result was
                // consulted, even when the keychain held the real blob.
                Err(SwapError::WriteFailed(format!(
                    "both storage backends errored: keyring={}, file={}",
                    keyring_result.err().map(|e| e.to_string()).unwrap_or_default(),
                    file_result.err().map(|e| e.to_string()).unwrap_or_default()
                )))
            }
        }
    }
}
