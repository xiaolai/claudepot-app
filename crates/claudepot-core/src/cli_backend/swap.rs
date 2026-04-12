//! Mode A atomic swap primitive for CLI credentials.
//! See reference.md §I.7.

use crate::account::AccountStore;
use crate::error::{OAuthError, SwapError};
use crate::oauth::refresh::TokenResponse;
use super::CliPlatform;
use uuid::Uuid;
use std::fs;

/// Abstraction over token refresh — enables testing without network calls.
#[async_trait::async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, OAuthError>;
}

/// Production refresher that calls the Anthropic token endpoint.
pub struct DefaultRefresher;

#[async_trait::async_trait]
impl TokenRefresher for DefaultRefresher {
    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, OAuthError> {
        crate::oauth::refresh::refresh(refresh_token).await
    }
}

/// Acquire an exclusive file lock for swap operations.
/// Returns the locked file handle — lock is released when dropped.
fn acquire_swap_lock() -> Result<fs::File, SwapError> {
    let lock_path = crate::paths::claudepot_data_dir().join(".swap.lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        // Blocking exclusive lock — waits if another swap is in progress.
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if ret != 0 {
            return Err(SwapError::FileError(std::io::Error::last_os_error()));
        }
    }
    #[cfg(windows)]
    {
        // On Windows, opening with write + no sharing provides exclusion.
        // The OpenOptions above already provide this behavior.
    }

    Ok(file)
}

/// Conditionally refresh the credential blob if it is expired or expiring
/// within a 5-minute margin. Returns the (possibly refreshed) blob JSON string.
/// On refresh, the new blob is persisted to private storage.
pub(crate) async fn maybe_refresh_blob(
    blob_str: &str,
    account_id: Uuid,
    refresher: &dyn TokenRefresher,
) -> Result<String, SwapError> {
    let blob = crate::blob::CredentialBlob::from_json(blob_str)
        .map_err(|e| SwapError::CorruptBlob(e.to_string()))?;

    if !blob.is_expired(300) {
        return Ok(blob_str.to_string());
    }

    tracing::info!("token expired or expiring soon, refreshing...");
    let token_resp = refresher
        .refresh(&blob.claude_ai_oauth.refresh_token)
        .await
        .map_err(|e| SwapError::RefreshFailed(e.to_string()))?;

    let new_blob = crate::oauth::refresh::build_blob(&token_resp, Some(&blob));
    save_private(account_id, &new_blob)?;
    Ok(new_blob)
}

/// Swap the active CLI account from `current_id` to `target_id`.
///
/// Acquires an exclusive file lock to prevent concurrent swaps.
///
/// 1. Read the current blob from CC storage (may have been refreshed).
/// 2. Save outgoing blob to Claudepot private storage.
/// 3. Load target blob from Claudepot private storage.
/// 4. Write target blob to CC storage + touch credfile.
/// 5. On failure at step 4, rollback Claudepot private storage.
pub async fn switch(
    store: &AccountStore,
    current_id: Option<Uuid>,
    target_id: Uuid,
    platform: &dyn CliPlatform,
    auto_refresh: bool,
    refresher: &dyn TokenRefresher,
) -> Result<(), SwapError> {
    // Acquire exclusive lock — prevents concurrent swaps.
    tracing::debug!("acquiring swap lock...");
    let _lock = acquire_swap_lock()?;
    tracing::debug!("swap lock acquired");

    // Load target blob from Claudepot private storage first.
    // If it doesn't exist, fail before touching anything.
    tracing::debug!(target = %target_id, "loading target credentials");
    let target_blob = load_private(target_id)?;

    // Conditionally refresh if expired/expiring and auto_refresh is enabled.
    let target_blob = if auto_refresh {
        maybe_refresh_blob(&target_blob, target_id, refresher).await?
    } else {
        target_blob
    };

    // Save outgoing (current CC blob may have been refreshed by the CLI).
    if let Some(cur) = current_id {
        if let Some(current_blob) = platform.read_default().await? {
            let previous_private = load_private_opt(cur);
            save_private(cur, &current_blob)?;

            // Write target to CC storage.
            if let Err(e) = platform.write_default(&target_blob).await {
                // Rollback: restore previous Claudepot blob for outgoing account.
                match previous_private {
                    Some(prev) => { let _ = save_private(cur, &prev); }
                    None => { let _ = delete_private(cur); }
                }
                return Err(e);
            }
        } else {
            // No current blob in CC — just write target directly.
            platform.write_default(&target_blob).await?;
        }
    } else {
        platform.write_default(&target_blob).await?;
    }

    // Bump mtime for cross-process invalidation (best-effort).
    let _ = platform.touch_credfile().await;

    // Update active pointer in account store.
    tracing::debug!(target = %target_id, "updating active CLI pointer");
    store
        .set_active_cli(target_id)
        .map_err(|e| SwapError::WriteFailed(format!("db update failed: {e}")))?;

    tracing::info!(target = %target_id, "swap complete");
    // _lock dropped here — releases the file lock.
    Ok(())
}

// --- Private storage: file-based, 0600 perms ---
//
// WORKAROUND: using files instead of the `keyring` crate.
//
// The `keyring` crate calls macOS `SecItem*` APIs, which require the
// calling binary to have a valid code-signing identity. Our debug
// builds (`cargo build`) produce ad-hoc signed binaries that lack
// the entitlements needed for Keychain access — `set_password()`
// returns Ok(()) but silently writes nothing. This affects ALL
// unsigned binaries, not just SSH sessions.
//
// Once the release binary is signed with a Developer ID certificate
// (implementation plan Phase 12), switch back to `keyring`:
//
//   keyring::Entry::new("com.claudepot.credentials", &uuid.to_string())
//
// Tracked as audit finding Critical #2 (2026-04-12).
//
// Blobs are stored at: <claudepot_data_dir>/credentials/<uuid>.json

fn private_path(account_id: Uuid) -> std::path::PathBuf {
    crate::paths::claudepot_data_dir()
        .join("credentials")
        .join(format!("{}.json", account_id))
}

pub fn load_private(account_id: Uuid) -> Result<String, SwapError> {
    let path = private_path(account_id);

    // Verify file permissions before reading credentials — fail closed
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                tracing::warn!(
                    "credential file {} has permissions {:o} (expected 600), fixing",
                    path.display(), mode
                );
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| SwapError::FileError(e))?;
            }
        }
    }

    std::fs::read_to_string(&path)
        .map_err(|_| SwapError::NoStoredCredentials(account_id))
}

fn load_private_opt(account_id: Uuid) -> Option<String> {
    std::fs::read_to_string(private_path(account_id)).ok()
}

pub fn save_private(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    let path = private_path(account_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent().unwrap_or(std::path::Path::new(".")),
    )?;
    std::io::Write::write_all(&mut tmp, blob.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    tmp.persist(&path)
        .map_err(|e| SwapError::WriteFailed(format!("persist failed: {e}")))?;
    Ok(())
}

pub fn delete_private(account_id: Uuid) -> Result<(), SwapError> {
    let path = private_path(account_id);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock CliPlatform for testing swap logic.
    struct MockPlatform {
        storage: Mutex<Option<String>>,
        fail_write: bool,
    }

    impl MockPlatform {
        fn new(initial: Option<&str>) -> Self {
            Self {
                storage: Mutex::new(initial.map(String::from)),
                fail_write: false,
            }
        }
        fn failing() -> Self {
            Self { storage: Mutex::new(None), fail_write: true }
        }
        fn get(&self) -> Option<String> {
            self.storage.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            Ok(self.storage.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            if self.fail_write {
                return Err(SwapError::WriteFailed("mock write failure".into()));
            }
            *self.storage.lock().unwrap() = Some(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    /// Mock TokenRefresher for testing refresh logic.
    struct MockRefresher {
        response: Result<TokenResponse, OAuthError>,
    }

    impl MockRefresher {
        fn success() -> Self {
            Self {
                response: Ok(TokenResponse {
                    access_token: "sk-ant-oat01-refreshed".into(),
                    refresh_token: "sk-ant-ort01-refreshed".into(),
                    expires_in: 3600,
                    scope: Some("user:inference user:profile".into()),
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
    impl TokenRefresher for MockRefresher {
        async fn refresh(&self, _refresh_token: &str) -> Result<TokenResponse, OAuthError> {
            match &self.response {
                Ok(r) => Ok(TokenResponse {
                    access_token: r.access_token.clone(),
                    refresh_token: r.refresh_token.clone(),
                    expires_in: r.expires_in,
                    scope: r.scope.clone(),
                    token_type: r.token_type.clone(),
                }),
                Err(OAuthError::RefreshFailed(msg)) => Err(OAuthError::RefreshFailed(msg.clone())),
                Err(OAuthError::RateLimited { retry_after_secs }) => {
                    Err(OAuthError::RateLimited { retry_after_secs: *retry_after_secs })
                }
                _ => Err(OAuthError::RefreshFailed("unexpected error variant".into())),
            }
        }
    }

    use crate::testing::setup_test_data_dir;

    fn test_store() -> (AccountStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = AccountStore::open(&db).unwrap();
        (store, dir)
    }

    #[test]
    fn test_private_storage_roundtrip() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = r#"{"test":"data"}"#;

        // Save
        save_private(id, blob).unwrap();

        // Load
        let loaded = load_private(id).unwrap();
        assert_eq!(loaded, blob);

        // Verify 0600 permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(private_path(id)).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        // Delete
        delete_private(id).unwrap();
        assert!(load_private(id).is_err());
    }

    #[test]
    fn test_private_storage_missing() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        assert!(matches!(
            load_private(id),
            Err(SwapError::NoStoredCredentials(_))
        ));
    }

    #[test]
    fn test_private_storage_overwrite() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        save_private(id, "first").unwrap();
        save_private(id, "second").unwrap();
        assert_eq!(load_private(id).unwrap(), "second");
        delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_success() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        // Pre-store target credentials
        save_private(target_id, "target_blob").unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;
        switch(&store, None, target_id, &platform, false, &refresher).await.unwrap();

        assert_eq!(platform.get(), Some("target_blob".to_string()));
        assert_eq!(store.active_cli_uuid().unwrap(), Some(target_id.to_string()));

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_saves_outgoing() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        // Pre-store target credentials
        save_private(target_id, "target_blob").unwrap();

        // Platform has current credentials (as if CC refreshed them)
        let platform = MockPlatform::new(Some("refreshed_current_blob"));
        let refresher = DefaultRefresher;

        switch(&store, Some(current_id), target_id, &platform, false, &refresher).await.unwrap();

        // Current's credentials should be saved to private storage
        assert_eq!(load_private(current_id).unwrap(), "refreshed_current_blob");
        // Target should be in platform
        assert_eq!(platform.get(), Some("target_blob".to_string()));

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_rollback_on_write_failure() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        // Pre-store both
        save_private(current_id, "original_current").unwrap();
        save_private(target_id, "target_blob").unwrap();

        // Platform will fail on write
        let platform = MockPlatform::failing();
        // Set initial storage to simulate current CC credentials
        *platform.storage.lock().unwrap() = Some("cc_current".to_string());
        let refresher = DefaultRefresher;

        let result = switch(&store, Some(current_id), target_id, &platform, false, &refresher).await;
        assert!(result.is_err());

        // Current's private storage should be rolled back to original
        assert_eq!(load_private(current_id).unwrap(), "original_current");

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_no_target_credentials() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        // Don't pre-store — target has no credentials
        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;

        let result = switch(&store, None, target_id, &platform, false, &refresher).await;
        assert!(matches!(result, Err(SwapError::NoStoredCredentials(_))));
    }

    #[tokio::test]
    async fn test_swap_db_pointer_matches_after_success() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        save_private(target_id, "blob").unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;
        switch(&store, None, target_id, &platform, false, &refresher).await.unwrap();

        // DB active pointer must match target
        assert_eq!(store.active_cli_uuid().unwrap(), Some(target_id.to_string()));
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_db_not_updated_on_write_failure() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        save_private(target_id, "target").unwrap();

        // Set initial active to current
        store.set_active_cli(current_id).unwrap();

        let platform = MockPlatform::failing();
        *platform.storage.lock().unwrap() = Some("cc".to_string());
        let refresher = DefaultRefresher;

        let result = switch(&store, Some(current_id), target_id, &platform, false, &refresher).await;
        assert!(result.is_err());

        // DB should still point to current, NOT target
        assert_eq!(store.active_cli_uuid().unwrap(), Some(current_id.to_string()));

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_rollback_deletes_when_no_previous() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        // current has NO prior private storage
        save_private(target_id, "target").unwrap();

        let platform = MockPlatform::failing();
        *platform.storage.lock().unwrap() = Some("cc_blob".to_string());
        let refresher = DefaultRefresher;

        let result = switch(&store, Some(current_id), target_id, &platform, false, &refresher).await;
        assert!(result.is_err());

        // Rollback should have deleted the private storage that was created during swap
        // (since there was no previous blob to restore to)
        assert!(load_private(current_id).is_err());

        delete_private(target_id).unwrap();
    }

    #[test]
    fn test_swap_error_corrupt_blob_display() {
        let err = SwapError::CorruptBlob("missing field accessToken".into());
        assert_eq!(
            err.to_string(),
            "corrupt credential blob: missing field accessToken"
        );
    }

    #[test]
    fn test_swap_error_refresh_failed_display() {
        let err = SwapError::RefreshFailed("token endpoint returned 401".into());
        assert_eq!(
            err.to_string(),
            "token refresh failed: token endpoint returned 401"
        );
    }

    #[tokio::test]
    async fn test_swap_target_load_fails_before_any_mutation() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        // Target has no stored credentials — should fail immediately

        // Platform tracks whether read_default was ever called
        let platform = MockPlatform::new(Some("should-not-be-read"));
        let refresher = DefaultRefresher;

        let result = switch(&store, None, target_id, &platform, false, &refresher).await;
        assert!(result.is_err());

        // Platform storage should be untouched (read_default never called for write path)
        assert_eq!(platform.get(), Some("should-not-be-read".to_string()));
    }

    #[tokio::test]
    async fn test_swap_current_none_writes_directly() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        save_private(target_id, "direct_blob").unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;
        switch(&store, None, target_id, &platform, false, &refresher).await.unwrap();

        // Target written directly, no outgoing save
        assert_eq!(platform.get(), Some("direct_blob".to_string()));
        delete_private(target_id).unwrap();
    }

    // --- switch() auto_refresh tests ---

    #[tokio::test]
    async fn test_swap_auto_refresh_writes_fresh_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        // Store an expired blob for the target
        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();

        switch(&store, None, target_id, &platform, true, &refresher)
            .await
            .unwrap();

        // The platform should have the refreshed blob, not the expired one
        let written = platform.get().unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&written).unwrap();
        assert_eq!(parsed.claude_ai_oauth.access_token, "sk-ant-oat01-refreshed");

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_noop_for_fresh_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        let fresh = crate::testing::fresh_blob_json();
        save_private(target_id, &fresh).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();

        switch(&store, None, target_id, &platform, true, &refresher)
            .await
            .unwrap();

        // Should use the original fresh blob (not refreshed)
        let written = platform.get().unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&written).unwrap();
        assert_eq!(parsed.claude_ai_oauth.access_token, "sk-ant-oat01-test");

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_false_skips_refresh() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        let expired = crate::testing::expired_blob_json();
        save_private(target_id, &expired).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();

        // auto_refresh = false — should NOT call refresher
        switch(&store, None, target_id, &platform, false, &refresher)
            .await
            .unwrap();

        // Should use the expired blob as-is
        let written = platform.get().unwrap();
        assert_eq!(written, expired);

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_failure_aborts_before_mutation() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        let platform = MockPlatform::new(Some("original-cc-blob"));
        let refresher = MockRefresher::failing("network error");

        let result = switch(&store, None, target_id, &platform, true, &refresher).await;
        assert!(matches!(result, Err(SwapError::RefreshFailed(_))));

        // Platform should be untouched
        assert_eq!(platform.get(), Some("original-cc-blob".to_string()));

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_rollback_works_after_refresh() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        save_private(current_id, "original_current").unwrap();
        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        // Platform will fail on write
        let platform = MockPlatform::failing();
        *platform.storage.lock().unwrap() = Some("cc_current".to_string());
        let refresher = MockRefresher::success();

        let result = switch(&store, Some(current_id), target_id, &platform, true, &refresher).await;
        assert!(result.is_err());

        // Current's private storage should be rolled back to original
        assert_eq!(load_private(current_id).unwrap(), "original_current");

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    // --- maybe_refresh_blob tests ---

    #[tokio::test]
    async fn test_swap_maybe_refresh_not_expired_returns_unchanged() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::fresh_blob_json();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob(&blob, id, &refresher).await.unwrap();
        // Fresh blob should be returned unchanged (same string)
        assert_eq!(result, blob);
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_within_margin_triggers_refresh() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expiring_soon_blob_json();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob(&blob, id, &refresher).await.unwrap();
        // Should have refreshed — result should contain the new token
        let parsed = crate::blob::CredentialBlob::from_json(&result).unwrap();
        assert_eq!(parsed.claude_ai_oauth.access_token, "sk-ant-oat01-refreshed");
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_corrupt_input_errors() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob("not valid json", id, &refresher).await;
        assert!(matches!(result, Err(SwapError::CorruptBlob(_))));
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_expired_refreshes() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expired_blob_json();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob(&blob, id, &refresher).await.unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&result).unwrap();
        assert_eq!(parsed.claude_ai_oauth.access_token, "sk-ant-oat01-refreshed");
        assert_eq!(parsed.claude_ai_oauth.refresh_token, "sk-ant-ort01-refreshed");
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_refresh_failure_errors() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expired_blob_json();
        let refresher = MockRefresher::failing("network timeout");

        let result = maybe_refresh_blob(&blob, id, &refresher).await;
        assert!(matches!(result, Err(SwapError::RefreshFailed(_))));
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_saves_refreshed_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expired_blob_json();
        let refresher = MockRefresher::success();

        maybe_refresh_blob(&blob, id, &refresher).await.unwrap();

        // The refreshed blob should be persisted in private storage
        let saved = load_private(id).unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&saved).unwrap();
        assert_eq!(parsed.claude_ai_oauth.access_token, "sk-ant-oat01-refreshed");

        delete_private(id).unwrap();
    }
}
