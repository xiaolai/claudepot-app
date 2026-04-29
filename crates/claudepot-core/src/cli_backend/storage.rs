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

/// Typed outcome from a `/usr/bin/security` invocation. Distinguishes
/// "the keychain told us the item isn't there" (safe to fall back to
/// file storage) from real failures like a locked login keychain or
/// a TCC/ACL denial (which must NOT fall back, otherwise an attacker
/// who can deny keychain access can force the load to read stale or
/// attacker-controlled file storage).
///
/// Audit fix for storage.rs:289 / storage.rs:328: callers must
/// inspect this outcome explicitly. Anything other than `NotFound`
/// causes the calling public API (`load`/`delete`) to fail closed.
#[cfg(target_os = "macos")]
#[derive(Debug)]
enum KeychainErr {
    /// macOS login keychain is locked (`security` exit 36, errSecAuthFailed).
    /// User-recoverable — surface a clear message asking them to
    /// unlock it in Keychain Access.
    Locked,
    /// Anything else: TCC/ACL denial, malformed binary path,
    /// command exited with an unrecognized code, or an I/O error
    /// spawning the subprocess. We cannot distinguish "denied" from
    /// "broken" reliably from the outside, so they share a variant —
    /// but the calling contract is the same: fail closed, never fall
    /// back to file storage.
    Other(String),
}

#[cfg(target_os = "macos")]
impl std::fmt::Display for KeychainErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Locked => write!(
                f,
                "macOS login keychain is locked — open Keychain Access and \
                 unlock the \"login\" keychain, then retry"
            ),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(target_os = "macos")]
impl From<KeychainErr> for SwapError {
    fn from(e: KeychainErr) -> Self {
        SwapError::KeychainError(e.to_string())
    }
}

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
fn save_to_keyring(account_id: Uuid, blob: &str) -> Result<(), KeychainErr> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let account = account_id.to_string();
    validate_keychain_attr(&account).map_err(|e| KeychainErr::Other(e.to_string()))?;
    validate_keychain_attr(KEYCHAIN_SERVICE).map_err(|e| KeychainErr::Other(e.to_string()))?;

    // Audit fix for storage.rs:113 — DO NOT pre-delete. The previous
    // shape ran `delete-generic-password` unconditionally before
    // `add -U`, so a subsequent add failure (TCC denial, ACL gate)
    // left the slot empty when the user previously had a working
    // copy. `add-generic-password -U` is documented as
    // update-or-create; the pre-delete bought nothing and could
    // destroy the only copy.
    //
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
        .spawn()
        .map_err(|e| KeychainErr::Other(format!("spawn /usr/bin/security: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(command_line.as_bytes())
            .map_err(|e| KeychainErr::Other(format!("stdin write: {e}")))?;
        // Closing stdin signals end-of-input to `security -i`.
        drop(stdin);
    }

    let out = child
        .wait_with_output()
        .map_err(|e| KeychainErr::Other(format!("wait: {e}")))?;
    if !out.status.success() {
        return Err(KeychainErr::Other(format!(
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
        Some(_) => Err(KeychainErr::Other(
            "keyring write did not take effect (read-back mismatch)".into(),
        )),
        None => Err(KeychainErr::Other(
            "keyring write returned success but item is absent on read-back".into(),
        )),
    }
}

#[cfg(target_os = "macos")]
fn load_from_keyring(account_id: Uuid) -> Result<Option<String>, KeychainErr> {
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
        .output()
        .map_err(|e| KeychainErr::Other(format!("spawn /usr/bin/security: {e}")))?;
    if out.status.success() {
        let blob = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        Ok(Some(blob))
    } else {
        let code = out.status.code().unwrap_or(-1);
        // Exit 44 = errSecItemNotFound. This is the only code that
        // means "item not in keychain"; treat it as a clean miss so
        // the Auto-mode caller can fall back to file storage.
        // Everything else (36 locked, TCC denial, ACL gate, parse
        // failure) is a real problem and must NOT fall back —
        // see the KeychainErr docstring.
        if code == 44 {
            Ok(None)
        } else if code == 36 {
            Err(KeychainErr::Locked)
        } else {
            Err(KeychainErr::Other(format!(
                "security find-generic-password failed (code {code}): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

#[cfg(target_os = "macos")]
fn delete_from_keyring(account_id: Uuid) -> Result<(), KeychainErr> {
    use std::process::Command;
    let out = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-a",
            &account_id.to_string(),
            "-s",
            KEYCHAIN_SERVICE,
        ])
        .output()
        .map_err(|e| KeychainErr::Other(format!("spawn /usr/bin/security: {e}")))?;
    let code = out.status.code().unwrap_or(-1);
    // success or exit 44 (item not found) → idempotent ok.
    if out.status.success() || code == 44 {
        Ok(())
    } else if code == 36 {
        Err(KeychainErr::Locked)
    } else {
        Err(KeychainErr::Other(format!(
            "security delete-generic-password failed (code {code}): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

// Non-macOS shim. Treat as "keychain not available" by always
// returning Ok(None) on read and a typed Other error on write/delete.
// This way, Auto mode on Linux/Windows takes the file-storage path
// without the typed-error machinery getting in the way.
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
enum KeychainErr {
    Other(String),
}

#[cfg(not(target_os = "macos"))]
impl std::fmt::Display for KeychainErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl From<KeychainErr> for SwapError {
    fn from(e: KeychainErr) -> Self {
        SwapError::WriteFailed(e.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn save_to_keyring(_account_id: Uuid, _blob: &str) -> Result<(), KeychainErr> {
    Err(KeychainErr::Other(
        "keyring backend is only implemented on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn load_from_keyring(_account_id: Uuid) -> Result<Option<String>, KeychainErr> {
    Err(KeychainErr::Other(
        "keyring backend is only implemented on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn delete_from_keyring(_account_id: Uuid) -> Result<(), KeychainErr> {
    Err(KeychainErr::Other(
        "keyring backend is only implemented on macOS".into(),
    ))
}

pub fn save(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    match backend() {
        CredBackend::FileOnly => save_to_file(account_id, blob),
        CredBackend::KeyringOnly => save_to_keyring(account_id, blob).map_err(SwapError::from),
        CredBackend::Auto => match save_to_keyring(account_id, blob) {
            Ok(()) => {
                let _ = delete_file(account_id);
                Ok(())
            }
            // On macOS, Auto-mode save still falls back to file when
            // the keychain isn't available — the previous behavior is
            // preserved for save (we'd rather store SOMEWHERE than
            // refuse the write entirely if the keychain is broken).
            // The fail-closed discipline lives on the read/delete
            // side, where stale data is the actual hazard.
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
            Err(e) => Err(e.into()),
        },
        CredBackend::Auto => match load_from_keyring(account_id) {
            Ok(Some(blob)) => {
                let _ = delete_file(account_id);
                Ok(blob)
            }
            // Audit fix for storage.rs:289 — only `NotFound` falls
            // back to file. A locked keychain or a TCC/ACL denial
            // means the keychain IS reachable but is refusing us;
            // falling back to file in that state would surface stale
            // data after a real keychain that the attacker just
            // forced unreachable. Fail closed.
            Ok(None) => match load_from_file(account_id) {
                Ok(blob) => {
                    if save_to_keyring(account_id, &blob).is_ok() {
                        let _ = delete_file(account_id);
                    }
                    Ok(blob)
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e.into()),
        },
    }
}

pub fn load_opt(account_id: Uuid) -> Option<String> {
    load(account_id).ok()
}

pub fn delete(account_id: Uuid) -> Result<(), SwapError> {
    match backend() {
        CredBackend::FileOnly => delete_file(account_id),
        CredBackend::KeyringOnly => delete_from_keyring(account_id).map_err(SwapError::from),
        CredBackend::Auto => {
            // Audit fix for storage.rs:328 — fail-closed on real
            // keychain errors. The previous shape returned Ok if
            // EITHER backend reported success, which masked the
            // case where the keychain delete ran into a TCC denial
            // (the secret stays in the keychain) and the file path
            // happened to succeed because no file existed (no-op
            // delete).
            //
            // New policy:
            //   - keychain-side `NotFound` (exit 44) is mapped to
            //     `Ok` inside `delete_from_keyring`, so it never
            //     reaches us as Err.
            //   - any other keychain error is a real failure —
            //     surface it regardless of what the file path did.
            //   - the file path only fails for IO errors; a missing
            //     file is `Ok`. So a file Err is also a real failure.
            let keyring_result = delete_from_keyring(account_id);
            let file_result = delete_file(account_id);

            match (keyring_result, file_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(k), Ok(())) => Err(SwapError::from(k)),
                (Ok(()), Err(f)) => Err(f),
                (Err(k), Err(f)) => Err(SwapError::WriteFailed(format!(
                    "both storage backends errored: keyring={k}, file={f}"
                ))),
            }
        }
    }
}
