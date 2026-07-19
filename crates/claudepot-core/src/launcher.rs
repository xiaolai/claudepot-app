//! Mode D — env-var inject launcher.
//!
//! Spawns a child process with `CLAUDE_CODE_OAUTH_TOKEN` set from the
//! account's stored credential. Zero disk state mutation.

use crate::blob::CredentialBlob;
use crate::cli_backend::swap::{self, DefaultRefresher, TokenRefresher};
use crate::cli_backend::CliPlatform;
use crate::oauth::refresh;
use crate::services::account_service;

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

    #[error("failed to read Claude Code credentials: {0}")]
    CredentialRead(String),

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
/// `expected_email` is the account's registered identity; it guards the
/// active-account heal path against a swap that changes CC's keychain
/// mid-fetch (running as the wrong account is worse than failing).
pub async fn get_access_token(
    account_id: Uuid,
    expected_email: &str,
) -> Result<String, LauncherError> {
    let platform = crate::cli_backend::create_platform();
    get_access_token_with(
        account_id,
        expected_email,
        platform.as_ref(),
        &account_service::DefaultProfileFetcher,
        &DefaultRefresher,
    )
    .await
}

/// Testable core of [`get_access_token`]. Splits on ONE question: is CC's
/// keychain currently holding THIS account's exact live credential?
///
/// If it is, blindly refreshing our private-slot copy would rotate the
/// single-use refresh token CC still holds → CC's next refresh 401s →
/// forced re-login (the bug this guards). So in that case we either ride
/// CC's still-valid token (no rotation at all) or, if it has expired, heal
/// it through the SAME race-safe resolver that account reconciliation uses
/// — which re-reads the keychain before spending a refresh token and
/// CAS-writes the rotation back, so CC is healed, not orphaned.
///
/// If CC is NOT on this account, the private slot is authoritative and
/// refreshing it can orphan nobody — the original behavior, unchanged.
async fn get_access_token_with(
    account_id: Uuid,
    expected_email: &str,
    platform: &dyn CliPlatform,
    fetch_profile: &dyn account_service::ProfileFetcher,
    refresher: &dyn TokenRefresher,
) -> Result<String, LauncherError> {
    let slot_str = swap::load_private(account_id)
        .await
        .map_err(|_| LauncherError::NoStoredCredentials(account_id))?;
    let slot = CredentialBlob::from_json(&slot_str)
        .map_err(|e| LauncherError::CorruptBlob(e.to_string()))?;

    // A read failure is not evidence that CC is using another account. Read
    // it before the expiry fast path so a transiently locked/unreadable
    // credential store never gets collapsed into a safe-looking mismatch.
    let _initial_cc_state = cc_keychain_state(platform, &slot).await?;
    if !slot.is_expired(300) {
        return Ok(slot.claude_ai_oauth.access_token.clone());
    }

    // Every refresh-needed path reloads both stores while holding the same
    // cross-process lock used by account swaps. This closes the window where
    // two launchers refresh the same single-use token, and rechecks CC after
    // the initial point-in-time collision snapshot.
    refresh_expiring_token_locked(
        account_id,
        expected_email,
        platform,
        fetch_profile,
        refresher,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcCredentialState {
    Same,
    DifferentOrEmpty,
}

/// Read CC's credential state for a private slot.
///
/// `Ok(None)` and a valid credential with a different refresh token are clean
/// non-collision results. Read errors and parse failures are deliberately
/// errors: treating either as "different" could rotate a token CC still
/// owns while its store is temporarily unavailable or malformed.
async fn cc_keychain_state(
    platform: &dyn CliPlatform,
    slot: &CredentialBlob,
) -> Result<CcCredentialState, LauncherError> {
    match platform.read_default().await {
        Ok(Some(kc_str)) => {
            let kc = CredentialBlob::from_json(&kc_str).map_err(|e| {
                LauncherError::CredentialRead(format!("CC credential blob is unparseable: {e}"))
            })?;
            if kc.claude_ai_oauth.refresh_token == slot.claude_ai_oauth.refresh_token {
                Ok(CcCredentialState::Same)
            } else {
                Ok(CcCredentialState::DifferentOrEmpty)
            }
        }
        Ok(None) => Ok(CcCredentialState::DifferentOrEmpty),
        Err(e) => Err(LauncherError::CredentialRead(e.to_string())),
    }
}

/// Refresh an expiring token while holding the cross-process credential lock.
/// Reloading the private slot is as important as re-reading CC: another
/// launcher may have already consumed and saved the previous single-use
/// refresh token while this call was waiting for the lock.
async fn refresh_expiring_token_locked(
    account_id: Uuid,
    expected_email: &str,
    platform: &dyn CliPlatform,
    fetch_profile: &dyn account_service::ProfileFetcher,
    refresher: &dyn TokenRefresher,
) -> Result<String, LauncherError> {
    let _lock = swap::acquire_swap_lock().map_err(|e| {
        LauncherError::CredentialRead(format!("failed to acquire credential lock: {e}"))
    })?;

    let slot_str = swap::load_private(account_id)
        .await
        .map_err(|_| LauncherError::NoStoredCredentials(account_id))?;
    let slot = CredentialBlob::from_json(&slot_str)
        .map_err(|e| LauncherError::CorruptBlob(e.to_string()))?;

    // A competing launcher may have healed the slot while we waited.
    if !slot.is_expired(300) {
        return Ok(slot.claude_ai_oauth.access_token.clone());
    }

    match cc_keychain_state(platform, &slot).await? {
        CcCredentialState::Same => {
            tracing::debug!(account = %account_id, "active-account token expired/expiring, healing CC keychain");
            match account_service::resolve_cc_identity_force_refresh_locked(
                platform,
                fetch_profile,
                refresher,
                expected_email,
                &slot.claude_ai_oauth.refresh_token,
            )
            .await
            {
                Ok(Some((blob_str, email))) => {
                    if !email.eq_ignore_ascii_case(expected_email) {
                        return Err(LauncherError::RefreshFailed(format!(
                            "active Claude Code account changed during token heal \
                             (expected {expected_email}, keychain now {email}) — retry"
                        )));
                    }
                    let blob = CredentialBlob::from_json(&blob_str)
                        .map_err(|e| LauncherError::CorruptBlob(e.to_string()))?;
                    // Best effort: the keychain is authoritative for this
                    // active-account path, and the returned token is already
                    // safe to launch with if this mirror write fails.
                    let _ = swap::save_private(account_id, &blob_str).await;
                    Ok(blob.claude_ai_oauth.access_token.clone())
                }
                Ok(None) => Err(LauncherError::NoStoredCredentials(account_id)),
                Err(e) => Err(LauncherError::RefreshFailed(e.to_string())),
            }
        }
        CcCredentialState::DifferentOrEmpty => {
            tracing::debug!(account = %account_id, "access token expired/expiring, refreshing private slot");
            let token_resp = refresher
                .refresh(&slot.claude_ai_oauth.refresh_token)
                .await
                .map_err(|e| LauncherError::RefreshFailed(e.to_string()))?;
            let new_blob_str = refresh::build_blob(&token_resp, Some(&slot));
            swap::save_private(account_id, &new_blob_str)
                .await
                .map_err(|e| LauncherError::SaveFailed(e.to_string()))?;
            Ok(token_resp.access_token)
        }
    }
}

/// Spawn a child process with CLAUDE_CODE_OAUTH_TOKEN injected.
/// Returns the child's exit code.
pub async fn run(
    account_id: Uuid,
    expected_email: &str,
    args: &[String],
) -> Result<i32, LauncherError> {
    let platform = crate::cli_backend::create_platform();
    run_with(
        account_id,
        expected_email,
        args,
        platform.as_ref(),
        &account_service::DefaultProfileFetcher,
        &DefaultRefresher,
    )
    .await
}

/// Testable core of [`run`].
async fn run_with(
    account_id: Uuid,
    expected_email: &str,
    args: &[String],
    platform: &dyn CliPlatform,
    fetch_profile: &dyn account_service::ProfileFetcher,
    refresher: &dyn TokenRefresher,
) -> Result<i32, LauncherError> {
    // Audit Low: validate args BEFORE touching credentials. Previously
    // this fetched + possibly refreshed the token first, then
    // discovered args were empty — wasteful I/O and the error was
    // NoStoredCredentials instead of the more accurate NoCommand.
    if args.is_empty() {
        return Err(LauncherError::NoCommand);
    }

    let access_token = get_access_token_with(
        account_id,
        expected_email,
        platform,
        fetch_profile,
        refresher,
    )
    .await?;

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
    use crate::cli_backend::SwapError;
    use crate::error::OAuthError;
    use crate::oauth::profile::Profile;
    use crate::oauth::refresh::TokenResponse;
    use crate::testing::{
        expired_blob_json, expiring_soon_blob_json, fresh_blob_json, lock_data_dir,
        setup_test_data_dir,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ---- mocks ----

    /// Mock CC keychain: `read_default` returns the configured blob;
    /// `write_default` records the write and updates the stored blob (so
    /// the resolver's CAS re-read sees what it just wrote).
    struct MockPlatform {
        blob: Mutex<Option<String>>,
        reads: Mutex<std::collections::VecDeque<Option<String>>>,
        writes: Mutex<Vec<String>>,
    }
    impl MockPlatform {
        fn with(blob: Option<&str>) -> Self {
            Self {
                blob: Mutex::new(blob.map(String::from)),
                reads: Mutex::new(std::collections::VecDeque::new()),
                writes: Mutex::new(Vec::new()),
            }
        }
        fn with_reads(reads: Vec<Option<String>>) -> Self {
            assert!(!reads.is_empty());
            let last = reads.last().cloned().unwrap();
            Self {
                blob: Mutex::new(last),
                reads: Mutex::new(reads.into_iter().collect()),
                writes: Mutex::new(Vec::new()),
            }
        }
        fn empty() -> Self {
            Self::with(None)
        }
        fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            let mut reads = self.reads.lock().unwrap();
            if !reads.is_empty() {
                return Ok(reads.pop_front().unwrap());
            }
            Ok(self.blob.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            self.writes.lock().unwrap().push(blob.to_string());
            *self.blob.lock().unwrap() = Some(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    struct FailingPlatform;

    #[async_trait::async_trait]
    impl CliPlatform for FailingPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            Err(SwapError::KeychainError("keychain is locked".into()))
        }
        async fn write_default(&self, _blob: &str) -> Result<(), SwapError> {
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    /// Mock profile fetcher. `reject_then_ok` returns a one-shot 401 to
    /// drive the resolver's refresh path, then succeeds.
    struct MockProfileFetcher {
        email: String,
        first_401: Mutex<bool>,
    }
    impl MockProfileFetcher {
        fn ok(email: &str) -> Self {
            Self {
                email: email.into(),
                first_401: Mutex::new(false),
            }
        }
        fn reject_then_ok(email: &str) -> Self {
            Self {
                email: email.into(),
                first_401: Mutex::new(true),
            }
        }
    }
    #[async_trait::async_trait]
    impl account_service::ProfileFetcher for MockProfileFetcher {
        async fn fetch(&self, _access_token: &str) -> Result<Profile, OAuthError> {
            let mut pending = self.first_401.lock().unwrap();
            if *pending {
                *pending = false;
                return Err(OAuthError::AuthFailed("401".into()));
            }
            Ok(Profile {
                email: self.email.clone(),
                org_uuid: String::new(),
                org_name: String::new(),
                subscription_type: String::new(),
                rate_limit_tier: None,
                account_uuid: String::new(),
                display_name: None,
            })
        }
    }

    /// Mock token refresher; counts calls so tests can assert no rotation.
    struct MockRefresher {
        access: String,
        refresh: String,
        calls: AtomicUsize,
    }
    impl MockRefresher {
        fn new(access: &str, refresh: &str) -> Self {
            Self {
                access: access.into(),
                refresh: refresh.into(),
                calls: AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    #[async_trait::async_trait]
    impl TokenRefresher for MockRefresher {
        async fn refresh(&self, _refresh_token: &str) -> Result<TokenResponse, OAuthError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(TokenResponse {
                access_token: self.access.clone(),
                refresh_token: self.refresh.clone(),
                expires_in: 3600,
                scope: None,
                token_type: None,
            })
        }
    }

    // ---- private-slot path (CC not on this account) ----

    #[tokio::test]
    async fn fresh_non_colliding_returns_slot_token_no_refresh() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let platform = MockPlatform::empty(); // CC has no credentials
        let fetcher = MockProfileFetcher::ok("a@b.com");
        let refresher = MockRefresher::new("unused", "unused");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-test");
        assert_eq!(refresher.calls(), 0, "valid slot must not refresh");
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn missing_credentials_errors() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let result = get_access_token_with(
            id,
            "a@b.com",
            &MockPlatform::empty(),
            &MockProfileFetcher::ok("x"),
            &MockRefresher::new("a", "b"),
        )
        .await;
        assert!(matches!(result, Err(LauncherError::NoStoredCredentials(_))));
    }

    #[tokio::test]
    async fn corrupt_blob_errors() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, "not json").await.unwrap();
        let result = get_access_token_with(
            id,
            "a@b.com",
            &MockPlatform::empty(),
            &MockProfileFetcher::ok("x"),
            &MockRefresher::new("a", "b"),
        )
        .await;
        assert!(matches!(result, Err(LauncherError::CorruptBlob(_))));
        swap::delete_private(id).await.unwrap();
    }

    /// Non-active account with an expired slot: refresh the SLOT and never
    /// touch CC's keychain (CC is signed in as someone else).
    #[tokio::test]
    async fn non_colliding_expired_refreshes_slot_only() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &expired_blob_json()).await.unwrap();

        // Keychain holds a DIFFERENT account (different refresh token).
        let other = r#"{"claudeAiOauth":{"accessToken":"other-oat","refreshToken":"sk-ant-ort01-OTHER","expiresAt":9999999999999,"scopes":[],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}"#;
        let platform = MockPlatform::with(Some(other));
        let fetcher = MockProfileFetcher::ok("a@b.com");
        let refresher = MockRefresher::new("sk-ant-oat01-slotnew", "sk-ant-ort01-slotnew");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-slotnew");
        assert_eq!(refresher.calls(), 1);
        assert!(
            platform.writes().is_empty(),
            "a non-colliding refresh must not write CC's keychain"
        );
        let slot_after = swap::load_private(id).await.unwrap();
        assert!(slot_after.contains("sk-ant-oat01-slotnew"));
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn cc_read_failure_fails_closed_before_private_refresh() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &expired_blob_json()).await.unwrap();
        let refresher = MockRefresher::new("unused", "unused");

        let result = get_access_token_with(
            id,
            "a@b.com",
            &FailingPlatform,
            &MockProfileFetcher::ok("a@b.com"),
            &refresher,
        )
        .await;

        assert!(matches!(result, Err(LauncherError::CredentialRead(_))));
        assert_eq!(
            refresher.calls(),
            0,
            "must not refresh on uncertain CC state"
        );
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn cc_parse_failure_fails_closed_before_private_refresh() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &expired_blob_json()).await.unwrap();
        let refresher = MockRefresher::new("unused", "unused");

        let result = get_access_token_with(
            id,
            "a@b.com",
            &MockPlatform::with(Some("not json")),
            &MockProfileFetcher::ok("a@b.com"),
            &refresher,
        )
        .await;

        assert!(matches!(result, Err(LauncherError::CredentialRead(_))));
        assert_eq!(
            refresher.calls(),
            0,
            "must not refresh on uncertain CC state"
        );
        swap::delete_private(id).await.unwrap();
    }

    // ---- collision path (CC IS live on this account) ----

    /// CC holds this exact credential and it is still valid → hand back CC's
    /// live token, rotate NOTHING (the fix: no re-login).
    #[tokio::test]
    async fn colliding_valid_uses_keychain_without_rotation() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = fresh_blob_json(); // refresh sk-ant-ort01-test, valid
        swap::save_private(id, &blob).await.unwrap();

        let platform = MockPlatform::with(Some(&blob)); // same refresh token → colliding
        let fetcher = MockProfileFetcher::ok("a@b.com");
        let refresher = MockRefresher::new("unused", "unused");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-test");
        assert_eq!(refresher.calls(), 0, "must not rotate CC's live token");
        assert!(
            platform.writes().is_empty(),
            "must not write CC's keychain when the live token is valid"
        );
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn colliding_near_expiry_forces_keychain_refresh() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let expiring = expiring_soon_blob_json();
        swap::save_private(id, &expiring).await.unwrap();

        let platform = MockPlatform::with(Some(&expiring));
        let fetcher = MockProfileFetcher::ok("a@b.com");
        let refresher = MockRefresher::new("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-healed");
        assert_eq!(
            refresher.calls(),
            1,
            "the five-minute margin must be honored"
        );
        assert_eq!(platform.writes().len(), 1, "refresh must heal CC in place");
        swap::delete_private(id).await.unwrap();
    }

    /// CC holds this exact credential but it has expired → heal via the
    /// race-safe resolver, writing the rotation BACK to the keychain so CC
    /// is healed (not orphaned), and return the rotated token.
    #[tokio::test]
    async fn colliding_expired_heals_keychain_in_place() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let expired = expired_blob_json(); // refresh sk-ant-ort01-test, expired
        swap::save_private(id, &expired).await.unwrap();

        let platform = MockPlatform::with(Some(&expired)); // same refresh token → colliding
        let fetcher = MockProfileFetcher::reject_then_ok("a@b.com"); // /profile 401 then ok
        let refresher = MockRefresher::new("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-healed");
        assert_eq!(refresher.calls(), 1);
        let writes = platform.writes();
        assert_eq!(writes.len(), 1, "exactly one keychain heal write");
        assert!(
            writes[0].contains("sk-ant-oat01-healed"),
            "the rotation must land in CC's keychain"
        );
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_active_heals_spend_single_use_refresh_once() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let expired = expired_blob_json();
        swap::save_private(id, &expired).await.unwrap();

        let platform = std::sync::Arc::new(MockPlatform::with(Some(&expired)));
        let fetcher = std::sync::Arc::new(MockProfileFetcher::ok("a@b.com"));
        let refresher = std::sync::Arc::new(MockRefresher::new(
            "sk-ant-oat01-healed",
            "sk-ant-ort01-healed",
        ));

        let (first, second) = tokio::join!(
            get_access_token_with(id, "a@b.com", &*platform, &*fetcher, &*refresher),
            get_access_token_with(id, "a@b.com", &*platform, &*fetcher, &*refresher),
        );

        assert_eq!(first.unwrap(), "sk-ant-oat01-healed");
        assert_eq!(second.unwrap(), "sk-ant-oat01-healed");
        assert_eq!(
            refresher.calls(),
            1,
            "concurrent launches must single-flight refresh"
        );
        assert_eq!(
            platform.writes().len(),
            1,
            "only one keychain heal is needed"
        );
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn cc_switch_to_this_account_is_rechecked_before_private_refresh() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let expired = expired_blob_json();
        swap::save_private(id, &expired).await.unwrap();

        let other = r#"{"claudeAiOauth":{"accessToken":"other-oat","refreshToken":"sk-ant-ort01-OTHER","expiresAt":9999999999999,"scopes":[],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}"#;
        // Initial snapshot says "different account"; the locked re-read
        // sees CC switch to this account before we spend the slot token.
        let platform = MockPlatform::with_reads(vec![Some(other.into()), Some(expired.clone())]);
        let fetcher = MockProfileFetcher::reject_then_ok("a@b.com");
        let refresher = MockRefresher::new("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-healed");
        assert_eq!(
            refresher.calls(),
            1,
            "the refresh must use the CC-heal path"
        );
        assert_eq!(
            platform.writes().len(),
            1,
            "CC must receive the rotated blob"
        );
        swap::delete_private(id).await.unwrap();
    }

    /// TOCTOU guard: if a swap changes CC's keychain identity while the heal
    /// is in flight, the resolver returns a DIFFERENT account's email —
    /// refuse rather than run the command as the wrong identity.
    #[tokio::test]
    async fn colliding_heal_refuses_when_identity_changes_mid_flight() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let expired = expired_blob_json();
        swap::save_private(id, &expired).await.unwrap();

        let platform = MockPlatform::with(Some(&expired));
        // Resolver ends up resolving a DIFFERENT account than expected.
        let fetcher = MockProfileFetcher::reject_then_ok("someone-else@evil.com");
        let refresher = MockRefresher::new("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let result =
            get_access_token_with(id, "expected@me.com", &platform, &fetcher, &refresher).await;
        assert!(
            matches!(result, Err(LauncherError::RefreshFailed(_))),
            "identity mismatch must refuse, got {result:?}"
        );
        swap::delete_private(id).await.unwrap();
    }

    // ---- run_with (spawn) ----

    #[tokio::test]
    async fn run_empty_args_returns_no_command() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let result = run_with(
            id,
            "a@b.com",
            &[],
            &MockPlatform::empty(),
            &MockProfileFetcher::ok("x"),
            &MockRefresher::new("a", "b"),
        )
        .await;
        assert!(matches!(result, Err(LauncherError::NoCommand)));
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn run_executes_command() {
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

        let exit_code = run_with(
            id,
            "a@b.com",
            &args,
            &MockPlatform::empty(),
            &MockProfileFetcher::ok("x"),
            &MockRefresher::new("a", "b"),
        )
        .await
        .unwrap();
        assert_eq!(exit_code, 0);
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn run_nonexistent_command_returns_spawn_failed() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let args = vec!["/nonexistent/binary/that/doesnt/exist".to_string()];
        let result = run_with(
            id,
            "a@b.com",
            &args,
            &MockPlatform::empty(),
            &MockProfileFetcher::ok("x"),
            &MockRefresher::new("a", "b"),
        )
        .await;
        assert!(matches!(result, Err(LauncherError::SpawnFailed(_))));
        swap::delete_private(id).await.unwrap();
    }
}
