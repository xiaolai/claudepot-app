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

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl crate::error_code::ErrorCode for LauncherError {
    fn code(&self) -> &'static str {
        match self {
            LauncherError::NoStoredCredentials(_) => "launcher.no_stored_credentials",
            LauncherError::CorruptBlob(_) => "launcher.corrupt_blob",
            LauncherError::CredentialRead(_) => "launcher.credential_read",
            LauncherError::RefreshFailed(_) => "launcher.refresh_failed",
            LauncherError::SaveFailed(_) => "launcher.save_failed",
            LauncherError::NoCommand => "launcher.no_command",
            LauncherError::SpawnFailed(_) => "launcher.spawn_failed",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            LauncherError::NoStoredCredentials(uuid) => {
                serde_json::json!({ "uuid": uuid.to_string() })
            }
            // Every `detail` here is a serde/keychain/HTTP diagnostic —
            // the credential blob itself is never formatted into one.
            // `rules/rust-conventions.md`'s token rule applies: if a
            // constructor ever starts interpolating a token, this arm
            // has to stop carrying it.
            LauncherError::CorruptBlob(detail)
            | LauncherError::CredentialRead(detail)
            | LauncherError::RefreshFailed(detail)
            | LauncherError::SaveFailed(detail)
            | LauncherError::SpawnFailed(detail) => serde_json::json!({ "detail": detail }),
            LauncherError::NoCommand => serde_json::json!({}),
        }
    }
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
    let claude_json = crate::cli_backend::claude_json::default_path();
    get_access_token_with(
        account_id,
        expected_email,
        platform.as_ref(),
        &account_service::DefaultProfileFetcher,
        &DefaultRefresher,
        claude_json.as_deref(),
    )
    .await
}

/// Testable core of [`get_access_token`]. Splits on ONE question: is
/// Claude Code signed in as THIS account right now?
///
/// If it is, CC's slot holds the live login and Claudepot's private copy
/// is out of date the moment CC renews it. Two things follow. The token
/// to hand out is CC's, never the private copy's — renewing a login
/// revokes the access token issued before it, so the private copy's
/// token is dead even while its `expiresAt` says otherwise. And when
/// CC's token needs renewing, it is renewed through CC's slot by the
/// race-safe resolver, never by spending the private copy's refresh
/// token, which CC has already spent.
///
/// "This account" is decided by identity, not by comparing refresh
/// tokens: the two stop matching at CC's first renewal, which is
/// exactly when the private copy goes stale.
///
/// If CC is NOT on this account, the private slot is authoritative and
/// refreshing it can orphan nobody.
async fn get_access_token_with(
    account_id: Uuid,
    expected_email: &str,
    platform: &dyn CliPlatform,
    fetch_profile: &dyn account_service::ProfileFetcher,
    refresher: &dyn TokenRefresher,
    claude_json: Option<&std::path::Path>,
) -> Result<String, LauncherError> {
    let slot = load_slot(account_id).await?;

    // A read failure is not evidence that CC is using another account.
    // Read it before the expiry fast path so a transiently locked or
    // unreadable credential store never collapses into a safe-looking
    // mismatch.
    match cc_state(platform, &slot, expected_email, fetch_profile, claude_json).await? {
        CcCredentialState::Same(cc) if !cc.is_expired(300) => {
            return Ok(cc.claude_ai_oauth.access_token)
        }
        CcCredentialState::DifferentOrEmpty if !slot.is_expired(300) => {
            return Ok(slot.claude_ai_oauth.access_token)
        }
        _ => {}
    }

    // Every refresh-needed path reloads both stores while holding the same
    // cross-process lock used by account swaps. This closes the window where
    // two launchers refresh the same single-use token, and rechecks CC after
    // the initial point-in-time snapshot.
    refresh_expiring_token_locked(
        account_id,
        expected_email,
        platform,
        fetch_profile,
        refresher,
        claude_json,
    )
    .await
}

async fn load_slot(account_id: Uuid) -> Result<CredentialBlob, LauncherError> {
    let slot_str = swap::load_private(account_id)
        .await
        .map_err(|_| LauncherError::NoStoredCredentials(account_id))?;
    CredentialBlob::from_json(&slot_str).map_err(|e| LauncherError::CorruptBlob(e.to_string()))
}

#[derive(Debug)]
pub(crate) enum CcCredentialState {
    /// CC is signed in as this account; carries CC's current blob.
    Same(CredentialBlob),
    DifferentOrEmpty,
}

/// Is CC signed in as `expected_email`?
///
/// Identical refresh tokens answer it offline. Otherwise CC's own access
/// token is asked `/profile`; when that cannot answer (CC's token has
/// expired, or the network is down), the `oauthAccount` CC records in its
/// global config is the offline fallback. With no answer at all this
/// fails closed: guessing "different" would spend a refresh token CC may
/// hold, and guessing "same" would hand out the wrong account's token.
///
/// No credentials, and CC's cleared-credentials sentinel, are clean
/// "different" results. Read errors and parse failures are errors.
pub(crate) async fn cc_state(
    platform: &dyn CliPlatform,
    slot: &CredentialBlob,
    expected_email: &str,
    fetch_profile: &dyn account_service::ProfileFetcher,
    claude_json: Option<&std::path::Path>,
) -> Result<CcCredentialState, LauncherError> {
    let cc = match platform.read_default().await {
        Ok(Some(kc_str)) => CredentialBlob::from_json(&kc_str).map_err(|e| {
            LauncherError::CredentialRead(format!("CC credential blob is unparseable: {e}"))
        })?,
        Ok(None) => return Ok(CcCredentialState::DifferentOrEmpty),
        Err(e) => return Err(LauncherError::CredentialRead(e.to_string())),
    };
    if cc.is_signed_out() {
        return Ok(CcCredentialState::DifferentOrEmpty);
    }
    if cc.claude_ai_oauth.refresh_token == slot.claude_ai_oauth.refresh_token {
        return Ok(CcCredentialState::Same(cc));
    }
    let cc_email = match fetch_profile.fetch(&cc.claude_ai_oauth.access_token).await {
        Ok(profile) => Some(profile.email),
        Err(_) => claude_json
            .and_then(|path| {
                crate::cli_backend::claude_json::read_oauth_account(path)
                    .ok()
                    .flatten()
            })
            .and_then(|account| {
                account
                    .get("emailAddress")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            }),
    };
    match cc_email {
        Some(email) if email.eq_ignore_ascii_case(expected_email) => {
            Ok(CcCredentialState::Same(cc))
        }
        Some(_) => Ok(CcCredentialState::DifferentOrEmpty),
        None => Err(LauncherError::CredentialRead(
            "cannot tell which account Claude Code is signed in as".into(),
        )),
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
    claude_json: Option<&std::path::Path>,
) -> Result<String, LauncherError> {
    let _lock = swap::acquire_swap_lock().map_err(|e| {
        LauncherError::CredentialRead(format!("failed to acquire credential lock: {e}"))
    })?;

    // A competing launcher may have healed either store while we waited.
    let slot = load_slot(account_id).await?;
    match cc_state(platform, &slot, expected_email, fetch_profile, claude_json).await? {
        CcCredentialState::Same(cc) => {
            if !cc.is_expired(300) {
                return Ok(cc.claude_ai_oauth.access_token);
            }
            tracing::debug!(account = %account_id, "active-account token expired/expiring, healing CC keychain");
            // CC's refresh token, never the private copy's: CC has spent
            // the private copy's at its last renewal.
            match account_service::resolve_cc_identity_force_refresh_locked(
                platform,
                fetch_profile,
                refresher,
                expected_email,
                &cc.claude_ai_oauth.refresh_token,
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
            if !slot.is_expired(300) {
                return Ok(slot.claude_ai_oauth.access_token.clone());
            }
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
    let claude_json = crate::cli_backend::claude_json::default_path();
    run_with(
        account_id,
        expected_email,
        args,
        platform.as_ref(),
        &account_service::DefaultProfileFetcher,
        &DefaultRefresher,
        claude_json.as_deref(),
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
    claude_json: Option<&std::path::Path>,
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
        claude_json,
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
        /// Access tokens that belong to some other account, answered
        /// before (and without consuming) the one-shot 401.
        others: Vec<(String, String)>,
        /// Access tokens the server rejects every time.
        rejected: Vec<String>,
    }
    impl MockProfileFetcher {
        fn ok(email: &str) -> Self {
            Self {
                email: email.into(),
                first_401: Mutex::new(false),
                others: Vec::new(),
                rejected: Vec::new(),
            }
        }
        fn reject_then_ok(email: &str) -> Self {
            Self {
                first_401: Mutex::new(true),
                ..Self::ok(email)
            }
        }
        fn with_other(mut self, access_token: &str, email: &str) -> Self {
            self.others.push((access_token.into(), email.into()));
            self
        }
        fn rejecting(mut self, access_token: &str) -> Self {
            self.rejected.push(access_token.into());
            self
        }
    }
    #[async_trait::async_trait]
    impl account_service::ProfileFetcher for MockProfileFetcher {
        async fn fetch(&self, access_token: &str) -> Result<Profile, OAuthError> {
            if self.rejected.iter().any(|t| t == access_token) {
                return Err(OAuthError::AuthFailed("401".into()));
            }
            let email = self
                .others
                .iter()
                .find(|(t, _)| t == access_token)
                .map(|(_, e)| e.clone());
            if let Some(email) = email {
                return Ok(profile_for(&email));
            }
            let mut pending = self.first_401.lock().unwrap();
            if *pending {
                *pending = false;
                return Err(OAuthError::AuthFailed("401".into()));
            }
            Ok(profile_for(&self.email))
        }
    }

    fn profile_for(email: &str) -> Profile {
        Profile {
            email: email.into(),
            org_uuid: String::new(),
            org_name: String::new(),
            subscription_type: String::new(),
            rate_limit_tier: None,
            account_uuid: String::new(),
            display_name: None,
        }
    }

    /// Mock token refresher; counts calls so tests can assert no rotation.
    struct MockRefresher {
        access: String,
        refresh: String,
        calls: AtomicUsize,
        spent: Mutex<Vec<String>>,
    }
    impl MockRefresher {
        fn new(access: &str, refresh: &str) -> Self {
            Self {
                access: access.into(),
                refresh: refresh.into(),
                calls: AtomicUsize::new(0),
                spent: Mutex::new(Vec::new()),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        /// The refresh tokens this refresher was asked to spend.
        fn spent(&self) -> Vec<String> {
            self.spent.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl TokenRefresher for MockRefresher {
        async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, OAuthError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.spent.lock().unwrap().push(refresh_token.into());
            Ok(TokenResponse {
                access_token: self.access.clone(),
                refresh_token: self.refresh.clone(),
                expires_in: 3600,
                scope: None,
                token_type: None,
                refresh_token_expires_in: None,
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

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
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
            None,
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
            None,
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
        let fetcher = MockProfileFetcher::ok("a@b.com").with_other("other-oat", "other@b.com");
        let refresher = MockRefresher::new("sk-ant-oat01-slotnew", "sk-ant-ort01-slotnew");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
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
            None,
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
            None,
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

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
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

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
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

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
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
            get_access_token_with(id, "a@b.com", &*platform, &*fetcher, &*refresher, None),
            get_access_token_with(id, "a@b.com", &*platform, &*fetcher, &*refresher, None),
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
        let fetcher =
            MockProfileFetcher::reject_then_ok("a@b.com").with_other("other-oat", "other@b.com");
        let refresher = MockRefresher::new("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
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
            get_access_token_with(id, "expected@me.com", &platform, &fetcher, &refresher, None)
                .await;
        assert!(
            matches!(result, Err(LauncherError::RefreshFailed(_))),
            "identity mismatch must refuse, got {result:?}"
        );
        swap::delete_private(id).await.unwrap();
    }

    // ---- CC renewed this account since Claudepot last saw it ----

    fn cc_blob(access: &str, refresh: &str, expires_at: i64) -> String {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{access}","refreshToken":"{refresh}","expiresAt":{expires_at},"scopes":[],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}}}"#
        )
    }

    fn claude_json_for(email: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("claude-json-{}.json", Uuid::new_v4()));
        std::fs::write(
            &path,
            format!(r#"{{"oauthAccount":{{"emailAddress":"{email}"}}}}"#),
        )
        .unwrap();
        path
    }

    /// The bug this guards: once CC renews, its refresh token no longer
    /// matches the private copy, and the old equality test called that
    /// "a different account" — handing out the private copy's access
    /// token, which the renewal had already revoked.
    #[tokio::test]
    async fn cc_renewed_this_account_hands_out_ccs_token_not_the_stale_copy() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).await.unwrap();

        let far = chrono::Utc::now().timestamp_millis() + 3_600_000;
        let platform = MockPlatform::with(Some(&cc_blob(
            "sk-ant-oat01-ccnew",
            "sk-ant-ort01-CCNEW",
            far,
        )));
        let fetcher = MockProfileFetcher::ok("a@b.com");
        let refresher = MockRefresher::new("unused", "unused");

        let token = get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None)
            .await
            .unwrap();
        assert_eq!(token, "sk-ant-oat01-ccnew");
        assert_eq!(refresher.calls(), 0);
        assert!(platform.writes().is_empty());
        swap::delete_private(id).await.unwrap();
    }

    /// When CC's renewed token needs renewing again, the refresh token
    /// spent is CC's — never the private copy's, which CC already spent.
    /// CC's expired token cannot answer `/profile`, so identity comes from
    /// CC's global config.
    #[tokio::test]
    async fn cc_renewed_this_account_heals_with_ccs_refresh_token() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &expired_blob_json()).await.unwrap();

        let past = chrono::Utc::now().timestamp_millis() - 60_000;
        let platform = MockPlatform::with(Some(&cc_blob(
            "sk-ant-oat01-ccold",
            "sk-ant-ort01-CCNEW",
            past,
        )));
        let fetcher = MockProfileFetcher::ok("a@b.com").rejecting("sk-ant-oat01-ccold");
        let refresher = MockRefresher::new("sk-ant-oat01-healed", "sk-ant-ort01-healed");
        let claude_json = claude_json_for("a@b.com");

        let token = get_access_token_with(
            id,
            "a@b.com",
            &platform,
            &fetcher,
            &refresher,
            Some(&claude_json),
        )
        .await
        .unwrap();
        assert_eq!(token, "sk-ant-oat01-healed");
        assert_eq!(refresher.spent(), vec!["sk-ant-ort01-CCNEW".to_string()]);
        assert_eq!(
            platform.writes().len(),
            1,
            "the renewal must land in CC's slot"
        );
        std::fs::remove_file(claude_json).unwrap();
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn offline_fallback_names_another_account_so_the_slot_refreshes() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &expired_blob_json()).await.unwrap();

        let past = chrono::Utc::now().timestamp_millis() - 60_000;
        let platform = MockPlatform::with(Some(&cc_blob(
            "sk-ant-oat01-them",
            "sk-ant-ort01-THEM",
            past,
        )));
        let fetcher = MockProfileFetcher::ok("a@b.com").rejecting("sk-ant-oat01-them");
        let refresher = MockRefresher::new("sk-ant-oat01-slotnew", "sk-ant-ort01-slotnew");
        let claude_json = claude_json_for("them@b.com");

        let token = get_access_token_with(
            id,
            "a@b.com",
            &platform,
            &fetcher,
            &refresher,
            Some(&claude_json),
        )
        .await
        .unwrap();
        assert_eq!(token, "sk-ant-oat01-slotnew");
        assert_eq!(refresher.spent(), vec!["sk-ant-ort01-test".to_string()]);
        assert!(platform.writes().is_empty(), "CC's slot is someone else's");
        std::fs::remove_file(claude_json).unwrap();
        swap::delete_private(id).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_cc_identity_fails_closed_without_spending_anything() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &expired_blob_json()).await.unwrap();

        let past = chrono::Utc::now().timestamp_millis() - 60_000;
        let platform =
            MockPlatform::with(Some(&cc_blob("sk-ant-oat01-who", "sk-ant-ort01-WHO", past)));
        let fetcher = MockProfileFetcher::ok("a@b.com").rejecting("sk-ant-oat01-who");
        let refresher = MockRefresher::new("unused", "unused");

        let result =
            get_access_token_with(id, "a@b.com", &platform, &fetcher, &refresher, None).await;
        assert!(
            matches!(result, Err(LauncherError::CredentialRead(_))),
            "{result:?}"
        );
        assert_eq!(refresher.calls(), 0);
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
            None,
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
            None,
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
            None,
        )
        .await;
        assert!(matches!(result, Err(LauncherError::SpawnFailed(_))));
        swap::delete_private(id).await.unwrap();
    }
}
