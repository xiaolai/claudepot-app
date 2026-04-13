//! Account registration, removal, and inspection — core business logic.
//! CLI and Tauri both call these functions.

use crate::account::{Account, AccountStore};
use crate::blob::CredentialBlob;
use crate::cli_backend;
use crate::cli_backend::swap;
use crate::oauth::{profile, usage};
use crate::paths;
use chrono::Utc;
use uuid::Uuid;

/// Result of registering a new account.
#[derive(Debug)]
pub struct RegisterResult {
    pub uuid: Uuid,
    pub email: String,
    pub org_name: String,
    pub subscription_type: String,
    pub rate_limit_tier: Option<String>,
}

/// Register an account by importing the current CC credentials.
pub async fn register_from_current(store: &AccountStore) -> Result<RegisterResult, RegisterError> {
    let platform = cli_backend::create_platform();
    register_from_current_with(store, platform.as_ref(), &DefaultProfileFetcher).await
}

/// Testable variant: accepts injectable platform and profile fetcher.
pub(crate) async fn register_from_current_with(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
) -> Result<RegisterResult, RegisterError> {
    tracing::info!("registering from current CC credentials");
    let blob_str = platform
        .read_default()
        .await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?
        .ok_or(RegisterError::NoCredentials)?;

    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?;

    let prof = fetch_profile
        .fetch(&blob.claude_ai_oauth.access_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    register_account_from_profile(store, &blob_str, &prof)
}

/// Trait for fetching an OAuth profile — enables testing without network.
#[async_trait::async_trait]
pub(crate) trait ProfileFetcher: Send + Sync {
    async fn fetch(&self, access_token: &str)
        -> Result<profile::Profile, crate::error::OAuthError>;
}

/// Production implementation that calls the Anthropic API.
struct DefaultProfileFetcher;

#[async_trait::async_trait]
impl ProfileFetcher for DefaultProfileFetcher {
    async fn fetch(
        &self,
        access_token: &str,
    ) -> Result<profile::Profile, crate::error::OAuthError> {
        profile::fetch(access_token).await
    }
}

/// Shared logic: given a credential blob string and a fetched profile,
/// save credentials and insert the account.
fn register_account_from_profile(
    store: &AccountStore,
    blob_str: &str,
    prof: &profile::Profile,
) -> Result<RegisterResult, RegisterError> {
    if let Some(existing) = store
        .find_by_email(&prof.email)
        .map_err(|e| RegisterError::Store(e.to_string()))?
    {
        return Err(RegisterError::AlreadyRegistered(
            existing.email,
            existing.uuid,
        ));
    }

    let account_id = Uuid::new_v4();
    swap::save_private(account_id, blob_str)
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    if let Err(e) = store.insert(&account) {
        // Rollback: delete orphaned private blob
        let _ = swap::delete_private(account_id);
        return Err(RegisterError::Store(e.to_string()));
    }

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email.clone(),
        org_name: prof.org_name.clone(),
        subscription_type: prof.subscription_type.clone(),
        rate_limit_tier: prof.rate_limit_tier.clone(),
    })
}

/// Register an account from a refresh token (headless).
pub async fn register_from_token(
    store: &AccountStore,
    refresh_token: &str,
) -> Result<RegisterResult, RegisterError> {
    use crate::cli_backend::swap::DefaultRefresher;
    register_from_token_with(
        store,
        refresh_token,
        &DefaultRefresher,
        &DefaultProfileFetcher,
    )
    .await
}

/// Testable variant: accepts injectable refresher and profile fetcher.
pub(crate) async fn register_from_token_with(
    store: &AccountStore,
    refresh_token: &str,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
    fetch_profile: &dyn ProfileFetcher,
) -> Result<RegisterResult, RegisterError> {
    use crate::oauth::refresh;

    let token_resp = refresher
        .refresh(refresh_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(format!("token exchange failed: {e}")))?;

    let prof = fetch_profile
        .fetch(&token_resp.access_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    let blob_str = refresh::build_blob(&token_resp, None);
    register_account_from_profile(store, &blob_str, &prof)
}

/// Register an account via browser-based OAuth login.
/// Runs `claude auth login` in a temp config dir, reads credentials,
/// fetches profile, and registers the account.
pub async fn register_from_browser(store: &AccountStore) -> Result<RegisterResult, RegisterError> {
    use crate::onboard;

    let config_dir = onboard::run_auth_login()
        .await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?;

    let blob_str = match onboard::read_credentials_from_dir(&config_dir).await {
        Ok(b) => b,
        Err(e) => {
            onboard::cleanup(&config_dir).await;
            return Err(RegisterError::CredentialRead(e.to_string()));
        }
    };

    let blob = CredentialBlob::from_json(&blob_str).map_err(|e| {
        // Fire-and-forget cleanup — don't propagate cleanup errors
        let config_dir = config_dir.clone();
        tokio::spawn(async move { onboard::cleanup(&config_dir).await });
        RegisterError::CredentialRead(e.to_string())
    })?;

    let prof = match profile::fetch(&blob.claude_ai_oauth.access_token).await {
        Ok(p) => p,
        Err(e) => {
            onboard::cleanup(&config_dir).await;
            return Err(RegisterError::ProfileFetch(e.to_string()));
        }
    };

    if let Some(existing) = store
        .find_by_email(&prof.email)
        .map_err(|e| RegisterError::Store(e.to_string()))?
    {
        onboard::cleanup(&config_dir).await;
        return Err(RegisterError::AlreadyRegistered(
            existing.email,
            existing.uuid,
        ));
    }

    let account_id = Uuid::new_v4();
    swap::save_private(account_id, &blob_str).map_err(|e| {
        // Cleanup on credential write failure
        let cd = config_dir.clone();
        tokio::spawn(async move { onboard::cleanup(&cd).await });
        RegisterError::CredentialWrite(e.to_string())
    })?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    if let Err(e) = store.insert(&account) {
        // Rollback: delete orphaned private blob + cleanup temp dir
        let _ = swap::delete_private(account_id);
        onboard::cleanup(&config_dir).await;
        return Err(RegisterError::Store(e.to_string()));
    }
    onboard::cleanup(&config_dir).await;

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email,
        org_name: prof.org_name,
        subscription_type: prof.subscription_type,
        rate_limit_tier: prof.rate_limit_tier,
    })
}

/// Remove an account and all its associated data.
/// Collects non-fatal warnings instead of silently swallowing errors.
pub fn remove_account(store: &AccountStore, uuid: Uuid) -> Result<RemoveResult, RegisterError> {
    let account = store
        .find_by_uuid(uuid)
        .map_err(|e| RegisterError::Store(e.to_string()))?
        .ok_or(RegisterError::NotFound)?;

    let mut warnings: Vec<String> = Vec::new();

    let profile_dir = paths::desktop_profile_dir(uuid);
    let had_profile = profile_dir.exists();

    // Clear active pointers first (reversible DB operations)
    if account.is_cli_active {
        if let Err(e) = store.clear_active_cli() {
            warnings.push(format!("failed to clear active CLI pointer: {e}"));
        }
    }
    if account.is_desktop_active {
        if let Err(e) = store.clear_active_desktop() {
            warnings.push(format!("failed to clear active Desktop pointer: {e}"));
        }
    }

    // Remove from DB before irreversible file deletions
    store
        .remove(uuid)
        .map_err(|e| RegisterError::Store(e.to_string()))?;

    // Now safe to delete files — DB row is already gone
    if let Err(e) = swap::delete_private(uuid) {
        warnings.push(format!("failed to delete credential file: {e}"));
    }
    if had_profile {
        if let Err(e) = std::fs::remove_dir_all(&profile_dir) {
            warnings.push(format!("failed to delete desktop profile: {e}"));
        }
    }

    Ok(RemoveResult {
        email: account.email,
        was_cli_active: account.is_cli_active,
        was_desktop_active: account.is_desktop_active,
        had_desktop_profile: had_profile,
        warnings,
    })
}

#[derive(Debug)]
pub struct RemoveResult {
    pub email: String,
    pub was_cli_active: bool,
    pub was_desktop_active: bool,
    pub had_desktop_profile: bool,
    pub warnings: Vec<String>,
}

/// Token health info for an account.
#[derive(Debug)]
pub struct TokenHealth {
    pub status: String,
    pub remaining_mins: Option<i64>,
}

/// Get token health for an account.
pub fn token_health(uuid: Uuid, has_credentials: bool) -> TokenHealth {
    if !has_credentials {
        return TokenHealth {
            status: "no credentials".into(),
            remaining_mins: None,
        };
    }
    match swap::load_private(uuid) {
        Ok(blob_str) => match CredentialBlob::from_json(&blob_str) {
            Ok(blob) => {
                let remaining =
                    (blob.claude_ai_oauth.expires_at - Utc::now().timestamp_millis()) / 60_000;
                if remaining > 0 {
                    TokenHealth {
                        status: format!("valid ({}m remaining)", remaining),
                        remaining_mins: Some(remaining),
                    }
                } else {
                    TokenHealth {
                        status: "expired".into(),
                        remaining_mins: Some(remaining),
                    }
                }
            }
            Err(_) => TokenHealth {
                status: "corrupt blob".into(),
                remaining_mins: None,
            },
        },
        Err(_) => TokenHealth {
            status: "missing".into(),
            remaining_mins: None,
        },
    }
}

/// Fetch live usage for an account (returns None if token expired or missing).
pub async fn fetch_usage(uuid: Uuid) -> Option<usage::UsageResponse> {
    let blob_str = swap::load_private(uuid).ok()?;
    let blob = CredentialBlob::from_json(&blob_str).ok()?;
    if blob.is_expired(0) {
        return None;
    }
    usage::fetch(&blob.claude_ai_oauth.access_token).await.ok()
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("no CC credentials found — log in with `claude auth login` first")]
    NoCredentials,
    #[error("failed to read credentials: {0}")]
    CredentialRead(String),
    #[error("failed to write credentials: {0}")]
    CredentialWrite(String),
    #[error("profile fetch failed: {0}")]
    ProfileFetch(String),
    #[error("already registered: {0} (uuid: {1})")]
    AlreadyRegistered(String, Uuid),
    #[error("store error: {0}")]
    Store(String),
    #[error("account not found")]
    NotFound,
}

#[cfg(test)]
mod tests {
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
        blob: Option<String>,
    }

    #[async_trait::async_trait]
    impl cli_backend::CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            Ok(self.blob.clone())
        }
        async fn write_default(&self, _blob: &str) -> Result<(), SwapError> {
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    struct MockProfileFetcher {
        profile: Result<profile::Profile, OAuthError>,
    }

    impl MockProfileFetcher {
        fn ok(email: &str) -> Self {
            Self {
                profile: Ok(profile::Profile {
                    email: email.to_string(),
                    org_uuid: "org-uuid-1".to_string(),
                    org_name: "Test Org".to_string(),
                    subscription_type: "pro".to_string(),
                    rate_limit_tier: Some("default_claude_pro".to_string()),
                    account_uuid: "acc-uuid-1".to_string(),
                    display_name: Some("Test User".to_string()),
                }),
            }
        }
        fn failing(msg: &str) -> Self {
            Self {
                profile: Err(OAuthError::AuthFailed(msg.to_string())),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProfileFetcher for MockProfileFetcher {
        async fn fetch(&self, _access_token: &str) -> Result<profile::Profile, OAuthError> {
            match &self.profile {
                Ok(p) => Ok(p.clone()),
                Err(OAuthError::AuthFailed(msg)) => Err(OAuthError::AuthFailed(msg.clone())),
                Err(OAuthError::RefreshFailed(msg)) => Err(OAuthError::RefreshFailed(msg.clone())),
                _ => Err(OAuthError::AuthFailed("mock error".into())),
            }
        }
    }

    struct MockRefresher {
        response: Result<TokenResponse, OAuthError>,
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
            }
        }
        fn failing(msg: &str) -> Self {
            Self {
                response: Err(OAuthError::RefreshFailed(msg.to_string())),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::cli_backend::swap::TokenRefresher for MockRefresher {
        async fn refresh(&self, _rt: &str) -> Result<TokenResponse, OAuthError> {
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

        let platform = MockPlatform {
            blob: Some(fresh_blob_json()),
        };
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

        let platform = MockPlatform { blob: None };
        let fetcher = MockProfileFetcher::ok("alice@example.com");

        let result = register_from_current_with(&store, &platform, &fetcher).await;
        assert!(matches!(result, Err(RegisterError::NoCredentials)));
    }

    #[tokio::test]
    async fn test_register_from_current_profile_fetch_fails() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let platform = MockPlatform {
            blob: Some(fresh_blob_json()),
        };
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

        let platform = MockPlatform {
            blob: Some(fresh_blob_json()),
        };
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

        let platform = MockPlatform {
            blob: Some("not json".to_string()),
        };
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

    #[test]
    fn test_remove_deletes_credential_file() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "cred@example.com");

        // Save a credential file
        swap::save_private(account.uuid, r#"{"test":"blob"}"#).unwrap();
        assert!(swap::load_private(account.uuid).is_ok());

        remove_account(&store, account.uuid).unwrap();

        // Credential file should be gone
        assert!(swap::load_private(account.uuid).is_err());
    }

    #[test]
    fn test_remove_deletes_desktop_profile() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "desk@example.com");

        // Create desktop profile dir
        let profile_dir = paths::desktop_profile_dir(account.uuid);
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join("config.json"), "cfg").unwrap();

        let result = remove_account(&store, account.uuid).unwrap();
        assert!(result.had_desktop_profile);
        assert!(!profile_dir.exists());
    }

    #[test]
    fn test_remove_removes_from_db() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "db@example.com");

        remove_account(&store, account.uuid).unwrap();
        assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
    }

    #[test]
    fn test_remove_clears_active_cli() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "cli@example.com");
        store.set_active_cli(account.uuid).unwrap();

        let result = remove_account(&store, account.uuid).unwrap();
        assert!(result.was_cli_active);
        assert!(store.active_cli_uuid().unwrap().is_none());
    }

    #[test]
    fn test_remove_clears_active_desktop() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "desk2@example.com");
        store.set_active_desktop(account.uuid).unwrap();

        let result = remove_account(&store, account.uuid).unwrap();
        assert!(result.was_desktop_active);
        assert!(store.active_desktop_uuid().unwrap().is_none());
    }

    #[test]
    fn test_remove_nonexistent_returns_not_found() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();

        let result = remove_account(&store, Uuid::new_v4());
        assert!(matches!(result, Err(RegisterError::NotFound)));
    }

    #[test]
    fn test_remove_missing_credential_succeeds_silently() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "warn@example.com");
        // Do NOT save_private — credential file doesn't exist

        let result = remove_account(&store, account.uuid).unwrap();
        // delete_private returns Ok when file doesn't exist,
        // so no warning is produced — this is correct behavior
        assert!(result.warnings.is_empty());
        // Account still removed from DB
        assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
    }

    #[test]
    fn test_remove_returns_correct_metadata() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "meta@example.com");

        let result = remove_account(&store, account.uuid).unwrap();
        assert_eq!(result.email, "meta@example.com");
        assert!(!result.was_cli_active);
        assert!(!result.was_desktop_active);
        assert!(!result.had_desktop_profile);
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

        let platform = MockPlatform {
            blob: Some(fresh_blob_json()),
        };
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

    #[test]
    fn test_remove_account_preserves_files_on_db_failure() {
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

        let result = remove_account(&store, account.uuid);
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

    #[test]
    fn test_remove_account_clears_pointers_before_db_remove() {
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

        let result = remove_account(&store, account.uuid).unwrap();
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
}
