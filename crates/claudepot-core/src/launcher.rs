//! Mode D — env-var inject launcher.
//!
//! Spawns a child process with `CLAUDE_CODE_OAUTH_TOKEN` set from the
//! account's stored credential. Zero disk state mutation.

use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::error::LauncherError;
use crate::oauth::refresh;

use uuid::Uuid;

/// Get a fresh access token for an account, refreshing if expired.
pub async fn get_access_token(account_id: Uuid) -> Result<String, LauncherError> {
    let blob_str = swap::load_private(account_id)
        .map_err(|_| LauncherError::NoStoredCredentials(account_id))?;
    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| LauncherError::CorruptBlob(e.to_string()))?;

    // If token has >5 minutes remaining, use it directly
    if !blob.is_expired(300) {
        return Ok(blob.claude_ai_oauth.access_token.clone());
    }

    // Refresh needed
    tracing::debug!("access token expired/expiring, refreshing...");
    let token_resp = refresh::refresh(&blob.claude_ai_oauth.refresh_token)
        .await
        .map_err(|e| LauncherError::RefreshFailed(e.to_string()))?;

    // Save the rotated credentials, preserving original subscription metadata
    let new_blob_str = refresh::build_blob(&token_resp, Some(&blob));
    swap::save_private(account_id, &new_blob_str)
        .map_err(|e| LauncherError::SaveFailed(e.to_string()))?;

    Ok(token_resp.access_token)
}

/// Spawn a child process with CLAUDE_CODE_OAUTH_TOKEN injected.
/// Returns the child's exit code.
pub async fn run(account_id: Uuid, args: &[String]) -> Result<i32, LauncherError> {
    // Audit Low: validate args BEFORE touching credentials. Previously
    // this fetched + possibly refreshed the token first, then
    // discovered args were empty — wasteful I/O and the error was
    // NoStoredCredentials instead of the more accurate NoCommand.
    if args.is_empty() {
        return Err(LauncherError::NoCommand);
    }

    let access_token = get_access_token(account_id).await?;

    let (cmd, cmd_args) = args.split_first().ok_or(LauncherError::NoCommand)?;

    let status = tokio::process::Command::new(cmd)
        .args(cmd_args)
        .env("CLAUDE_CODE_OAUTH_TOKEN", &access_token)
        .env("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB", "1")
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
        .map_err(|e| LauncherError::SpawnFailed(e.to_string()))?;

    Ok(status.code().unwrap_or(1))
}

// Tests serialize through `lock_data_dir()` (a `Mutex<()>`) so they
// don't trample the shared `CLAUDEPOT_DATA_DIR` env var. The
// MutexGuard is intentionally held across `.await` for the lifetime
// of each test, which `clippy::await_holding_lock` flags. The lock
// is single-threaded, never poisoned, and never contended in a way
// that could deadlock — silence it at the module boundary.
#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::testing::{fresh_blob_json, lock_data_dir, setup_test_data_dir};

    #[tokio::test]
    async fn test_get_access_token_fresh_returns_directly() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        swap::save_private(id, &fresh_blob_json()).unwrap();

        let token = get_access_token(id).await.unwrap();
        assert_eq!(token, "sk-ant-oat01-test");

        swap::delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_get_access_token_missing_credentials() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        let result = get_access_token(id).await;
        assert!(matches!(result, Err(LauncherError::NoStoredCredentials(_))));
    }

    #[tokio::test]
    async fn test_get_access_token_corrupt_blob() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        swap::save_private(id, "not json").unwrap();

        let result = get_access_token(id).await;
        assert!(matches!(result, Err(LauncherError::CorruptBlob(_))));

        swap::delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_run_empty_args_returns_no_command() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).unwrap();

        let result = run(id, &[]).await;
        assert!(matches!(result, Err(LauncherError::NoCommand)));

        swap::delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_run_executes_command() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).unwrap();

        // Cross-platform: `echo` is a cmd.exe builtin on Windows (no .exe),
        // but `cmd /c exit 0` always works. On Unix, prefer `true`.
        #[cfg(windows)]
        let args = vec!["cmd".to_string(), "/c".to_string(), "exit 0".to_string()];
        #[cfg(not(windows))]
        let args = vec!["echo".to_string(), "hello".to_string()];

        let exit_code = run(id, &args).await.unwrap();
        assert_eq!(exit_code, 0);

        swap::delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_run_nonexistent_command_returns_spawn_failed() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).unwrap();

        let args = vec!["/nonexistent/binary/that/doesnt/exist".to_string()];
        let result = run(id, &args).await;
        assert!(matches!(result, Err(LauncherError::SpawnFailed(_))));

        swap::delete_private(id).unwrap();
    }
}
