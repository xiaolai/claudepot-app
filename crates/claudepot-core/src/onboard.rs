//! Onboarding: add a new account via `claude auth login` scaffold (Mode B).
//!
//! Uses a temp CLAUDE_CONFIG_DIR so the current active account isn't clobbered.
//! After login, imports the credential from the hashed keychain item or file.

use crate::error::OnboardError;
use std::path::PathBuf;

/// Hard timeout for `claude auth login` — generous enough that slow
/// readers completing OAuth in the browser finish in time, tight enough
/// that a user who closed the browser or walked away doesn't leave the
/// GUI stuck on a spinner forever. Matches Kannon's 10-minute window.
pub const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Cancellable variant: pass a shared `Notify`; when another task calls
/// `notify.notify_one()`, the subprocess is killed and this function
/// returns `AuthLoginCancelled`. Used by the GUI's Cancel button.
///
/// Error cases:
/// - `AuthLoginCancelled` — user clicked Cancel
/// - `AuthLoginFailed(-2)` — hit LOGIN_TIMEOUT
/// - `AuthLoginFailed(code)` — subprocess exited with failure
pub async fn run_auth_login_in_place_cancellable(
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), OnboardError> {
    let claude_path = which_claude()?;

    tracing::info!(
        binary = %claude_path.display(),
        timeout_secs = LOGIN_TIMEOUT.as_secs(),
        "spawning `claude auth login` in place"
    );

    let mut child = tokio::process::Command::new(&claude_path)
        .arg("auth")
        .arg("login")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(OnboardError::Io)?;

    // Drain stdout / stderr into tracing so logs from the child surface
    // when the GUI is launched without a terminal. Tasks are aborted
    // automatically when the parent `child` is dropped.
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pipe_to_tracing(stdout, "claude-stdout"));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(pipe_to_tracing(stderr, "claude-stderr"));
    }

    let cancel_fut = async {
        match cancel.as_ref() {
            Some(n) => n.notified().await,
            // Never-resolving future when no cancel channel was provided.
            None => std::future::pending::<()>().await,
        }
    };

    tokio::select! {
        exit = child.wait() => {
            match exit {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(OnboardError::AuthLoginFailed(status.code().unwrap_or(-1))),
                Err(e) => Err(OnboardError::Io(e)),
            }
        }
        _ = tokio::time::sleep(LOGIN_TIMEOUT) => {
            tracing::warn!(
                "`claude auth login` exceeded {}s — killing child",
                LOGIN_TIMEOUT.as_secs()
            );
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginFailed(-2))
        }
        _ = cancel_fut => {
            tracing::info!("login cancelled by user — killing child");
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginCancelled)
        }
    }
}

async fn pipe_to_tracing<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    stream_name: &'static str,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::info!(target: "claudepot::onboard", stream = stream_name, "{}", line);
    }
}

/// Run `claude auth login` with a temporary config dir.
/// Returns the path to the temp dir (caller is responsible for cleanup).
pub async fn run_auth_login() -> Result<PathBuf, OnboardError> {
    let temp_dir = tempfile::Builder::new()
        .prefix("claudepot-onboard-")
        .tempdir()
        .map_err(OnboardError::Io)?;
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
        .map_err(OnboardError::Io)?;

    if !status.success() {
        return Err(OnboardError::AuthLoginFailed(status.code().unwrap_or(-1)));
    }

    // The temp dir must NOT be dropped here — caller reads credentials from it.
    // Leak the TempDir so it persists; caller cleans up.
    let path = temp_dir.keep();
    Ok(path)
}

/// Read the credential blob from a temp config dir (file fallback).
pub async fn read_credentials_from_dir(
    config_dir: &std::path::Path,
) -> Result<String, OnboardError> {
    let cred_file = config_dir.join(".credentials.json");
    if cred_file.exists() {
        return std::fs::read_to_string(&cred_file).map_err(OnboardError::Io);
    }

    // Try the hashed keychain item (macOS)
    #[cfg(target_os = "macos")]
    {
        let hash = crate::cli_backend::keychain::hashed_service_name(&config_dir.to_string_lossy());
        if let Ok(Some(blob)) = crate::cli_backend::keychain::read(&hash).await {
            return Ok(blob);
        }
    }

    Err(OnboardError::ImportFailed(config_dir.display().to_string()))
}

/// Clean up after onboarding: remove temp dir and hashed keychain item.
pub async fn cleanup(config_dir: &std::path::Path) {
    // Remove temp directory
    let _ = std::fs::remove_dir_all(config_dir);

    // Remove hashed keychain item (macOS)
    #[cfg(target_os = "macos")]
    {
        let hash = crate::cli_backend::keychain::hashed_service_name(&config_dir.to_string_lossy());
        let _ = crate::cli_backend::keychain::delete(&hash).await;
    }
}

fn which_claude() -> Result<PathBuf, OnboardError> {
    crate::fs_utils::find_claude_binary().ok_or_else(|| {
        OnboardError::CliBinaryNotFound("claude not found in PATH or common locations".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The -2 sentinel for "timed out" must render a clear, actionable
    /// message — the GUI shows this verbatim in a toast, so it needs to
    /// guide the user instead of displaying a cryptic exit code.
    #[test]
    fn test_auth_login_timeout_error_message() {
        let err = OnboardError::AuthLoginFailed(-2);
        let msg = err.to_string();
        assert!(
            msg.contains("timed out"),
            "timeout message should say 'timed out'; got: {msg}"
        );
        assert!(
            msg.contains("try again"),
            "timeout message should include a recovery hint; got: {msg}"
        );
    }

    #[test]
    fn test_auth_login_non_timeout_exit_code_is_reported() {
        // Any non-(-2) exit code should include the actual code so the
        // user can diagnose (e.g. 1 = generic CC failure).
        let err = OnboardError::AuthLoginFailed(1);
        assert!(err.to_string().contains("1"));
    }
}
