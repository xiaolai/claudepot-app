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
    run_auth_login_in_place_cancellable_with_binary(&claude_path, cancel).await
}

/// Internal seam: accepts an explicit binary path so tests can point
/// the loop at a controllable stub process. Mirrors
/// [`run_auth_login_cancellable_with_binary`] — kept private to the
/// crate so the public surface stays tied to the auto-discovered
/// `claude` binary.
pub(crate) async fn run_auth_login_in_place_cancellable_with_binary(
    claude_path: &std::path::Path,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), OnboardError> {
    tracing::info!(
        binary = %claude_path.display(),
        timeout_secs = LOGIN_TIMEOUT.as_secs(),
        "spawning `claude auth login` in place"
    );

    let mut child = tokio::process::Command::new(claude_path)
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
        // `claude auth login` can occasionally echo OAuth artifacts into
        // its diagnostic output. Run every line through the same
        // `sk-ant-*` masker the rest of the codebase uses before it
        // reaches `tracing` — otherwise any sink (file, syslog, console)
        // becomes a credential disclosure surface.
        // See `.claude/rules/rust-conventions.md` §Security.
        let safe = crate::session_export::redact_secrets(&line);
        tracing::info!(target: "claudepot::onboard", stream = stream_name, "{}", safe);
    }
}

/// Run `claude auth login` with a temporary config dir.
/// Returns the path to the temp dir (caller is responsible for cleanup).
///
/// Non-cancellable — prefer `run_auth_login_cancellable` when a user
/// might close the browser mid-flow.
pub async fn run_auth_login() -> Result<PathBuf, OnboardError> {
    run_auth_login_cancellable(None).await
}

/// Cancellable temp-dir variant of `run_auth_login`.
///
/// Pass a shared `Notify`; when another task calls `notify.notify_one()`,
/// the child `claude auth login` process is killed and this function
/// returns `AuthLoginCancelled`. The temp config dir is returned either
/// way so the caller can inspect it (credential read) or clean it up
/// (cancelled / failed).
///
/// Error cases mirror `run_auth_login_in_place_cancellable`:
/// * `AuthLoginCancelled` — user clicked Cancel
/// * `AuthLoginFailed(-2)` — hit `LOGIN_TIMEOUT`
/// * `AuthLoginFailed(code)` — subprocess exited with failure
pub async fn run_auth_login_cancellable(
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<PathBuf, OnboardError> {
    let claude_path = which_claude()?;
    run_auth_login_cancellable_with_binary(&claude_path, cancel).await
}

/// Internal seam: accepts an explicit binary path so tests can point
/// the loop at a controllable stub process. Not re-exported.
pub(crate) async fn run_auth_login_cancellable_with_binary(
    claude_path: &std::path::Path,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<PathBuf, OnboardError> {
    let temp_dir = tempfile::Builder::new()
        .prefix("claudepot-onboard-")
        .tempdir()
        .map_err(OnboardError::Io)?;
    let config_dir = temp_dir.path().to_path_buf();

    tracing::info!(
        binary = %claude_path.display(),
        config_dir = %config_dir.display(),
        timeout_secs = LOGIN_TIMEOUT.as_secs(),
        "spawning `claude auth login` in temp dir"
    );

    let mut child = tokio::process::Command::new(claude_path)
        .arg("auth")
        .arg("login")
        .env("CLAUDE_CONFIG_DIR", &config_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(OnboardError::Io)?;

    // Mirror run_auth_login_in_place_cancellable's drain-to-tracing
    // pattern. Stdout/stderr piped so a closed terminal doesn't lose
    // diagnostic output.
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pipe_to_tracing(stdout, "claude-stdout"));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(pipe_to_tracing(stderr, "claude-stderr"));
    }

    let cancel_fut = async {
        match cancel.as_ref() {
            Some(n) => n.notified().await,
            None => std::future::pending::<()>().await,
        }
    };

    let outcome = tokio::select! {
        exit = child.wait() => {
            match exit {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(OnboardError::AuthLoginFailed(
                    status.code().unwrap_or(-1),
                )),
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
            tracing::info!("browser login cancelled by user — killing child");
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginCancelled)
        }
    };

    match outcome {
        Ok(()) => {
            // Keep the tempdir alive for the caller; they clean it up
            // after reading credentials.
            Ok(temp_dir.keep())
        }
        Err(e) => {
            // `claude auth login` can write credentials to two places:
            // the temp `.credentials.json` file *and*, on macOS, a
            // hashed Keychain entry keyed off the temp dir's path. If
            // OAuth completed in the browser milliseconds before the
            // user clicked Cancel (or the timeout elapsed), the child
            // may have already written into both. `TempDir::drop`
            // only removes the directory; the Keychain item would be
            // orphaned. Route cleanup through `cleanup()` which knows
            // about both surfaces.
            let config_dir = temp_dir.path().to_path_buf();
            // Release the TempDir handle first so its Drop doesn't
            // race with the explicit `remove_dir_all` inside cleanup.
            let _ = temp_dir.keep();
            cleanup(&config_dir).await;
            Err(e)
        }
    }
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

    /// Smoke test for the cancel path. The real `claude` binary isn't
    /// reliably installed in CI, so we stand in with a tiny shell
    /// stub that `exec`s into `sleep 30`. The stub ignores the
    /// `auth login` argv injected by the runner — it just needs to
    /// stay alive long enough for the test to fire the Notify. The
    /// test asserts that cancel resolves quickly with
    /// `AuthLoginCancelled` and that the temp dir no longer exists
    /// after the error-path cleanup.
    #[tokio::test]
    #[cfg(unix)]
    async fn cancel_kills_child_and_cleans_tempdir() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Notify;

        // Write a stub binary to a tempdir and make it executable.
        let stub_dir = tempfile::tempdir().expect("mk stub tempdir");
        let stub = stub_dir.path().join("claude-stub.sh");
        std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").expect("write stub");
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();

        // Snapshot any pre-existing `claudepot-onboard-*` directories
        // so stale state (from previous failed runs or parallel tests)
        // doesn't confuse the post-cleanup assertion below.
        let temp_root = std::env::temp_dir();
        let before: std::collections::HashSet<std::ffi::OsString> = std::fs::read_dir(&temp_root)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .filter(|n| n.to_string_lossy().starts_with("claudepot-onboard-"))
                    .collect()
            })
            .unwrap_or_default();

        let notify = Arc::new(Notify::new());
        let notify_clone = notify.clone();
        let stub_path = stub.clone();

        let task = tokio::spawn(async move {
            run_auth_login_cancellable_with_binary(&stub_path, Some(notify_clone)).await
        });

        // Let the child actually spawn before we fire the Notify so
        // the tokio::select! is parked on child.wait() when the
        // cancel arm resolves.
        tokio::time::sleep(Duration::from_millis(150)).await;
        notify.notify_one();

        let outcome = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancel should complete within 5s")
            .expect("join handle should not panic");
        assert!(
            matches!(outcome, Err(OnboardError::AuthLoginCancelled)),
            "expected AuthLoginCancelled, got {outcome:?}"
        );

        // New tempdirs created by *this* test run must all be gone
        // after the awaited cleanup. Tempdirs from prior runs /
        // parallel tests are filtered out via the `before` snapshot.
        let after: std::collections::HashSet<std::ffi::OsString> = std::fs::read_dir(&temp_root)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .filter(|n| n.to_string_lossy().starts_with("claudepot-onboard-"))
                    .collect()
            })
            .unwrap_or_default();
        let new_leftovers: Vec<_> = after.difference(&before).collect();
        assert!(
            new_leftovers.is_empty(),
            "cleanup should remove onboarding tempdirs created by this test run, still have: {new_leftovers:?}"
        );
    }
}
