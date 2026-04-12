//! Onboarding: add a new account via `claude auth login` scaffold (Mode B).
//!
//! Uses a temp CLAUDE_CONFIG_DIR so the current active account isn't clobbered.
//! After login, imports the credential from the hashed keychain item or file.

use crate::error::OnboardError;
use std::path::PathBuf;

/// Run `claude auth login` with a temporary config dir.
/// Returns the path to the temp dir (caller is responsible for cleanup).
pub async fn run_auth_login() -> Result<PathBuf, OnboardError> {
    let temp_dir = tempfile::Builder::new()
        .prefix("claudepot-onboard-")
        .tempdir()
        .map_err(|e| OnboardError::Io(e))?;
    let config_dir = temp_dir.path().to_path_buf();

    // Find claude binary
    let claude_path = which_claude()?;

    tracing::debug!("onboarding with CLAUDE_CONFIG_DIR={}", config_dir.display());

    let status = tokio::process::Command::new(&claude_path)
        .arg("auth")
        .arg("login")
        .env("CLAUDE_CONFIG_DIR", &config_dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
        .map_err(|e| OnboardError::Io(e))?;

    if !status.success() {
        return Err(OnboardError::AuthLoginFailed(
            status.code().unwrap_or(-1),
        ));
    }

    // The temp dir must NOT be dropped here — caller reads credentials from it.
    // Leak the TempDir so it persists; caller cleans up.
    let path = temp_dir.into_path();
    Ok(path)
}

/// Read the credential blob from a temp config dir (file fallback).
pub async fn read_credentials_from_dir(config_dir: &std::path::Path) -> Result<String, OnboardError> {
    let cred_file = config_dir.join(".credentials.json");
    if cred_file.exists() {
        return std::fs::read_to_string(&cred_file)
            .map_err(OnboardError::Io);
    }

    // Try the hashed keychain item (macOS)
    #[cfg(target_os = "macos")]
    {
        let hash = crate::cli_backend::keychain::hashed_service_name(
            &config_dir.to_string_lossy(),
        );
        if let Ok(Some(blob)) = crate::cli_backend::keychain::read(&hash).await {
            return Ok(blob);
        }
    }

    Err(OnboardError::ImportFailed(
        config_dir.display().to_string(),
    ))
}

/// Clean up after onboarding: remove temp dir and hashed keychain item.
pub async fn cleanup(config_dir: &std::path::Path) {
    // Remove temp directory
    let _ = std::fs::remove_dir_all(config_dir);

    // Remove hashed keychain item (macOS)
    #[cfg(target_os = "macos")]
    {
        let hash = crate::cli_backend::keychain::hashed_service_name(
            &config_dir.to_string_lossy(),
        );
        let _ = crate::cli_backend::keychain::delete(&hash).await;
    }
}

fn which_claude() -> Result<PathBuf, OnboardError> {
    // Check common locations
    let candidates = [
        dirs::home_dir().map(|h| h.join(".local/bin/claude")),
        Some(PathBuf::from("/usr/local/bin/claude")),
        Some(PathBuf::from("/usr/bin/claude")),
    ];
    for candidate in candidates.iter().flatten() {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    // Try PATH
    if let Ok(output) = std::process::Command::new("which").arg("claude").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    Err(OnboardError::CliBinaryNotFound("claude not found in PATH or common locations".into()))
}
