//! Mode D — env-var inject launcher.
//!
//! Spawns a child process with `CLAUDE_CODE_OAUTH_TOKEN` set from the
//! account's stored credential. Zero disk state mutation.

use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::oauth::refresh;

use uuid::Uuid;

/// Boundary error for the launcher. Historically lived in the
/// crate-root `error.rs`; relocated next to its boundary per
/// rust-conventions ("one enum per module boundary").
/// `crate::error::LauncherError` remains a re-export.
#[derive(thiserror::Error, Debug)]
pub enum LauncherError {
    #[error("no stored credentials for account {0}")]
    NoStoredCredentials(uuid::Uuid),

    #[error("corrupt credential blob: {0}")]
    CorruptBlob(String),

    #[error("token refresh failed: {0}")]
    RefreshFailed(String),

    #[error("failed to save refreshed credentials: {0}")]
    SaveFailed(String),

    #[error("no command specified")]
    NoCommand,

    #[error("spawn failed: {0}")]
    SpawnFailed(String),
}

/// Get a fresh access token for an account, refreshing if expired.
///
/// `is_active` marks the account CC is currently signed in as. That one
/// account has a second live copy of its token family sitting in CC's
/// own slot, and Anthropic invalidates the old refresh_token on every
/// rotation — so refreshing the private copy would leave CC holding a
/// dead token and force the user back through `/login`. For the active
/// account we refresh CC's slot in place instead (see
/// [`swap::fresh_token_from_cc_slot`]).
pub async fn get_access_token(account_id: Uuid, is_active: bool) -> Result<String, LauncherError> {
    let platform = crate::cli_backend::create_platform();
    get_access_token_with(
        account_id,
        is_active,
        platform.as_ref(),
        &swap::DefaultRefresher,
    )
    .await
}

/// Testable variant: inject the [`CliPlatform`] and [`TokenRefresher`].
pub(crate) async fn get_access_token_with(
    account_id: Uuid,
    is_active: bool,
    platform: &dyn crate::cli_backend::CliPlatform,
    refresher: &dyn swap::TokenRefresher,
) -> Result<String, LauncherError> {
    if is_active {
        match swap::fresh_token_from_cc_slot(account_id, platform, refresher).await {
            Ok(Some(token)) => return Ok(token),
            // CC's slot is empty or unparseable — no competing copy to
            // strand, so the private slot below is safe to use.
            Ok(None) => {}
            Err(e) => return Err(LauncherError::RefreshFailed(e.to_string())),
        }
    }

    let blob_str = swap::load_private(account_id)
        .await
        .map_err(|_| LauncherError::NoStoredCredentials(account_id))?;
    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| LauncherError::CorruptBlob(e.to_string()))?;

    // If token has >5 minutes remaining, use it directly
    if !blob.is_expired(300) {
        return Ok(blob.claude_ai_oauth.access_token.clone());
    }

    // Refresh needed
    tracing::debug!("access token expired/expiring, refreshing...");
    let token_resp = refresher
        .refresh(&blob.claude_ai_oauth.refresh_token)
        .await
        .map_err(|e| LauncherError::RefreshFailed(e.to_string()))?;

    // Save the rotated credentials, preserving original subscription metadata
    let new_blob_str = refresh::build_blob(&token_resp, Some(&blob));
    swap::save_private(account_id, &new_blob_str)
        .await
        .map_err(|e| LauncherError::SaveFailed(e.to_string()))?;

    Ok(token_resp.access_token)
}

/// Spawn a child process with CLAUDE_CODE_OAUTH_TOKEN injected.
/// Returns the child's exit code.
pub async fn run(account_id: Uuid, args: &[String], is_active: bool) -> Result<i32, LauncherError> {
    // Audit Low: validate args BEFORE touching credentials. Previously
    // this fetched + possibly refreshed the token first, then
    // discovered args were empty — wasteful I/O and the error was
    // NoStoredCredentials instead of the more accurate NoCommand.
    if args.is_empty() {
        return Err(LauncherError::NoCommand);
    }

    let access_token = get_access_token(account_id, is_active).await?;

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

        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let token = get_access_token(id, false).await.unwrap();
        assert_eq!(token, "sk-ant-oat01-test");

        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_get_access_token_missing_credentials() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        let result = get_access_token(id, false).await;
        assert!(matches!(result, Err(LauncherError::NoStoredCredentials(_))));
    }

    #[tokio::test]
    async fn test_get_access_token_corrupt_blob() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        swap::save_private(id, "not json").await.unwrap();

        let result = get_access_token(id, false).await;
        assert!(matches!(result, Err(LauncherError::CorruptBlob(_))));

        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_run_empty_args_returns_no_command() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let result = run(id, &[], false).await;
        assert!(matches!(result, Err(LauncherError::NoCommand)));

        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_run_executes_command() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        // Cross-platform: `echo` is a cmd.exe builtin on Windows (no .exe),
        // but `cmd /c exit 0` always works. On Unix, prefer `true`.
        #[cfg(windows)]
        let args = vec!["cmd".to_string(), "/c".to_string(), "exit 0".to_string()];
        #[cfg(not(windows))]
        let args = vec!["echo".to_string(), "hello".to_string()];

        let exit_code = run(id, &args, false).await.unwrap();
        assert_eq!(exit_code, 0);

        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_run_nonexistent_command_returns_spawn_failed() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let args = vec!["/nonexistent/binary/that/doesnt/exist".to_string()];
        let result = run(id, &args, false).await;
        assert!(matches!(result, Err(LauncherError::SpawnFailed(_))));

        swap::delete_private(id).await.unwrap();
    }

    // -- Active account: CC's slot owns the live token family --

    /// Stand-in for CC's credential slot.
    struct MockPlatform {
        blob: std::sync::Mutex<Option<String>>,
        writes: std::sync::Mutex<u32>,
    }

    impl MockPlatform {
        fn holding(blob: Option<&str>) -> Self {
            Self {
                blob: std::sync::Mutex::new(blob.map(String::from)),
                writes: std::sync::Mutex::new(0),
            }
        }
        fn get(&self) -> Option<String> {
            self.blob.lock().unwrap().clone()
        }
        fn writes(&self) -> u32 {
            *self.writes.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl crate::cli_backend::CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, crate::error::SwapError> {
            Ok(self.blob.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), crate::error::SwapError> {
            *self.blob.lock().unwrap() = Some(blob.to_string());
            *self.writes.lock().unwrap() += 1;
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), crate::error::SwapError> {
            Ok(())
        }
    }

    struct CountingRefresher {
        calls: std::sync::Mutex<u32>,
    }

    impl CountingRefresher {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(0),
            }
        }
        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl swap::TokenRefresher for CountingRefresher {
        async fn refresh(
            &self,
            _refresh_token: &str,
        ) -> Result<refresh::TokenResponse, crate::error::OAuthError> {
            *self.calls.lock().unwrap() += 1;
            Ok(refresh::TokenResponse {
                access_token: "sk-ant-oat01-rotated".to_string(),
                refresh_token: "sk-ant-ort01-rotated".to_string(),
                expires_in: 3600,
                scope: None,
                token_type: None,
            })
        }
    }

    /// Refreshing the ACTIVE account must rotate CC's own slot, not just
    /// Claudepot's private copy — otherwise CC is left holding a
    /// refresh_token the server invalidated and the user is forced back
    /// through `/login`.
    #[tokio::test]
    async fn active_account_refresh_rotates_cc_slot_in_place() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        let expired = crate::testing::expired_blob_json();
        swap::save_private(id, &expired).await.unwrap();
        let platform = MockPlatform::holding(Some(&expired));
        let refresher = CountingRefresher::new();

        let token = get_access_token_with(id, true, &platform, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-rotated");
        assert_eq!(refresher.calls(), 1);

        let cc = platform.get().unwrap();
        assert!(
            cc.contains("sk-ant-ort01-rotated"),
            "CC's slot must hold the rotated refresh_token"
        );
        let private = swap::load_private(id).await.unwrap();
        assert!(
            private.contains("sk-ant-ort01-rotated"),
            "private slot must mirror the rotation"
        );

        swap::delete_private(id).await.unwrap();
    }

    /// CC refreshed on its own, so its slot is ahead of Claudepot's
    /// copy. Use CC's live token instead of spending the dead
    /// refresh_token the private slot still holds.
    #[tokio::test]
    async fn active_account_prefers_cc_slot_over_stale_private_copy() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        swap::save_private(id, &crate::testing::expired_blob_json())
            .await
            .unwrap();
        let cc_blob = fresh_blob_json().replace("oat01-test", "oat01-cc-rotated");
        let platform = MockPlatform::holding(Some(&cc_blob));
        let refresher = CountingRefresher::new();

        let token = get_access_token_with(id, true, &platform, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-cc-rotated");
        assert_eq!(refresher.calls(), 0, "CC's token is live — no exchange");
        assert_eq!(platform.writes(), 0, "nothing to write back");
        assert_eq!(
            swap::load_private(id).await.unwrap(),
            cc_blob,
            "private slot must adopt CC's blob"
        );

        swap::delete_private(id).await.unwrap();
    }

    /// A non-active account has no competing copy in CC's slot, so it
    /// keeps the private-slot path and must never touch CC's slot.
    #[tokio::test]
    async fn inactive_account_never_touches_cc_slot() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        swap::save_private(id, &crate::testing::expired_blob_json())
            .await
            .unwrap();
        let someone_else = fresh_blob_json().replace("oat01-test", "oat01-other-account");
        let platform = MockPlatform::holding(Some(&someone_else));
        let refresher = CountingRefresher::new();

        let token = get_access_token_with(id, false, &platform, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-rotated");
        assert_eq!(platform.writes(), 0);
        assert_eq!(platform.get().as_deref(), Some(someone_else.as_str()));

        swap::delete_private(id).await.unwrap();
    }

    /// Active account, but CC's slot is empty — nothing to strand, so
    /// the private-slot path still runs.
    #[tokio::test]
    async fn active_account_with_empty_cc_slot_falls_back_to_private_slot() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();

        swap::save_private(id, &fresh_blob_json()).await.unwrap();
        let platform = MockPlatform::holding(None);
        let refresher = CountingRefresher::new();

        let token = get_access_token_with(id, true, &platform, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-test");
        assert_eq!(refresher.calls(), 0);

        swap::delete_private(id).await.unwrap();
    }
}
