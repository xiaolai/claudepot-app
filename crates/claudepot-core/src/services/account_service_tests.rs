//! Inline test module for `account_service.rs`. Lives in this sibling file
//! so `account_service.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "account_service_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use crate::error::{OAuthError, SwapError};
use crate::oauth::refresh::TokenResponse;
use crate::testing::{
    fresh_blob_json, make_account, setup_test_data_dir, test_store, DATA_DIR_LOCK,
};

fn insert_account(store: &AccountStore, email: &str) -> Account {
    let account = make_account(email);
    store.insert(&account).unwrap();
    account
}

// -- Mock infrastructure --

struct MockPlatform {
    /// Scripted queue of `read_default` responses. Calls pop from
    /// the front while more than one response remains; once the
    /// queue holds its last entry, further calls clone it
    /// indefinitely. This preserves the original single-blob
    /// behaviour (`MockPlatform::new`) while letting race-sensitive
    /// tests (`MockPlatform::with_read_sequence`) script a keychain
    /// that appears to change between reads — modelling a
    /// concurrent writer.
    reads: std::sync::Mutex<std::collections::VecDeque<Option<String>>>,
    /// Full history of `write_default` payloads so tests can assert
    /// both whether a write happened and in what order.
    writes: std::sync::Mutex<Vec<String>>,
}

impl MockPlatform {
    fn new(blob: Option<String>) -> Self {
        let mut q = std::collections::VecDeque::new();
        q.push_back(blob);
        Self {
            reads: std::sync::Mutex::new(q),
            writes: std::sync::Mutex::new(Vec::new()),
        }
    }
    /// Build a platform whose `read_default` returns each scripted
    /// value in order, then repeats the last one. Used by the
    /// race regression tests to inject a "CC rotated the blob
    /// while we were mid-flight" transition.
    fn with_read_sequence(reads: Vec<Option<String>>) -> Self {
        assert!(
            !reads.is_empty(),
            "with_read_sequence needs at least one scripted response"
        );
        Self {
            reads: std::sync::Mutex::new(reads.into_iter().collect()),
            writes: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn last_written(&self) -> Option<String> {
        self.writes.lock().unwrap().last().cloned()
    }
    fn write_count(&self) -> usize {
        self.writes.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl cli_backend::CliPlatform for MockPlatform {
    async fn read_default(&self) -> Result<Option<String>, SwapError> {
        let mut q = self.reads.lock().unwrap();
        if q.len() > 1 {
            Ok(q.pop_front().unwrap())
        } else {
            Ok(q.front().cloned().unwrap_or(None))
        }
    }
    async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
        self.writes.lock().unwrap().push(blob.to_string());
        Ok(())
    }
    async fn touch_credfile(&self) -> Result<(), SwapError> {
        Ok(())
    }
}

/// Profile fetcher with an optional response queue. `ok`/`failing`
/// preserve the original single-response behaviour (every `fetch`
/// returns the same result). `sequence` pops one result per call —
/// used by auto-refresh tests where the first `/profile` call 401s
/// on a stale access_token and the second call succeeds with the
/// fresh one.
struct MockProfileFetcher {
    profile: Result<profile::Profile, OAuthError>,
    queue: std::sync::Mutex<std::collections::VecDeque<Result<profile::Profile, OAuthError>>>,
    /// Records every access_token passed to `fetch` so tests can
    /// assert the retry used the new token (not the stale one).
    seen_tokens: std::sync::Mutex<Vec<String>>,
}

fn sample_profile(email: &str) -> profile::Profile {
    profile::Profile {
        email: email.to_string(),
        org_uuid: "org-uuid-1".to_string(),
        org_name: "Test Org".to_string(),
        subscription_type: "pro".to_string(),
        rate_limit_tier: Some("default_claude_pro".to_string()),
        account_uuid: "acc-uuid-1".to_string(),
        display_name: Some("Test User".to_string()),
    }
}

fn clone_oauth_error(e: &OAuthError) -> OAuthError {
    match e {
        OAuthError::AuthFailed(m) => OAuthError::AuthFailed(m.clone()),
        OAuthError::RefreshFailed(m) => OAuthError::RefreshFailed(m.clone()),
        OAuthError::ServerError(m) => OAuthError::ServerError(m.clone()),
        OAuthError::RateLimited { retry_after_secs } => OAuthError::RateLimited {
            retry_after_secs: *retry_after_secs,
        },
        // HttpError isn't constructible in tests; any remaining
        // variant collapses to AuthFailed so the fall-through
        // behaves like the original mock.
        _ => OAuthError::AuthFailed("mock error".into()),
    }
}

impl MockProfileFetcher {
    fn ok(email: &str) -> Self {
        Self {
            profile: Ok(sample_profile(email)),
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            seen_tokens: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn failing(msg: &str) -> Self {
        Self {
            profile: Err(OAuthError::AuthFailed(msg.to_string())),
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            seen_tokens: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn failing_with(err: OAuthError) -> Self {
        Self {
            profile: Err(err),
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            seen_tokens: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn sequence(responses: Vec<Result<profile::Profile, OAuthError>>) -> Self {
        Self {
            // `profile` is used as the fall-through once the queue
            // drains — set to a hard AuthFailed so an unexpected
            // extra call during tests fails loudly instead of
            // silently succeeding.
            profile: Err(OAuthError::AuthFailed(
                "MockProfileFetcher sequence exhausted".into(),
            )),
            queue: std::sync::Mutex::new(responses.into_iter().collect()),
            seen_tokens: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn tokens_seen(&self) -> Vec<String> {
        self.seen_tokens.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ProfileFetcher for MockProfileFetcher {
    async fn fetch(&self, access_token: &str) -> Result<profile::Profile, OAuthError> {
        self.seen_tokens
            .lock()
            .unwrap()
            .push(access_token.to_string());
        if let Some(next) = self.queue.lock().unwrap().pop_front() {
            return match next {
                Ok(p) => Ok(p),
                Err(e) => Err(clone_oauth_error(&e)),
            };
        }
        match &self.profile {
            Ok(p) => Ok(p.clone()),
            Err(e) => Err(clone_oauth_error(e)),
        }
    }
}

struct MockRefresher {
    response: Result<TokenResponse, OAuthError>,
    /// Records every refresh_token passed to `refresh` so tests can
    /// assert the race-aware path used the LATEST refresh_token,
    /// not the stale snapshot captured at the top of sync.
    seen_tokens: std::sync::Mutex<Vec<String>>,
}

impl MockRefresher {
    fn success() -> Self {
        Self {
            response: Ok(TokenResponse {
                access_token: "sk-ant-oat01-new".into(),
                refresh_token: "sk-ant-ort01-new".into(),
                expires_in: 3600,
                scope: Some("user:inference".into()),
                token_type: Some("Bearer".into()),
            }),
            seen_tokens: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn failing(msg: &str) -> Self {
        Self {
            response: Err(OAuthError::RefreshFailed(msg.to_string())),
            seen_tokens: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn tokens_seen(&self) -> Vec<String> {
        self.seen_tokens.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::cli_backend::swap::TokenRefresher for MockRefresher {
    async fn refresh(&self, rt: &str) -> Result<TokenResponse, OAuthError> {
        self.seen_tokens.lock().unwrap().push(rt.to_string());
        match &self.response {
            Ok(r) => Ok(TokenResponse {
                access_token: r.access_token.clone(),
                refresh_token: r.refresh_token.clone(),
                expires_in: r.expires_in,
                scope: r.scope.clone(),
                token_type: r.token_type.clone(),
            }),
            Err(OAuthError::RefreshFailed(msg)) => Err(OAuthError::RefreshFailed(msg.clone())),
            _ => Err(OAuthError::RefreshFailed("mock".into())),
        }
    }
}

// -- register_from_current_with tests --

#[tokio::test]
async fn test_register_from_current_success() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::ok("alice@example.com");

    let result = register_from_current_with(&store, &platform, &fetcher)
        .await
        .unwrap();
    assert_eq!(result.email, "alice@example.com");
    assert_eq!(result.org_name, "Test Org");
    assert_eq!(result.subscription_type, "pro");

    // Account inserted into store
    let found = store.find_by_email("alice@example.com").unwrap().unwrap();
    assert_eq!(found.email, "alice@example.com");
    assert!(found.has_cli_credentials);

    swap::delete_private(result.uuid).unwrap();
}

#[tokio::test]
async fn test_register_from_current_no_credentials() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let platform = MockPlatform::new(None);
    let fetcher = MockProfileFetcher::ok("alice@example.com");

    let result = register_from_current_with(&store, &platform, &fetcher).await;
    assert!(matches!(result, Err(RegisterError::NoCredentials)));
}

#[tokio::test]
async fn test_register_from_current_profile_fetch_fails() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::failing("401 Unauthorized");

    let result = register_from_current_with(&store, &platform, &fetcher).await;
    assert!(matches!(result, Err(RegisterError::ProfileFetch(_))));
}

#[tokio::test]
async fn test_register_from_current_duplicate_account() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    // Pre-register
    insert_account(&store, "dup@example.com");

    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::ok("dup@example.com");

    let result = register_from_current_with(&store, &platform, &fetcher).await;
    assert!(matches!(
        result,
        Err(RegisterError::AlreadyRegistered(_, _))
    ));
}

#[tokio::test]
async fn test_register_from_current_corrupt_blob() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let platform = MockPlatform::new(Some("not json".to_string()));
    let fetcher = MockProfileFetcher::ok("alice@example.com");

    let result = register_from_current_with(&store, &platform, &fetcher).await;
    assert!(matches!(result, Err(RegisterError::CredentialRead(_))));
}

// -- register_from_token_with tests --

#[tokio::test]
async fn test_register_from_token_success() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let refresher = MockRefresher::success();
    let fetcher = MockProfileFetcher::ok("bob@example.com");

    let result = register_from_token_with(&store, "rt-test", &refresher, &fetcher)
        .await
        .unwrap();

    assert_eq!(result.email, "bob@example.com");
    assert!(store.find_by_email("bob@example.com").unwrap().is_some());

    swap::delete_private(result.uuid).unwrap();
}

#[tokio::test]
async fn test_register_from_token_refresh_fails() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let refresher = MockRefresher::failing("invalid token");
    let fetcher = MockProfileFetcher::ok("bob@example.com");

    let result = register_from_token_with(&store, "rt-bad", &refresher, &fetcher).await;

    assert!(matches!(result, Err(RegisterError::ProfileFetch(_))));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("token exchange failed"));
}

#[tokio::test]
async fn test_register_from_token_duplicate() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();
    insert_account(&store, "dup@example.com");

    let refresher = MockRefresher::success();
    let fetcher = MockProfileFetcher::ok("dup@example.com");

    let result = register_from_token_with(&store, "rt-test", &refresher, &fetcher).await;

    assert!(matches!(
        result,
        Err(RegisterError::AlreadyRegistered(_, _))
    ));
}

// -- format_duration_mins tests --

#[test]
fn test_format_duration_mins() {
    assert_eq!(format_duration_mins(0), "0m");
    assert_eq!(format_duration_mins(-5), "0m");
    assert_eq!(format_duration_mins(1), "1m");
    assert_eq!(format_duration_mins(45), "45m");
    assert_eq!(format_duration_mins(60), "1h");
    assert_eq!(format_duration_mins(88), "1h 28m");
    assert_eq!(format_duration_mins(120), "2h");
    assert_eq!(format_duration_mins(125), "2h 5m");
    assert_eq!(format_duration_mins(1440), "1d");
    assert_eq!(format_duration_mins(1500), "1d 1h");
    assert_eq!(format_duration_mins(2880), "2d");
    assert_eq!(format_duration_mins(2945), "2d 1h"); // minutes dropped when days present
}

// -- token_health tests --

#[test]
fn test_token_health_no_credentials() {
    let health = token_health(Uuid::new_v4(), false);
    assert_eq!(health.status, "no credentials");
    assert!(health.remaining_mins.is_none());
}

#[test]
fn test_token_health_missing_blob() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let health = token_health(Uuid::new_v4(), true);
    assert_eq!(health.status, "missing");
}

#[test]
fn test_token_health_valid_token() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let id = Uuid::new_v4();
    swap::save_private(id, &fresh_blob_json()).unwrap();

    let health = token_health(id, true);
    assert!(health.status.contains("valid"));
    assert!(health.remaining_mins.unwrap() > 0);

    swap::delete_private(id).unwrap();
}

#[test]
fn test_token_health_expired_token() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let id = Uuid::new_v4();
    swap::save_private(id, &crate::testing::expired_blob_json()).unwrap();

    let health = token_health(id, true);
    assert_eq!(health.status, "expired");
    assert!(health.remaining_mins.unwrap() < 0);

    swap::delete_private(id).unwrap();
}

#[test]
fn test_token_health_corrupt_blob() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let id = Uuid::new_v4();
    swap::save_private(id, "not json").unwrap();

    let health = token_health(id, true);
    assert_eq!(health.status, "corrupt blob");

    swap::delete_private(id).unwrap();
}

#[tokio::test]
async fn test_remove_deletes_credential_file() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "cred@example.com");

    // Save a credential file
    swap::save_private(account.uuid, r#"{"test":"blob"}"#).unwrap();
    assert!(swap::load_private(account.uuid).is_ok());

    remove_account(&store, account.uuid, None).await.unwrap();

    // Credential file should be gone
    assert!(swap::load_private(account.uuid).is_err());
}

#[tokio::test]
async fn test_remove_deletes_desktop_profile() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "desk@example.com");

    // Create desktop profile dir
    let profile_dir = paths::desktop_profile_dir(account.uuid);
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(profile_dir.join("config.json"), "cfg").unwrap();

    let result = remove_account(&store, account.uuid, None).await.unwrap();
    assert!(result.had_desktop_profile);
    assert!(!profile_dir.exists());
}

#[tokio::test]
async fn test_remove_removes_from_db() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "db@example.com");

    remove_account(&store, account.uuid, None).await.unwrap();
    assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
}

#[tokio::test]
async fn test_remove_clears_active_cli() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "cli@example.com");
    store.set_active_cli(account.uuid).unwrap();

    let result = remove_account(&store, account.uuid, None).await.unwrap();
    assert!(result.was_cli_active);
    assert!(store.active_cli_uuid().unwrap().is_none());
}

#[tokio::test]
async fn test_remove_clears_active_desktop() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "desk2@example.com");
    store.set_active_desktop(account.uuid).unwrap();

    let result = remove_account(&store, account.uuid, None).await.unwrap();
    assert!(result.was_desktop_active);
    assert!(store.active_desktop_uuid().unwrap().is_none());
}

#[tokio::test]
async fn test_remove_nonexistent_returns_not_found() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();

    let result = remove_account(&store, Uuid::new_v4(), None).await;
    assert!(matches!(result, Err(RegisterError::NotFound)));
}

#[tokio::test]
async fn test_remove_missing_credential_succeeds_silently() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "warn@example.com");
    // Do NOT save_private — credential file doesn't exist

    let result = remove_account(&store, account.uuid, None).await.unwrap();
    // delete_private returns Ok when file doesn't exist,
    // so no warning is produced — this is correct behavior
    assert!(result.warnings.is_empty());
    // Account still removed from DB
    assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
}

#[tokio::test]
async fn test_remove_returns_correct_metadata() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db_dir) = test_store();
    let account = insert_account(&store, "meta@example.com");

    let result = remove_account(&store, account.uuid, None).await.unwrap();
    assert_eq!(result.email, "meta@example.com");
    assert!(!result.was_cli_active);
    assert!(!result.was_desktop_active);
    assert!(!result.had_desktop_profile);
}

// -- sync_from_current_cc --

#[tokio::test]
async fn test_sync_adopts_cc_blob_when_email_matches_registered_account() {
    // Startup scenario: CC is signed in as an account the user has
    // already registered in Claudepot, but Claudepot's stored blob
    // slot for that account is empty (e.g. after a reinstall).
    // sync_from_current_cc should write the blob + flip the flag +
    // set active_cli — no user action needed.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let account = insert_account(&store, "alice@example.com");
    // DB flag was flipped off (e.g. reinstall wiped storage).
    let _ = store.update_credentials_flag(account.uuid, false);
    // Capture the blob once — fresh_blob_json() uses Utc::now() so
    // calling it twice returns JSON strings whose expiresAt differs
    // by ~1ms, which makes the post-sync comparison flaky.
    let cc_blob = fresh_blob_json();
    let platform = MockPlatform::new(Some(cc_blob.clone()));
    let fetcher = MockProfileFetcher::ok("alice@example.com");
    let refresher = MockRefresher::success();

    let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();

    assert_eq!(synced, Some(account.uuid), "should report the synced uuid");
    // Blob now in Claudepot's storage.
    assert_eq!(swap::load_private(account.uuid).unwrap(), cc_blob);
    // active_cli aligned with CC's current reality.
    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(account.uuid.to_string())
    );
    // Happy path never touched the refresher — no blob rotation
    // should land in CC's keychain.
    assert!(
        platform.last_written().is_none(),
        "platform.write_default must not fire when /profile succeeds"
    );

    swap::delete_private(account.uuid).unwrap();
}

#[tokio::test]
async fn test_sync_is_noop_when_cc_email_is_not_registered() {
    // CC holds a blob for an account Claudepot doesn't know about.
    // We should NOT auto-register; just leave the state alone.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::ok("stranger@example.com");
    let refresher = MockRefresher::success();

    let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();

    assert_eq!(result, None);
    // No accounts were registered.
    assert_eq!(store.list().unwrap().len(), 0);
}

#[tokio::test]
async fn test_sync_is_noop_when_cc_has_no_credentials() {
    // CC empty (logged out) — sync should return Ok(None).
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    insert_account(&store, "alice@example.com");
    let platform = MockPlatform::new(None);
    let fetcher = MockProfileFetcher::ok("alice@example.com");
    let refresher = MockRefresher::success();

    let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_sync_refreshes_stale_access_token_and_retries_profile() {
    // The xaiolai scenario: CC's access_token expired in the
    // background. /profile returns 401, but the paired
    // refresh_token is still valid. Expected behavior: sync
    // silently rotates the tokens, writes the fresh blob back to
    // CC's keychain, then retries /profile and completes the
    // adopt flow. No user-facing error.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let account = insert_account(&store, "alice@example.com");
    let _ = store.update_credentials_flag(account.uuid, false);
    let cc_blob = fresh_blob_json();
    let platform = MockPlatform::new(Some(cc_blob.clone()));
    // First call: 401. Second call (after refresh): success.
    let fetcher = MockProfileFetcher::sequence(vec![
        Err(OAuthError::AuthFailed("401 Unauthorized".into())),
        Ok(sample_profile("alice@example.com")),
    ]);
    let refresher = MockRefresher::success();

    let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();

    assert_eq!(synced, Some(account.uuid));

    // CC's keychain got the rotated blob. The new blob must carry
    // the fresh access_token ("sk-ant-oat01-new" from
    // MockRefresher::success) — confirming we wrote the rotated
    // tokens back to CC and not the stale ones.
    let written = platform
        .last_written()
        .expect("write_default must fire after successful refresh");
    let written_blob = crate::blob::CredentialBlob::from_json(&written).unwrap();
    assert_eq!(
        written_blob.claude_ai_oauth.access_token, "sk-ant-oat01-new",
        "CC keychain must hold the freshly-rotated access token"
    );

    // The retry used the NEW access token, not the stale one.
    let tokens_seen = fetcher.tokens_seen();
    assert_eq!(tokens_seen.len(), 2, "profile fetch should run twice");
    assert_eq!(
        tokens_seen[0], "sk-ant-oat01-test",
        "first call must use the stale token from CC's blob"
    );
    assert_eq!(
        tokens_seen[1], "sk-ant-oat01-new",
        "retry must use the freshly-rotated access token"
    );

    // Claudepot's private slot also gets the fresh blob, and the
    // account's active_cli pointer is set.
    let stored = swap::load_private(account.uuid).unwrap();
    assert_eq!(
        stored, written,
        "private slot must match what we wrote to CC's keychain"
    );
    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(account.uuid.to_string())
    );

    swap::delete_private(account.uuid).unwrap();
}

#[tokio::test]
async fn test_sync_returns_auth_rejected_when_refresh_token_is_dead() {
    // Terminal case: access_token rejected AND refresh_token
    // refused. The user revoked access elsewhere, or the grant
    // aged out. Expected: AuthRejected — a first-class error the
    // UI can surface as "Sign in again" instead of the generic
    // ProfileFetch warning that's currently dropped on the floor.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    insert_account(&store, "alice@example.com");
    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::failing("401 Unauthorized");
    let refresher = MockRefresher::failing("refresh_token revoked");

    let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher).await;

    assert!(
        matches!(result, Err(RegisterError::AuthRejected)),
        "expected AuthRejected, got {:?}",
        result
    );
    // We never wrote anything to CC's keychain — refresh failed
    // before we had a fresh blob to write.
    assert!(platform.last_written().is_none());
}

/// Build a credential blob with caller-specified access_token /
/// refresh_token values. Used by the race regression tests to
/// stand up distinguishable "before" and "after" snapshots so
/// assertions can prove WHICH blob the sync path acted on.
fn blob_json_with(access: &str, refresh: &str) -> String {
    let expires = chrono::Utc::now().timestamp_millis() + 3_600_000;
    format!(
        r#"{{"claudeAiOauth":{{"accessToken":"{access}","refreshToken":"{refresh}","expiresAt":{expires},"scopes":["user:inference","user:profile"],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}}}"#
    )
}

#[tokio::test]
async fn test_sync_race_adopts_fresh_keychain_blob_without_burning_refresh_token() {
    // The user-reported scenario: CC auto-refreshed between our
    // initial keychain read and our /profile call. Our snapshot's
    // access_token is now stale, but CC wrote a fresh blob whose
    // access_token works. Before this fix, sync would refresh
    // using the stale refresh_token — which CC just consumed —
    // and report AuthRejected. After the fix, the re-read picks
    // up CC's fresh blob and /profile succeeds without touching
    // the refresh endpoint.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();
    let account = insert_account(&store, "alice@example.com");
    let _ = store.update_credentials_flag(account.uuid, false);

    let old_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
    let fresh_blob = blob_json_with("sk-ant-oat01-fresh", "sk-ant-ort01-fresh");
    let platform = MockPlatform::with_read_sequence(vec![
        Some(old_blob.clone()),
        Some(fresh_blob.clone()),
    ]);
    // First /profile (stale access) → 401, second (fresh access) → Ok.
    let fetcher = MockProfileFetcher::sequence(vec![
        Err(OAuthError::AuthFailed("401 Unauthorized".into())),
        Ok(sample_profile("alice@example.com")),
    ]);
    // Refresher configured to fail loudly — if the race-aware path
    // ever dispatches it when the fresh access_token already works,
    // this test catches the regression.
    let refresher = MockRefresher::failing("refresher must not be called");

    let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();

    assert_eq!(synced, Some(account.uuid));

    // /profile was called twice: once with the stale access_token,
    // then with the fresh one from CC's rotated blob.
    let tokens = fetcher.tokens_seen();
    assert_eq!(tokens.len(), 2, "expected exactly two /profile calls");
    assert_eq!(tokens[0], "sk-ant-oat01-stale");
    assert_eq!(tokens[1], "sk-ant-oat01-fresh");

    // Refresh endpoint must NOT have been hit — the race was
    // resolved by reading the fresh blob, not by burning a
    // refresh_token that CC had already consumed.
    assert!(
        refresher.tokens_seen().is_empty(),
        "refresh must not run when a fresh keychain read resolves /profile"
    );

    // CC's keychain wasn't overwritten — we didn't produce a new
    // blob to write.
    assert_eq!(platform.write_count(), 0);

    // Claudepot's private slot mirrors the FRESH blob, not the
    // stale snapshot. If we stored the stale one, the next swap
    // would feed CC dead tokens.
    let stored = swap::load_private(account.uuid).unwrap();
    assert_eq!(stored, fresh_blob);

    swap::delete_private(account.uuid).unwrap();
}

#[tokio::test]
async fn test_sync_race_refresh_uses_latest_refresh_token_not_stale_snapshot() {
    // Defence in depth: even when the fresh access_token from a
    // re-read also 401s (e.g. clock skew, server-side lag on
    // newly-rotated tokens), the refresh MUST be attempted with
    // the LATEST refresh_token from the keychain — not the stale
    // one captured at the top of sync. Calling /token with a
    // refresh_token that's already been consumed is what produced
    // the false AuthRejected in the first place.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();
    let account = insert_account(&store, "alice@example.com");
    let _ = store.update_credentials_flag(account.uuid, false);

    let stale_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
    let fresh_blob = blob_json_with("sk-ant-oat01-fresh", "sk-ant-ort01-fresh");
    let platform = MockPlatform::with_read_sequence(vec![
        Some(stale_blob.clone()),
        Some(fresh_blob.clone()),
        // Pre-write CAS read sees the same fresh blob — no further
        // race after refresh, so the CAS write proceeds.
        Some(fresh_blob.clone()),
    ]);
    // 1st /profile (stale access) → 401
    // 2nd /profile (fresh access) → 401 too (still in limbo)
    // 3rd /profile (post-refresh access) → Ok
    let fetcher = MockProfileFetcher::sequence(vec![
        Err(OAuthError::AuthFailed("401 stale".into())),
        Err(OAuthError::AuthFailed("401 fresh".into())),
        Ok(sample_profile("alice@example.com")),
    ]);
    let refresher = MockRefresher::success();

    let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();

    assert_eq!(synced, Some(account.uuid));

    // Critical assertion: refresh used the FRESH refresh_token
    // (from the re-read), not the stale one from our initial
    // snapshot. Reversing this is exactly what produces the false
    // AuthRejected banner.
    let rt_seen = refresher.tokens_seen();
    assert_eq!(rt_seen.len(), 1, "refresh must run exactly once");
    assert_eq!(
        rt_seen[0], "sk-ant-ort01-fresh",
        "refresh must use the LATEST refresh_token, not the stale snapshot"
    );

    // CAS allowed the write because the keychain still matched
    // `fresh_blob` when we checked pre-write. The written blob
    // carries the post-refresh access_token.
    assert_eq!(platform.write_count(), 1);
    let written = platform.last_written().unwrap();
    let written_blob = CredentialBlob::from_json(&written).unwrap();
    assert_eq!(written_blob.claude_ai_oauth.access_token, "sk-ant-oat01-new");

    swap::delete_private(account.uuid).unwrap();
}

#[tokio::test]
async fn test_sync_race_cas_skips_keychain_writeback_when_concurrent_writer_landed() {
    // Belt-and-braces: after our refresh succeeds, another writer
    // (another Claudepot instance, or CC racing the same window)
    // may have landed a different blob in the keychain. Writing
    // our rotated blob would clobber theirs. The CAS guard reads
    // the keychain right before the write and skips if it no
    // longer matches what we refreshed from. Because the rotated
    // blob CC never installed must NOT be persisted to our slot,
    // the function then re-reads the live blob and re-verifies
    // identity against it, persisting the intruder's blob (CC's
    // truth) into the matching account slot.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();
    let account = insert_account(&store, "alice@example.com");
    let _ = store.update_credentials_flag(account.uuid, false);

    let our_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
    let intruder_blob =
        blob_json_with("sk-ant-oat01-intruder", "sk-ant-ort01-intruder");
    let platform = MockPlatform::with_read_sequence(vec![
        // #1 initial snapshot.
        Some(our_blob.clone()),
        // #2 race-check re-read — still our blob, no race yet.
        Some(our_blob.clone()),
        // #3 pre-write CAS — surprise: someone wrote between
        // refresh and write-back.
        Some(intruder_blob.clone()),
    ]);
    let fetcher = MockProfileFetcher::sequence(vec![
        // Step 1: profile call on `our_blob` access_token → 401.
        Err(OAuthError::AuthFailed("401 Unauthorized".into())),
        // Step 4: re-verify the intruder's live blob succeeds
        // (still alice — same account, just rotated by the race).
        Ok(sample_profile("alice@example.com")),
    ]);
    let refresher = MockRefresher::success();

    let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .unwrap();

    assert_eq!(synced, Some(account.uuid));

    // CAS must have suppressed the write. Leaving the intruder's
    // newer blob in place is the correct trade-off: we don't know
    // what state they're in, but we know our rotated blob is no
    // fresher than theirs.
    assert_eq!(
        platform.write_count(),
        0,
        "CAS must skip write-back when the keychain changed during refresh"
    );

    // Persisted blob must be CC's live blob (intruder's), NOT the
    // rotated `new_blob_str` we minted but never installed. That
    // would mis-file our orphan token into the account's slot.
    let stored = swap::load_private(account.uuid).unwrap();
    assert_eq!(
        stored, intruder_blob,
        "must persist CC's live blob, never the orphan rotated blob"
    );

    swap::delete_private(account.uuid).unwrap();
}

#[tokio::test]
async fn test_sync_race_cas_miss_aborts_when_live_blob_unverifiable() {
    // CAS miss + live blob can't be verified (token rejected, blob
    // unparseable, etc.) → must NOT persist either the rotated
    // blob or the live blob. Surface a typed retry-able error.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();
    let account = insert_account(&store, "alice@example.com");
    let _ = store.update_credentials_flag(account.uuid, false);

    let our_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
    let intruder_blob =
        blob_json_with("sk-ant-oat01-intruder", "sk-ant-ort01-intruder");
    let platform = MockPlatform::with_read_sequence(vec![
        Some(our_blob.clone()),
        Some(our_blob.clone()),
        // CAS check — intruder's blob landed.
        Some(intruder_blob.clone()),
    ]);
    let fetcher = MockProfileFetcher::sequence(vec![
        // Step 1: 401 on our_blob access token.
        Err(OAuthError::AuthFailed("401 Unauthorized".into())),
        // Step 4: re-verify on intruder's blob also 401 (token
        // already rotated again, or simply unverifiable).
        Err(OAuthError::AuthFailed("401 Unauthorized".into())),
    ]);
    let refresher = MockRefresher::success();

    let err = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
        .await
        .expect_err("must abort when live blob can't be verified");
    assert!(
        matches!(err, RegisterError::CcChangedDuringRefresh),
        "expected CcChangedDuringRefresh, got {err:?}"
    );

    // Nothing was persisted to the account's private slot.
    assert!(
        swap::load_private(account.uuid).is_err(),
        "must not persist anything when live blob unverifiable"
    );
}

#[tokio::test]
async fn test_sync_treats_non_auth_profile_errors_as_transient() {
    // Guardrail: refresh should only kick in for 401 (AuthFailed).
    // Server-side errors, rate limits, and transport failures must
    // fall through to ProfileFetch so verified_email history
    // survives transient outages. Refresher must NOT be called.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    insert_account(&store, "alice@example.com");
    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::failing_with(OAuthError::ServerError(
        "502 Bad Gateway".into(),
    ));
    // Configure refresher to fail loudly — if sync ever dispatches
    // it for a non-auth error, this test will catch the regression.
    let refresher = MockRefresher::failing("refresher must not be called");

    let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher).await;

    assert!(
        matches!(result, Err(RegisterError::ProfileFetch(_))),
        "expected ProfileFetch (transient), got {:?}",
        result
    );
    assert!(
        platform.last_written().is_none(),
        "server errors must not trigger a keychain write"
    );
}

// -- Group 5: account service rollbacks --

#[tokio::test]
async fn test_register_from_current_duplicate_cleans_no_blob() {
    // When the fetched profile matches an existing account's email,
    // registration fails with AlreadyRegistered BEFORE any blob is saved.
    // Verify: no credential file for the attempted UUID exists after.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    insert_account(&store, "dup@example.com");
    let before_privates = count_private_files();

    let platform = MockPlatform::new(Some(fresh_blob_json()));
    let fetcher = MockProfileFetcher::ok("dup@example.com");
    let result = register_from_current_with(&store, &platform, &fetcher).await;
    assert!(matches!(
        result,
        Err(RegisterError::AlreadyRegistered(_, _))
    ));

    let after_privates = count_private_files();
    assert_eq!(
        before_privates, after_privates,
        "duplicate rejection must not leave orphan blob on disk"
    );
}

#[tokio::test]
async fn test_remove_account_preserves_files_on_db_failure() {
    // If store.remove() fails, credential file and profile dir must still
    // exist (irreversible file deletions gated behind successful DB remove).
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let account = insert_account(&store, "dbfail@example.com");
    swap::save_private(account.uuid, "credential-content").unwrap();
    let profile_dir = paths::desktop_profile_dir(account.uuid);
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(profile_dir.join("config.json"), "{}").unwrap();

    // Make store.remove() fail by dropping the accounts table.
    store.corrupt_for_test();

    let result = remove_account(&store, account.uuid, None).await;
    assert!(matches!(result, Err(RegisterError::Store(_))));

    // Credential + profile files still on disk since DB remove failed first.
    assert!(
        swap::load_private(account.uuid).is_ok(),
        "credential blob preserved after DB failure"
    );
    assert!(
        profile_dir.exists() && profile_dir.join("config.json").exists(),
        "desktop profile preserved after DB failure"
    );

    // Cleanup — tear down manually since store is now corrupt.
    let _ = swap::delete_private(account.uuid);
    let _ = std::fs::remove_dir_all(&profile_dir);
}

#[tokio::test]
async fn test_remove_account_clears_pointers_before_db_remove() {
    // The ordering fix: pointers are cleared before store.remove(). Even
    // though that's partly structural, the observable effect is: after a
    // successful remove_account, active_cli_uuid() and active_desktop_uuid()
    // return None for the removed account.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let account = insert_account(&store, "ordering@example.com");
    store.set_active_cli(account.uuid).unwrap();
    store.set_active_desktop(account.uuid).unwrap();

    let result = remove_account(&store, account.uuid, None).await.unwrap();
    assert!(result.was_cli_active);
    assert!(result.was_desktop_active);

    assert!(store.active_cli_uuid().unwrap().is_none());
    assert!(store.active_desktop_uuid().unwrap().is_none());
    assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
}

fn count_private_files() -> usize {
    let dir = crate::paths::claudepot_data_dir().join("credentials");
    std::fs::read_dir(&dir)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Reconcile tests (B-2)
// ---------------------------------------------------------------------

#[test]
fn test_reconcile_cli_flags_flips_stale_true_to_false() {
    // DB says the account has CLI credentials but the keychain is
    // empty (the user removed the blob out-of-band, or a swap
    // failed mid-write). reconcile_cli_flags must flip the flag to
    // false and report the flip.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let mut acct = make_account("stale-true@example.com");
    acct.has_cli_credentials = true;
    store.insert(&acct).unwrap();
    // No swap::save_private — keychain is empty for this uuid.

    let flips = reconcile_cli_flags(&store).unwrap();
    assert_eq!(flips.len(), 1);
    assert_eq!(flips[0].uuid, acct.uuid);
    assert_eq!(flips[0].email, acct.email);
    assert!(!flips[0].new_value);

    let after = store.find_by_uuid(acct.uuid).unwrap().unwrap();
    assert!(!after.has_cli_credentials);
}

#[test]
fn test_reconcile_cli_flags_flips_stale_false_to_true() {
    // DB says no CLI credentials but a parseable blob is on the
    // keychain. The flag must be lifted to true and the flip
    // reported.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let mut acct = make_account("stale-false@example.com");
    acct.has_cli_credentials = false;
    store.insert(&acct).unwrap();
    swap::save_private(acct.uuid, &crate::testing::fresh_blob_json()).unwrap();

    let flips = reconcile_cli_flags(&store).unwrap();
    assert_eq!(flips.len(), 1);
    assert_eq!(flips[0].uuid, acct.uuid);
    assert!(flips[0].new_value);

    let after = store.find_by_uuid(acct.uuid).unwrap().unwrap();
    assert!(after.has_cli_credentials);

    swap::delete_private(acct.uuid).unwrap();
}

#[test]
fn test_reconcile_cli_flags_idempotent() {
    // After a converged pass, a second run must report zero flips
    // and leave the DB untouched.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    // One account in alignment (false / no blob), one in
    // alignment (true / present blob).
    let mut a_no = make_account("none@example.com");
    a_no.has_cli_credentials = false;
    store.insert(&a_no).unwrap();

    let mut a_yes = make_account("yes@example.com");
    a_yes.has_cli_credentials = false; // start drifted
    store.insert(&a_yes).unwrap();
    swap::save_private(a_yes.uuid, &crate::testing::fresh_blob_json()).unwrap();

    // First pass converges the drifted row.
    let first = reconcile_cli_flags(&store).unwrap();
    assert_eq!(first.len(), 1);
    // Second pass on a converged store: empty Vec.
    let second = reconcile_cli_flags(&store).unwrap();
    assert!(
        second.is_empty(),
        "expected idempotent second pass, got {} flips",
        second.len()
    );

    swap::delete_private(a_yes.uuid).unwrap();
}

#[test]
fn test_reconcile_all_combines_cli_and_desktop() {
    // Drift one CLI flag (DB says true, keychain empty) and one
    // desktop flag (DB says true, snapshot dir absent). reconcile_all
    // must report both passes via its bundled outcome.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _db) = test_store();

    let mut cli_drift = make_account("cli@example.com");
    cli_drift.has_cli_credentials = true;
    store.insert(&cli_drift).unwrap();

    let mut desk_drift = make_account("desk@example.com");
    // Make CLI side aligned (false + no blob) so only desktop drifts.
    desk_drift.has_cli_credentials = false;
    desk_drift.has_desktop_profile = true;
    store.insert(&desk_drift).unwrap();
    // Snapshot dir intentionally missing on disk.

    let report = reconcile_all(&store).unwrap();
    assert_eq!(report.cli_flips.len(), 1);
    assert_eq!(report.cli_flips[0].uuid, cli_drift.uuid);
    assert!(!report.cli_flips[0].new_value);
    assert_eq!(report.desktop.flag_flips.len(), 1);
    assert_eq!(report.desktop.flag_flips[0].uuid, desk_drift.uuid);
    assert!(!report.desktop.flag_flips[0].new_value);
}

// -- Login progress tests (C-1) -------------------------------------

/// Recording sink — captures every `phase` / `error` call so tests
/// can assert ordering and content. Thread-safe via `Mutex`.
struct RecordingLoginSink {
    events: std::sync::Mutex<Vec<RecordedLoginEvent>>,
}

#[derive(Debug, Clone, PartialEq)]
enum RecordedLoginEvent {
    Phase(LoginPhase),
    Error(LoginPhase, String),
}

impl RecordingLoginSink {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn events(&self) -> Vec<RecordedLoginEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl LoginProgressSink for RecordingLoginSink {
    fn phase(&self, phase: LoginPhase) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedLoginEvent::Phase(phase));
    }
    fn error(&self, phase: LoginPhase, msg: &str) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedLoginEvent::Error(phase, msg.to_string()));
    }
}

/// Cancel before the subprocess finishes — assert sink saw
/// `Spawning` then `WaitingForBrowser` then an `error` whose detail
/// mentions cancellation.
#[tokio::test]
#[cfg(unix)]
async fn test_login_cancel_emits_error_phase_with_cancelled_msg() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Notify;

    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let acct = insert_account(&store, "alice@example.com");

    // Stub binary that blocks for 30s — long enough for the test
    // to fire its Notify before exit.
    let stub_dir = tempfile::tempdir().expect("mk stub tempdir");
    let stub = stub_dir.path().join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").expect("write stub");
    let mut perms = std::fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).unwrap();

    let sink = Arc::new(RecordingLoginSink::new());
    let notify = Arc::new(Notify::new());
    let notify_clone = notify.clone();
    let sink_clone = Arc::clone(&sink);
    let stub_path = stub.clone();
    let store_arc = Arc::new(store);
    let store_handle = Arc::clone(&store_arc);
    let uuid = acct.uuid;

    let task = tokio::spawn(async move {
        login_and_reimport_with_progress_test_binary(
            &store_handle,
            uuid,
            &stub_path,
            Some(notify_clone),
            sink_clone.as_ref(),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    notify.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancel should complete within 5s")
        .expect("join handle should not panic");
    assert!(outcome.is_err(), "expected RegisterError, got Ok");

    let events = sink.events();
    // Must have at least Spawning + WaitingForBrowser + error.
    assert_eq!(events[0], RecordedLoginEvent::Phase(LoginPhase::Spawning));
    assert_eq!(
        events[1],
        RecordedLoginEvent::Phase(LoginPhase::WaitingForBrowser)
    );
    match events.last().unwrap() {
        RecordedLoginEvent::Error(LoginPhase::WaitingForBrowser, msg) => {
            assert!(
                msg.to_lowercase().contains("cancel"),
                "error detail should mention cancellation; got: {msg}"
            );
        }
        other => panic!("expected error on WaitingForBrowser, got {other:?}"),
    }
}

/// `Spawning` fires before account validation; if the account is
/// missing the sink must see `Spawning` then `error(Spawning)`.
#[tokio::test]
async fn test_login_progress_emits_spawning_then_error_for_unknown_account() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();

    let sink = RecordingLoginSink::new();
    let result =
        login_and_reimport_with_progress(&store, Uuid::new_v4(), None, &sink).await;
    assert!(matches!(result, Err(RegisterError::NotFound)));

    let events = sink.events();
    assert_eq!(events[0], RecordedLoginEvent::Phase(LoginPhase::Spawning));
    match events.last().unwrap() {
        RecordedLoginEvent::Error(LoginPhase::Spawning, _) => {}
        other => panic!("expected error on Spawning, got {other:?}"),
    }
}

// -- Verify-all progress tests (C-2) --------------------------------

/// Recording sink for VerifyEvent — captures every event in order.
struct RecordingVerifySink {
    events: std::sync::Mutex<Vec<VerifyEvent>>,
}

impl RecordingVerifySink {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn events(&self) -> Vec<VerifyEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl VerifyProgressSink for RecordingVerifySink {
    fn event(&self, ev: VerifyEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

/// `swap::ProfileFetcher` mock — different from the inner
/// `ProfileFetcher` mock used elsewhere in this module. Returns
/// `email_for(token)` so verify can drive different outcomes per
/// account.
struct VerifyFetcher {
    emails: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl VerifyFetcher {
    fn new() -> Self {
        Self {
            emails: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
    fn returns(self, _token_prefix: &str, email: &str) -> Self {
        // Mock all calls to return this email regardless of token.
        self.emails
            .lock()
            .unwrap()
            .insert("any".into(), email.into());
        self
    }
}

#[async_trait::async_trait]
impl crate::cli_backend::swap::ProfileFetcher for VerifyFetcher {
    async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
        self.emails
            .lock()
            .unwrap()
            .get("any")
            .cloned()
            .ok_or_else(|| OAuthError::AuthFailed("no scripted email".into()))
    }
}

#[tokio::test]
async fn test_verify_all_with_progress_emits_started_then_per_account_then_done() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();

    // Two accounts both with credentials and a fresh blob.
    let a1 = insert_account(&store, "alice@example.com");
    swap::save_private(a1.uuid, &fresh_blob_json()).unwrap();
    let a2 = insert_account(&store, "bob@example.com");
    swap::save_private(a2.uuid, &fresh_blob_json()).unwrap();

    // Simple fetcher that returns "alice@example.com" for every call —
    // a1 is Ok, a2 is Drift (label "bob" vs server "alice").
    let fetcher = VerifyFetcher::new().returns("any", "alice@example.com");

    let sink = RecordingVerifySink::new();
    verify_all_with_progress(&store, &fetcher, &sink).await;

    let events = sink.events();
    // First event is Started { total: 2 }.
    match &events[0] {
        VerifyEvent::Started { total } => assert_eq!(*total, 2),
        other => panic!("expected Started, got {other:?}"),
    }
    // Last event is Done.
    assert!(matches!(events.last(), Some(VerifyEvent::Done)));

    // Two Account events in between, indices 1 and 2.
    let account_events: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            VerifyEvent::Account { idx, total, outcome, .. } => Some((*idx, *total, *outcome)),
            _ => None,
        })
        .collect();
    assert_eq!(account_events.len(), 2);
    assert_eq!(account_events[0].0, 1);
    assert_eq!(account_events[0].1, 2);
    assert_eq!(account_events[1].0, 2);
    assert_eq!(account_events[1].1, 2);

    // Cleanup — drop the per-account credential files so the data
    // dir lock isn't polluted for siblings.
    let _ = swap::delete_private(a1.uuid);
    let _ = swap::delete_private(a2.uuid);
}

#[tokio::test]
async fn test_verify_all_with_progress_skips_accounts_without_credentials() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();

    // alice has credentials.
    let a1 = insert_account(&store, "alice@example.com");
    swap::save_private(a1.uuid, &fresh_blob_json()).unwrap();
    // bob does NOT — flip the flag explicitly. No swap::save_private
    // here so the blob is genuinely absent on disk too.
    let mut acc = make_account("nocreds@example.com");
    acc.has_cli_credentials = false;
    store.insert(&acc).unwrap();

    let fetcher = VerifyFetcher::new().returns("any", "alice@example.com");
    let sink = RecordingVerifySink::new();
    verify_all_with_progress(&store, &fetcher, &sink).await;

    let events = sink.events();
    match &events[0] {
        // Only `alice` is eligible — `nocreds` was filtered out.
        VerifyEvent::Started { total } => assert_eq!(*total, 1),
        other => panic!("expected Started {{ total: 1 }}, got {other:?}"),
    }
    let account_count = events
        .iter()
        .filter(|e| matches!(e, VerifyEvent::Account { .. }))
        .count();
    assert_eq!(account_count, 1);

    // Cleanup
    for a in store.list().unwrap() {
        let _ = swap::delete_private(a.uuid);
    }
}

/// Stagger only fires BETWEEN calls — not before the first or after
/// the last. With N=3 accounts the total elapsed time should be
/// >= 2 * 200ms but the first event fires immediately.
#[tokio::test]
async fn test_verify_all_with_progress_uses_200ms_stagger_only_between_calls() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();

    let a1 = insert_account(&store, "a1@example.com");
    swap::save_private(a1.uuid, &fresh_blob_json()).unwrap();
    let a2 = insert_account(&store, "a2@example.com");
    swap::save_private(a2.uuid, &fresh_blob_json()).unwrap();
    let a3 = insert_account(&store, "a3@example.com");
    swap::save_private(a3.uuid, &fresh_blob_json()).unwrap();

    let fetcher = VerifyFetcher::new().returns("any", "a1@example.com");
    let sink = RecordingVerifySink::new();

    let start = std::time::Instant::now();
    verify_all_with_progress(&store, &fetcher, &sink).await;
    let elapsed = start.elapsed();

    // Two stagger gaps for 3 accounts → >= 400ms minimum.
    assert!(
        elapsed >= std::time::Duration::from_millis(380),
        "stagger should add ~400ms; elapsed={elapsed:?}"
    );
    // Sanity upper bound — we shouldn't sleep before the first or
    // after the last (would push elapsed above 600ms+jitter).
    assert!(
        elapsed < std::time::Duration::from_millis(900),
        "no extra stagger before first / after last; elapsed={elapsed:?}"
    );

    // Cleanup
    for a in store.list().unwrap() {
        let _ = swap::delete_private(a.uuid);
    }
}
