//! macOS Keychain operations via `/usr/bin/security` subprocess.
//!
//! This module interacts with CC's `Claude Code-credentials` Keychain
//! item by spawning `/usr/bin/security` — never by calling `SecItem*`
//! directly. See reference.md §I.6 for why this is non-negotiable.

use crate::error::SwapError;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const SECURITY_BIN: &str = "/usr/bin/security";
const TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_SERVICE: &str = "Claude Code-credentials";

/// Compute the hashed Keychain service name for a given config dir.
/// Matches CC's `getMacOsKeychainStorageServiceName()`.
pub fn hashed_service_name(config_dir: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(config_dir.as_bytes());
    let digest = h.finalize();
    let prefix = hex::encode(&digest[..4]);
    format!("Claude Code-credentials-{prefix}")
}

/// Read a credential blob from the Keychain via `security find-generic-password -w`.
pub async fn read(service: &str) -> Result<Option<String>, SwapError> {
    let user = std::env::var("USER").unwrap_or_else(|_| whoami::username());
    let output = tokio::time::timeout(TIMEOUT, async {
        Command::new(SECURITY_BIN)
            .args(["find-generic-password", "-a", &user, "-s", service, "-w"])
            .output()
            .await
    })
    .await
    .map_err(|_| SwapError::KeychainError("security read timed out".into()))?
    .map_err(|e| SwapError::KeychainError(format!("security spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Audit Low: accept both the English stderr match AND exit
        // code 44 (errSecItemNotFound). The stderr-text check was
        // fragile on localized or version-changed macOS output; the
        // numeric exit code is stable across locales.
        if output.status.code() == Some(44) || stderr.contains("could not be found") {
            return Ok(None);
        }
        // Exit 36 = errSecAuthFailed — on macOS this is nearly always "the
        // login keychain is locked". It's distinguishable from "item not
        // found" (exit 44) and worth reporting verbatim so the UI can
        // prompt the user to unlock.
        if output.status.code() == Some(36) {
            return Err(SwapError::KeychainError(
                "macOS login keychain is locked — open Keychain Access and \
                 unlock the \"login\" keychain, then retry"
                    .into(),
            ));
        }
        return Err(SwapError::KeychainError(format!(
            "security find-generic-password failed: {stderr}"
        )));
    }

    let blob = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(Some(blob))
}

/// Short, non-reversible fingerprint of a credential blob, suitable for
/// log correlation. SHA-256, truncated to 8 hex chars (32 bits). NEVER
/// returns blob content — pre-images cannot be recovered from this value.
/// 32 bits is small enough that it cannot meaningfully identify the blob
/// to an attacker reading logs but distinct enough to tell two blobs apart
/// in the same trace.
fn blob_digest(blob: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(blob.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..4])
}

/// Validate that a string is safe to interpolate in a `security -i` command.
/// Rejects values containing quotes, newlines, or backslashes that could
/// alter the parsed command semantics.
fn validate_security_input(value: &str, label: &str) -> Result<(), SwapError> {
    if value.contains('"') || value.contains('\n') || value.contains('\r') || value.contains('\\') {
        return Err(SwapError::KeychainError(format!(
            "{label} contains unsafe characters for keychain command"
        )));
    }
    Ok(())
}

/// Write a credential blob to the Keychain via `security -i` stdin mode.
/// Uses hex-encoded payload to avoid shell escaping issues.
/// See reference.md §I.6 for the exact protocol.
pub async fn write(service: &str, blob: &str) -> Result<(), SwapError> {
    let user = std::env::var("USER").unwrap_or_else(|_| whoami::username());
    validate_security_input(&user, "USER")?;
    validate_security_input(service, "service")?;
    let hex_value = hex::encode(blob.as_bytes());
    tracing::info!(
        target: "claudepot::keychain_write",
        service = %service,
        user = %user,
        blob_len = blob.len(),
        blob_digest = %blob_digest(blob),
        "writing to CC keychain via `security -i add-generic-password -U`"
    );
    let command_line =
        format!("add-generic-password -U -a \"{user}\" -s \"{service}\" -X \"{hex_value}\"\n");

    let output = tokio::time::timeout(TIMEOUT, async {
        let mut child = Command::new(SECURITY_BIN)
            .args(["-i"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SwapError::KeychainError(format!("security spawn failed: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(command_line.as_bytes())
                .await
                .map_err(|e| SwapError::KeychainError(format!("stdin write failed: {e}")))?;
            drop(stdin);
        }

        child
            .wait_with_output()
            .await
            .map_err(|e| SwapError::KeychainError(format!("security wait failed: {e}")))
    })
    .await
    .map_err(|_| SwapError::KeychainError("security write timed out".into()))??;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::info!(
        target: "claudepot::keychain_write",
        exit_code = output.status.code().unwrap_or(-1),
        stderr = %stderr.trim(),
        stdout = %stdout.trim(),
        "security -i returned"
    );
    if !output.status.success() {
        return Err(SwapError::KeychainError(format!(
            "security add-generic-password failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }
    // Exit-zero is not always sufficient — `security -i` can silently accept
    // a command and return 0 while the inner command fails (notably when
    // TCC or an ACL gate kicks in for the keychain write). Read back to
    // verify our blob is actually what's in the keychain.
    match read(service).await {
        Ok(Some(stored)) if stored == blob => {
            tracing::info!(target: "claudepot::keychain_write", "readback verified");
        }
        Ok(Some(stored)) => {
            tracing::error!(
                target: "claudepot::keychain_write",
                expected_len = blob.len(),
                actual_len = stored.len(),
                expected_digest = %blob_digest(blob),
                actual_digest = %blob_digest(&stored),
                "readback MISMATCH — the write silently didn't stick"
            );
            return Err(SwapError::KeychainError(
                "write to CC keychain did not take effect — verify the login \
                 keychain is unlocked and Claudepot is allowed to modify the \
                 'Claude Code-credentials' item in Keychain Access"
                    .into(),
            ));
        }
        Ok(None) => {
            tracing::error!(target: "claudepot::keychain_write", "readback found no item");
            return Err(SwapError::KeychainError(
                "wrote to CC keychain but the item disappeared on readback".into(),
            ));
        }
        Err(e) => {
            tracing::warn!(
                target: "claudepot::keychain_write",
                error = %e,
                "readback errored — assuming success on exit-zero"
            );
        }
    }
    Ok(())
}

/// Delete a Keychain item via `security delete-generic-password`.
pub async fn delete(service: &str) -> Result<(), SwapError> {
    let user = std::env::var("USER").unwrap_or_else(|_| whoami::username());
    let output = tokio::time::timeout(TIMEOUT, async {
        Command::new(SECURITY_BIN)
            .args(["delete-generic-password", "-a", &user, "-s", service])
            .output()
            .await
    })
    .await
    .map_err(|_| SwapError::KeychainError("security delete timed out".into()))?
    .map_err(|e| SwapError::KeychainError(format!("security spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Exit 44 = errSecItemNotFound. Also accept the English
        // stderr phrase for backward compat on older macOS without
        // the numeric exit conventions.
        let not_found = output.status.code() == Some(44) || stderr.contains("could not be found");
        if !not_found {
            return Err(SwapError::KeychainError(format!(
                "security delete-generic-password failed: {stderr}"
            )));
        }
    }

    Ok(())
}

/// Read the default `Claude Code-credentials` item.
pub async fn read_default() -> Result<Option<String>, SwapError> {
    read(DEFAULT_SERVICE).await
}

/// Write to the default `Claude Code-credentials` item.
pub async fn write_default(blob: &str) -> Result<(), SwapError> {
    write(DEFAULT_SERVICE, blob).await
}

/// The macOS CliPlatform implementation.
pub struct MacosKeychain;

#[async_trait::async_trait]
impl super::CliPlatform for MacosKeychain {
    async fn read_default(&self) -> Result<Option<String>, SwapError> {
        read(DEFAULT_SERVICE).await
    }

    async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
        write(DEFAULT_SERVICE, blob).await
    }

    async fn touch_credfile(&self) -> Result<(), SwapError> {
        let path = crate::paths::claude_credentials_file();
        if path.exists() {
            filetime::set_file_mtime(&path, filetime::FileTime::now())
                .map_err(SwapError::FileError)?;
        }
        Ok(())
    }

    async fn clear_default(&self) -> Result<(), SwapError> {
        // `security delete-generic-password -s "Claude Code-credentials"`.
        // Treat "item doesn't exist" as success so the rollback path is
        // idempotent regardless of what state CC was in.
        use tokio::process::Command;
        let out = Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-s", DEFAULT_SERVICE])
            .output()
            .await
            .map_err(|e| SwapError::WriteFailed(format!("security spawn: {e}")))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Exit 44 / "SecKeychainSearchCopyNext: The specified item could
        // not be found in the keychain." → nothing to delete, which is
        // the intended post-condition.
        if stderr.contains("could not be found") || out.status.code() == Some(44) {
            return Ok(());
        }
        Err(SwapError::WriteFailed(format!(
            "security delete-generic-password: {}",
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hashed_service_name_deterministic() {
        let a = hashed_service_name("/Users/joker/.claude");
        let b = hashed_service_name("/Users/joker/.claude");
        assert_eq!(a, b);
    }

    #[test]
    fn test_hashed_service_name_format() {
        let result = hashed_service_name("/Users/joker/.claude");
        assert!(result.starts_with("Claude Code-credentials-"));
        // SHA-256 first 4 bytes = 8 hex chars
        let suffix = result.strip_prefix("Claude Code-credentials-").unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hashed_service_name_different_paths() {
        let a = hashed_service_name("/Users/alice/.claude");
        let b = hashed_service_name("/Users/bob/.claude");
        assert_ne!(a, b);
    }

    #[test]
    fn test_validate_security_input_rejects_quotes() {
        assert!(validate_security_input("normal", "test").is_ok());
        assert!(validate_security_input("has\"quote", "test").is_err());
        assert!(validate_security_input("has\nnewline", "test").is_err());
        assert!(validate_security_input("has\\backslash", "test").is_err());
        assert!(validate_security_input("has\rreturn", "test").is_err());
    }

    #[test]
    fn test_default_service_name() {
        assert_eq!(DEFAULT_SERVICE, "Claude Code-credentials");
    }
}
