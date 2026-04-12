//! macOS Keychain operations via `/usr/bin/security` subprocess.
//!
//! This module interacts with CC's `Claude Code-credentials` Keychain
//! item by spawning `/usr/bin/security` — never by calling `SecItem*`
//! directly. See reference.md §I.6 for why this is non-negotiable.

use crate::error::SwapError;
use std::time::Duration;
use tokio::process::Command;
use tokio::io::AsyncWriteExt;

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
            .args([
                "find-generic-password",
                "-a", &user,
                "-s", service,
                "-w",
            ])
            .output()
            .await
    })
    .await
    .map_err(|_| SwapError::KeychainError("security read timed out".into()))?
    .map_err(|e| SwapError::KeychainError(format!("security spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not be found") {
            return Ok(None);
        }
        return Err(SwapError::KeychainError(format!(
            "security find-generic-password failed: {stderr}"
        )));
    }

    let blob = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(Some(blob))
}

/// Write a credential blob to the Keychain via `security -i` stdin mode.
/// Uses hex-encoded payload to avoid shell escaping issues.
/// See reference.md §I.6 for the exact protocol.
pub async fn write(service: &str, blob: &str) -> Result<(), SwapError> {
    let user = std::env::var("USER").unwrap_or_else(|_| whoami::username());
    let hex_value = hex::encode(blob.as_bytes());
    let command_line = format!(
        "add-generic-password -U -a \"{user}\" -s \"{service}\" -X \"{hex_value}\"\n"
    );

    let output = tokio::time::timeout(TIMEOUT, async {
        let mut child = Command::new(SECURITY_BIN)
            .args(["-i"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SwapError::KeychainError(format!("security spawn failed: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(command_line.as_bytes()).await
                .map_err(|e| SwapError::KeychainError(format!("stdin write failed: {e}")))?;
            drop(stdin);
        }

        child.wait_with_output().await
            .map_err(|e| SwapError::KeychainError(format!("security wait failed: {e}")))
    })
    .await
    .map_err(|_| SwapError::KeychainError("security write timed out".into()))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SwapError::KeychainError(format!(
            "security add-generic-password failed: {stderr}"
        )));
    }

    Ok(())
}

/// Delete a Keychain item via `security delete-generic-password`.
pub async fn delete(service: &str) -> Result<(), SwapError> {
    let user = std::env::var("USER").unwrap_or_else(|_| whoami::username());
    let output = tokio::time::timeout(TIMEOUT, async {
        Command::new(SECURITY_BIN)
            .args([
                "delete-generic-password",
                "-a", &user,
                "-s", service,
            ])
            .output()
            .await
    })
    .await
    .map_err(|_| SwapError::KeychainError("security delete timed out".into()))?
    .map_err(|e| SwapError::KeychainError(format!("security spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("could not be found") {
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
                .map_err(|e| SwapError::FileError(e))?;
        }
        Ok(())
    }
}
