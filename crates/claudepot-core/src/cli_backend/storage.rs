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

/// Subprocess timeout for every `/usr/bin/security` invocation in this
/// module. A TCC consent dialog the user doesn't see, a locked login
/// keychain hanging on prompt, or `security` itself stalling must not
/// hang the tokio worker that called us — every call site is reached
/// from the async swap/launcher path, and the only acceptable failure
/// mode under stall is a typed error after a bounded wait.
///
/// 5 s matches the sibling `cli_backend::keychain` `TIMEOUT` for the
/// CC-credentials store. Both subprocesses do the same kind of work
/// against the same login keychain; using the same budget here means
/// a failure in one surface and the other report at the same cadence.
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
#[derive(Debug, Copy, Clone, PartialEq)]
enum CredBackend {
    FileOnly,
    KeyringOnly,
    #[cfg(target_os = "macos")]
    Auto,
}

fn backend() -> CredBackend {
    backend_from(
        std::env::var("CLAUDEPOT_CREDENTIAL_BACKEND")
            .ok()
            .as_deref(),
    )
}

/// Pure env-var → backend mapping (the env read lives in
/// [`backend`]).
fn backend_from(value: Option<&str>) -> CredBackend {
    match value {
        Some("file") => CredBackend::FileOnly,
        Some("keyring") => CredBackend::KeyringOnly,
        // `Auto` resolves to Keychain-first on macOS. On every other
        // platform the keychain stubs always return Err("not
        // implemented"), and the fail-closed logic in `load()` treats
        // that error as a real keychain denial — refusing to fall back
        // to file storage even though credentials were saved there.
        // Default to FileOnly on non-macOS so reads reach the file
        // that saves already write on Windows / Linux.
        #[cfg(not(target_os = "macos"))]
        _ => CredBackend::FileOnly,
        #[cfg(target_os = "macos")]
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
async fn save_to_keyring(account_id: Uuid, blob: &str) -> Result<(), KeychainErr> {
    use age::secrecy::{ExposeSecret, SecretString};
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt as _;
    use tokio::process::Command;

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
    let hex_value = SecretString::from(hex::encode(blob.as_bytes()));
    let command_line = SecretString::from(format!(
        "add-generic-password -U -a \"{account}\" -s \"{KEYCHAIN_SERVICE}\" -X \"{}\"\n",
        hex_value.expose_secret()
    ));

    // Total `save_to_keyring` budget is `2 × KEYCHAIN_TIMEOUT`: this
    // block (the `add-generic-password` subprocess) plus the
    // `load_from_keyring` read-back call below, which carries its own
    // independent `tokio::time::timeout` block.
    //
    // The two timeouts are INDEPENDENT on purpose — combining them
    // (one outer timeout wrapping both subprocess calls) would let a
    // slow write eat into the read-back budget and produce false
    // "read-back mismatch" errors after a near-deadline write. Keeping
    // them separate means each subprocess gets a full 5 s.
    //
    // On timeout the child is dropped, which sends SIGKILL via tokio's
    // `kill_on_drop(true)` semantics for tokio::process::Command — see
    // the docs. A leaked `security` process would block subsequent
    // keychain accesses on the same login keychain until macOS reaps
    // it; the kill avoids that pile-up.
    let result = tokio::time::timeout(KEYCHAIN_TIMEOUT, async {
        let mut child = Command::new("/usr/bin/security")
            .args(["-i"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| KeychainErr::Other(format!("spawn /usr/bin/security: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(command_line.expose_secret().as_bytes())
                .await
                .map_err(|e| KeychainErr::Other(format!("stdin write: {e}")))?;
            // Dropping stdin signals end-of-input to `security -i`.
            drop(stdin);
        }

        child
            .wait_with_output()
            .await
            .map_err(|e| KeychainErr::Other(format!("wait: {e}")))
    })
    .await
    .map_err(|_| KeychainErr::Other("security add-generic-password timed out".into()))?;

    let out = result?;
    if !out.status.success() {
        return Err(KeychainErr::Other(format!(
            "security add-generic-password failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    // Read-back verification — `security -i` returns 0 even when the
    // inner command silently fails (TCC denial, ACL gate). Only confirm
    // success once we observe the blob actually landed. `load_from_keyring`
    // wraps its own subprocess in `tokio::time::timeout(KEYCHAIN_TIMEOUT,
    // ...)`, so this `.await` is bounded — see the design note above the
    // surrounding write block for why the two timeouts stay independent.
    verify_read_back(load_from_keyring(account_id).await?, blob)
}

/// Pure read-back verification decision for [`save_to_keyring`]:
/// success is confirmed only when the freshly-read blob equals what
/// was written. A mismatch or an absent item means the `security -i`
/// add silently failed (TCC denial, ACL gate) despite exit 0.
#[cfg(target_os = "macos")]
fn verify_read_back(stored: Option<String>, expected: &str) -> Result<(), KeychainErr> {
    match stored {
        Some(stored) if stored == expected => Ok(()),
        Some(_) => Err(KeychainErr::Other(
            "keyring write did not take effect (read-back mismatch)".into(),
        )),
        None => Err(KeychainErr::Other(
            "keyring write returned success but item is absent on read-back".into(),
        )),
    }
}

#[cfg(target_os = "macos")]
async fn load_from_keyring(account_id: Uuid) -> Result<Option<String>, KeychainErr> {
    use tokio::process::Command;
    let out = tokio::time::timeout(KEYCHAIN_TIMEOUT, async {
        Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-a",
                &account_id.to_string(),
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
            ])
            .kill_on_drop(true)
            .output()
            .await
    })
    .await
    .map_err(|_| KeychainErr::Other("security find-generic-password timed out".into()))?
    .map_err(|e| KeychainErr::Other(format!("spawn /usr/bin/security: {e}")))?;
    parse_find_output(
        out.status.success(),
        out.status.code().unwrap_or(-1),
        &out.stdout,
        &out.stderr,
    )
}

/// Pure exit-status → outcome mapping for `security
/// find-generic-password`.
///
/// Exit 44 = errSecItemNotFound. This is the only code that means
/// "item not in keychain"; treat it as a clean miss so the Auto-mode
/// caller can fall back to file storage. Everything else (36 locked,
/// TCC denial, ACL gate, parse failure) is a real problem and must
/// NOT fall back — see the KeychainErr docstring.
#[cfg(target_os = "macos")]
fn parse_find_output(
    success: bool,
    code: i32,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Option<String>, KeychainErr> {
    if success {
        let blob = String::from_utf8_lossy(stdout).trim_end().to_string();
        Ok(Some(blob))
    } else if code == 44 {
        Ok(None)
    } else if code == 36 {
        Err(KeychainErr::Locked)
    } else {
        Err(KeychainErr::Other(format!(
            "security find-generic-password failed (code {code}): {}",
            String::from_utf8_lossy(stderr).trim()
        )))
    }
}

#[cfg(target_os = "macos")]
async fn delete_from_keyring(account_id: Uuid) -> Result<(), KeychainErr> {
    use tokio::process::Command;
    let out = tokio::time::timeout(KEYCHAIN_TIMEOUT, async {
        Command::new("/usr/bin/security")
            .args([
                "delete-generic-password",
                "-a",
                &account_id.to_string(),
                "-s",
                KEYCHAIN_SERVICE,
            ])
            .kill_on_drop(true)
            .output()
            .await
    })
    .await
    .map_err(|_| KeychainErr::Other("security delete-generic-password timed out".into()))?
    .map_err(|e| KeychainErr::Other(format!("spawn /usr/bin/security: {e}")))?;
    parse_delete_status(
        out.status.success(),
        out.status.code().unwrap_or(-1),
        &out.stderr,
    )
}

/// Pure exit-status → outcome mapping for `security
/// delete-generic-password`: success or exit 44 (item not found) →
/// idempotent ok; 36 → locked; anything else fails closed.
#[cfg(target_os = "macos")]
fn parse_delete_status(success: bool, code: i32, stderr: &[u8]) -> Result<(), KeychainErr> {
    if success || code == 44 {
        Ok(())
    } else if code == 36 {
        Err(KeychainErr::Locked)
    } else {
        Err(KeychainErr::Other(format!(
            "security delete-generic-password failed (code {code}): {}",
            String::from_utf8_lossy(stderr).trim()
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
async fn save_to_keyring(_account_id: Uuid, _blob: &str) -> Result<(), KeychainErr> {
    Err(KeychainErr::Other(
        "keyring backend is only implemented on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
async fn load_from_keyring(_account_id: Uuid) -> Result<Option<String>, KeychainErr> {
    Err(KeychainErr::Other(
        "keyring backend is only implemented on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
async fn delete_from_keyring(_account_id: Uuid) -> Result<(), KeychainErr> {
    Err(KeychainErr::Other(
        "keyring backend is only implemented on macOS".into(),
    ))
}

pub async fn save(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    match backend() {
        CredBackend::FileOnly => save_to_file(account_id, blob),
        CredBackend::KeyringOnly => save_to_keyring(account_id, blob)
            .await
            .map_err(SwapError::from),
        #[cfg(target_os = "macos")]
        CredBackend::Auto => match save_to_keyring(account_id, blob).await {
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

pub async fn load(account_id: Uuid) -> Result<String, SwapError> {
    match backend() {
        CredBackend::FileOnly => load_from_file(account_id),
        CredBackend::KeyringOnly => match load_from_keyring(account_id).await {
            Ok(Some(blob)) => Ok(blob),
            Ok(None) => Err(SwapError::NoStoredCredentials(account_id)),
            Err(e) => Err(e.into()),
        },
        #[cfg(target_os = "macos")]
        CredBackend::Auto => match classify_auto_load(load_from_keyring(account_id).await) {
            AutoLoadAction::UseKeychain(blob) => {
                let _ = delete_file(account_id);
                Ok(blob)
            }
            AutoLoadAction::FallBackToFile => match load_from_file(account_id) {
                Ok(blob) => {
                    if save_to_keyring(account_id, &blob).await.is_ok() {
                        let _ = delete_file(account_id);
                    }
                    Ok(blob)
                }
                Err(e) => Err(e),
            },
            AutoLoadAction::FailClosed(e) => Err(e.into()),
        },
    }
}

/// What Auto-mode `load` does next, given the keyring read outcome.
#[cfg(target_os = "macos")]
#[derive(Debug)]
enum AutoLoadAction {
    /// Keychain returned the blob — use it (and clean up any stale
    /// file copy).
    UseKeychain(String),
    /// Keychain is reachable and reports a clean miss (exit 44) —
    /// the only state where the file fallback + keychain import is
    /// permitted.
    FallBackToFile,
    /// Keychain errored — fail closed without touching file storage.
    FailClosed(KeychainErr),
}

/// Pure Auto-mode load policy — the audit-fix for storage.rs:289.
///
/// Only a clean keychain miss (`Ok(None)`) falls back to file. A
/// locked keychain or a TCC/ACL denial means the keychain IS
/// reachable but is refusing us; falling back to file in that state
/// would surface stale data after a real keychain that the attacker
/// just forced unreachable. Fail closed.
#[cfg(target_os = "macos")]
fn classify_auto_load(outcome: Result<Option<String>, KeychainErr>) -> AutoLoadAction {
    match outcome {
        Ok(Some(blob)) => AutoLoadAction::UseKeychain(blob),
        Ok(None) => AutoLoadAction::FallBackToFile,
        Err(e) => AutoLoadAction::FailClosed(e),
    }
}

pub async fn load_opt(account_id: Uuid) -> Option<String> {
    load(account_id).await.ok()
}

pub async fn delete(account_id: Uuid) -> Result<(), SwapError> {
    match backend() {
        CredBackend::FileOnly => delete_file(account_id),
        CredBackend::KeyringOnly => delete_from_keyring(account_id)
            .await
            .map_err(SwapError::from),
        #[cfg(target_os = "macos")]
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
            let keyring_result = delete_from_keyring(account_id).await;
            let file_result = delete_file(account_id);
            combine_delete_results(keyring_result, file_result)
        }
    }
}

/// Pure Auto-mode delete policy — the audit-fix for storage.rs:328.
/// Both backends must report success; a real error on EITHER side is
/// surfaced (keychain-side `NotFound` was already mapped to `Ok`
/// inside `delete_from_keyring`, so it never reaches this match).
#[cfg(target_os = "macos")]
fn combine_delete_results(
    keyring: Result<(), KeychainErr>,
    file: Result<(), SwapError>,
) -> Result<(), SwapError> {
    match (keyring, file) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(k), Ok(())) => Err(SwapError::from(k)),
        (Ok(()), Err(f)) => Err(f),
        (Err(k), Err(f)) => Err(SwapError::WriteFailed(format!(
            "both storage backends errored: keyring={k}, file={f}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── backend selection (pure env-var mapping) ───────────────────

    #[test]
    fn test_backend_from_file_value_selects_file_only() {
        assert_eq!(backend_from(Some("file")), CredBackend::FileOnly);
    }

    #[test]
    fn test_backend_from_keyring_value_selects_keyring_only() {
        assert_eq!(backend_from(Some("keyring")), CredBackend::KeyringOnly);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_backend_from_default_is_auto_on_macos() {
        assert_eq!(backend_from(None), CredBackend::Auto);
        assert_eq!(backend_from(Some("garbage")), CredBackend::Auto);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_backend_from_default_is_file_only_off_macos() {
        // The keychain stubs always error off macOS; defaulting to
        // Auto there would fail-closed every read of credentials the
        // save path put in a file. See the comment in `backend_from`.
        assert_eq!(backend_from(None), CredBackend::FileOnly);
        assert_eq!(backend_from(Some("garbage")), CredBackend::FileOnly);
    }

    // ── macOS keyring decision helpers ─────────────────────────────

    #[cfg(target_os = "macos")]
    mod macos {
        use super::super::*;

        // validate_keychain_attr — the `security -i` quoting gate.

        #[test]
        fn test_validate_keychain_attr_accepts_uuid_and_service() {
            validate_keychain_attr(&Uuid::new_v4().to_string()).unwrap();
            validate_keychain_attr(KEYCHAIN_SERVICE).unwrap();
        }

        #[test]
        fn test_validate_keychain_attr_rejects_quote_breakout_chars() {
            for bad in ["a\"b", "a\nb", "a\rb", "a\\b"] {
                assert!(
                    validate_keychain_attr(bad).is_err(),
                    "should reject {bad:?}"
                );
            }
        }

        // parse_find_output — exit-code → outcome mapping.

        #[test]
        fn test_parse_find_output_success_trims_trailing_newline() {
            let got = parse_find_output(true, 0, b"blob-bytes\n", b"").unwrap();
            assert_eq!(got.as_deref(), Some("blob-bytes"));
        }

        #[test]
        fn test_parse_find_output_exit_44_is_clean_miss() {
            // errSecItemNotFound — the ONLY code allowed to read as
            // "not there"; Auto-mode file fallback keys off this.
            let got = parse_find_output(false, 44, b"", b"could not be found").unwrap();
            assert!(got.is_none());
        }

        #[test]
        fn test_parse_find_output_exit_36_is_locked() {
            let err = parse_find_output(false, 36, b"", b"").unwrap_err();
            assert!(matches!(err, KeychainErr::Locked), "err={err:?}");
        }

        #[test]
        fn test_parse_find_output_unknown_exit_fails_closed() {
            // TCC denial / ACL gate / anything unrecognized — a real
            // failure, never a clean miss.
            let err = parse_find_output(false, 51, b"", b"denied").unwrap_err();
            match err {
                KeychainErr::Other(msg) => {
                    assert!(msg.contains("code 51"), "msg={msg}");
                    assert!(msg.contains("denied"), "msg={msg}");
                }
                other => panic!("expected Other, got {other:?}"),
            }
        }

        // parse_delete_status — idempotency + fail-closed mapping.

        #[test]
        fn test_parse_delete_status_success_is_ok() {
            parse_delete_status(true, 0, b"").unwrap();
        }

        #[test]
        fn test_parse_delete_status_exit_44_is_idempotent_ok() {
            parse_delete_status(false, 44, b"not found").unwrap();
        }

        #[test]
        fn test_parse_delete_status_exit_36_is_locked() {
            let err = parse_delete_status(false, 36, b"").unwrap_err();
            assert!(matches!(err, KeychainErr::Locked), "err={err:?}");
        }

        #[test]
        fn test_parse_delete_status_unknown_exit_fails_closed() {
            let err = parse_delete_status(false, 51, b"acl gate").unwrap_err();
            match err {
                KeychainErr::Other(msg) => {
                    assert!(msg.contains("code 51"), "msg={msg}");
                    assert!(msg.contains("acl gate"), "msg={msg}");
                }
                other => panic!("expected Other, got {other:?}"),
            }
        }

        // verify_read_back — the `security -i` exit-0-lies guard.

        #[test]
        fn test_verify_read_back_matching_blob_is_ok() {
            verify_read_back(Some("blob".to_string()), "blob").unwrap();
        }

        #[test]
        fn test_verify_read_back_mismatch_is_error() {
            let err = verify_read_back(Some("other".to_string()), "blob").unwrap_err();
            match err {
                KeychainErr::Other(msg) => assert!(msg.contains("read-back mismatch")),
                other => panic!("expected Other, got {other:?}"),
            }
        }

        #[test]
        fn test_verify_read_back_absent_item_is_error() {
            let err = verify_read_back(None, "blob").unwrap_err();
            match err {
                KeychainErr::Other(msg) => assert!(msg.contains("absent on read-back")),
                other => panic!("expected Other, got {other:?}"),
            }
        }

        // classify_auto_load — the fail-closed load matrix
        // (audit fix for storage.rs:289).

        #[test]
        fn test_classify_auto_load_keychain_hit_uses_blob() {
            let action = classify_auto_load(Ok(Some("blob".to_string())));
            assert!(
                matches!(action, AutoLoadAction::UseKeychain(ref b) if b == "blob"),
                "action={action:?}"
            );
        }

        #[test]
        fn test_classify_auto_load_clean_miss_falls_back_to_file() {
            let action = classify_auto_load(Ok(None));
            assert!(
                matches!(action, AutoLoadAction::FallBackToFile),
                "action={action:?}"
            );
        }

        #[test]
        fn test_classify_auto_load_locked_keychain_fails_closed() {
            // THE regression guard: a locked keychain must never read
            // as "fall back to file" — that is the attacker-forces-
            // stale-credentials path the audit fix closed.
            let action = classify_auto_load(Err(KeychainErr::Locked));
            assert!(
                matches!(action, AutoLoadAction::FailClosed(KeychainErr::Locked)),
                "action={action:?}"
            );
        }

        #[test]
        fn test_classify_auto_load_other_error_fails_closed() {
            let action = classify_auto_load(Err(KeychainErr::Other("tcc denial".into())));
            assert!(
                matches!(action, AutoLoadAction::FailClosed(KeychainErr::Other(_))),
                "action={action:?}"
            );
        }

        // combine_delete_results — the four-arm delete matrix
        // (audit fix for storage.rs:328).

        #[test]
        fn test_combine_delete_results_both_ok_is_ok() {
            combine_delete_results(Ok(()), Ok(())).unwrap();
        }

        #[test]
        fn test_combine_delete_results_keyring_error_surfaces_despite_file_ok() {
            // The masked case the audit fix closed: keychain delete
            // hit a TCC denial (secret still in the keychain) while
            // the file path no-op-succeeded.
            let err = combine_delete_results(Err(KeychainErr::Locked), Ok(())).unwrap_err();
            assert!(matches!(err, SwapError::KeychainError(_)), "err={err:?}");
        }

        #[test]
        fn test_combine_delete_results_file_error_surfaces_despite_keyring_ok() {
            let file_err = SwapError::WriteFailed("io".into());
            let err = combine_delete_results(Ok(()), Err(file_err)).unwrap_err();
            assert!(matches!(err, SwapError::WriteFailed(_)), "err={err:?}");
        }

        #[test]
        fn test_combine_delete_results_both_errors_reports_both() {
            let err = combine_delete_results(
                Err(KeychainErr::Other("kc".into())),
                Err(SwapError::WriteFailed("fs".into())),
            )
            .unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("kc"), "msg={msg}");
            assert!(msg.contains("fs"), "msg={msg}");
        }
    }
}
