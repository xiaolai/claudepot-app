//! Inline test module for `swap.rs`. Lives in this sibling file
//! so `swap.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "swap_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

// Tests hold `lock_data_dir()` (a `Mutex<()>`) across `.await` to
// serialize the shared `CLAUDEPOT_DATA_DIR` env-var across the test
// binary. The `await_holding_lock` lint flags it but the lock is
// single-threaded and never contended in a deadlock-inducing way.
// Mocks build patches via `Default::default()` then assign fields —
// readable for tests even if a struct literal would suit production.
#![allow(clippy::await_holding_lock, clippy::field_reassign_with_default)]

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
        Self {
            storage: Mutex::new(None),
            fail_write: true,
        }
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
            Err(OAuthError::RateLimited { retry_after_secs }) => Err(OAuthError::RateLimited {
                retry_after_secs: *retry_after_secs,
            }),
            _ => Err(OAuthError::RefreshFailed("unexpected error variant".into())),
        }
    }
}

use crate::testing::{make_account, setup_test_data_dir};

/// Insert a placeholder account for `uuid` so set_active_cli's strict
/// zero-row check doesn't fail in tests that only care about swap mechanics.
fn seed_account(store: &super::AccountStore, uuid: Uuid) {
    let mut a = make_account(&format!("seed-{uuid}@example.com"));
    a.uuid = uuid;
    store.insert(&a).unwrap();
}

/// Mock ProfileFetcher — lets tests assert identity-verification behavior
/// without any network calls. Most tests pass a placeholder via
/// `noop_fetcher()`; tests that exercise the verification path configure
/// a specific email to return.
struct MockProfileFetcher {
    email: String,
}

impl MockProfileFetcher {
    fn returning(email: &str) -> Self {
        Self {
            email: email.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl super::ProfileFetcher for MockProfileFetcher {
    async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
        Ok(self.email.clone())
    }
}

/// Placeholder fetcher used by tests that want verify_blob_identity
/// to always succeed against the seeded account. Returns the email
/// that `seed_account` writes for the given uuid, so the identity
/// check matches the store row. Previously `verify_blob_identity`
/// skipped on unparseable blobs and the name "noop_fetcher" fit —
/// after that security-relevant bypass was removed (audit H2),
/// swap tests need a real fetcher paired with valid blob JSON.
fn noop_fetcher() -> MockProfileFetcher {
    // Tests that still pass this accept any email — they assert on
    // swap mechanics, not identity. The generic returning() value
    // is intentionally a string the seeded accounts won't use, so
    // tests that rely on verify-succeeds must switch to
    // `matching_fetcher(uuid)` explicitly.
    MockProfileFetcher::returning("never-called@example.com")
}

/// Fetcher matched to `seed_account(uuid)` — returns the exact
/// email the store row carries so `verify_blob_identity` passes.
fn matching_fetcher(uuid: Uuid) -> MockProfileFetcher {
    MockProfileFetcher::returning(&format!("seed-{uuid}@example.com"))
}

/// Minimal valid CredentialBlob JSON for tests that don't care
/// about token values. expires_at = year 2100 so blob never looks
/// expired. Paired with `matching_fetcher(uuid)` so the identity
/// check passes against the stored account email.
fn test_blob_json() -> String {
    crate::testing::sample_blob_json(4_102_444_800_000)
}

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

// -- Group 7: permission auto-repair on load_private --
#[cfg(unix)]
#[test]
fn test_load_private_permission_repair_succeeds() {
    // Save a private blob (0600), widen to 0644, then load.
    // Expected: load succeeds, perms auto-repaired back to 0600.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let id = Uuid::new_v4();
    save_private(id, "secret-blob").unwrap();

    use std::os::unix::fs::PermissionsExt;
    let path = private_path(id);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644,
        "setup: widened to 0644"
    );

    let loaded = load_private(id).unwrap();
    assert_eq!(loaded, "secret-blob");

    let after = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(after, 0o600, "load_private must auto-repair perms to 0600");

    delete_private(id).unwrap();
}

#[tokio::test]
async fn test_swap_success() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id);

    // Valid blob JSON — post-H2 verify_blob_identity REJECTS
    // unparseable blobs instead of silently accepting them.
    let target_blob = test_blob_json();
    save_private(target_id, &target_blob).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = DefaultRefresher;
    switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        false,
        &refresher,
        &matching_fetcher(target_id),
    )
    .await
    .unwrap();

    assert_eq!(platform.get(), Some(target_blob));
    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(target_id.to_string())
    );

    delete_private(target_id).unwrap();
}

#[tokio::test]
async fn test_swap_saves_outgoing() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let current_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    seed_account(&store, current_id);
    seed_account(&store, target_id);

    // Both blobs must be valid JSON — post-H2, verify_blob_identity
    // rejects unparseable blobs for both target (pre-write) and
    // outgoing backup (skip_backup path). Making the outgoing blob
    // parseable is what exercises the save-outgoing branch.
    let target_blob = test_blob_json();
    let refreshed_current = test_blob_json();
    save_private(target_id, &target_blob).unwrap();

    let platform = MockPlatform::new(Some(&refreshed_current));
    let refresher = DefaultRefresher;

    // Both current and target blobs authenticate as their seeded
    // emails; since target_id != current_id, we need a fetcher that
    // honors whichever token is queried. For this test the simplest
    // is an email-dispatching stub: but since both blobs carry the
    // same fixture token (sample_blob_json's constant), we instead
    // use the matching fetcher for `current_id` so the outgoing
    // backup path verifies and runs. The target verify uses a
    // separate matching_fetcher in the pre-write call — we need
    // one fetcher that returns the right email for each. Since
    // test_blob_json shares the access_token across calls, split
    // the responsibility by using two fetchers is not possible;
    // use `EchoingFetcher` that returns different emails by uuid
    // resolution via the store. Simpler: since `matching_fetcher`
    // returns a single email and the two calls are sequential,
    // run with a fetcher that returns current_email for the first
    // call and target_email for the second. `MockProfileFetcher`
    // doesn't support that, so use a round-robin fetcher helper.
    // Three verify calls in order: (1) pre-write target, (2)
    // outgoing backup against CC's current blob, (3) post-write
    // read-back of target. The target_id blob is shared as the
    // test_blob_json fixture but the fetcher is consulted by
    // sequence, so we return the right email at each step.
    let fetcher = RoundRobinFetcher::new(vec![
        format!("seed-{target_id}@example.com"),  // (1) pre-write
        format!("seed-{current_id}@example.com"), // (2) outgoing
        format!("seed-{target_id}@example.com"),  // (3) post-write
    ]);

    switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &fetcher,
    )
    .await
    .unwrap();

    // Current's credentials should be saved to private storage
    assert_eq!(load_private(current_id).unwrap(), refreshed_current);
    // Target should be in platform
    assert_eq!(platform.get(), Some(target_blob));

    delete_private(current_id).unwrap();
    delete_private(target_id).unwrap();
}

/// Test fetcher that cycles through a fixed list of emails so
/// sequential verify calls can return different identities. Used
/// by tests that exercise both the outgoing-backup identity check
/// and the post-write target identity check in one swap.
struct RoundRobinFetcher {
    emails: Vec<String>,
    idx: std::sync::atomic::AtomicUsize,
}

impl RoundRobinFetcher {
    fn new(emails: Vec<String>) -> Self {
        Self {
            emails,
            idx: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl super::ProfileFetcher for RoundRobinFetcher {
    async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
        let i = self.idx.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.emails[i % self.emails.len()].clone())
    }
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

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
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

    let result = switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
    assert!(matches!(result, Err(SwapError::NoStoredCredentials(_))));
}

#[tokio::test]
async fn test_swap_db_pointer_matches_after_success() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id);
    save_private(target_id, &test_blob_json()).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = DefaultRefresher;
    switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        false,
        &refresher,
        &matching_fetcher(target_id),
    )
    .await
    .unwrap();

    // DB active pointer must match target
    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(target_id.to_string())
    );
    delete_private(target_id).unwrap();
}

#[tokio::test]
async fn test_swap_db_not_updated_on_write_failure() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let current_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    seed_account(&store, current_id);
    seed_account(&store, target_id);
    save_private(target_id, "target").unwrap();

    // Set initial active to current
    store.set_active_cli(current_id).unwrap();

    let platform = MockPlatform::failing();
    *platform.storage.lock().unwrap() = Some("cc".to_string());
    let refresher = DefaultRefresher;

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
    assert!(result.is_err());

    // DB should still point to current, NOT target
    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(current_id.to_string())
    );

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

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
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

    let result = switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
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
    seed_account(&store, target_id);
    let direct_blob = test_blob_json();
    save_private(target_id, &direct_blob).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = DefaultRefresher;
    switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        false,
        &refresher,
        &matching_fetcher(target_id),
    )
    .await
    .unwrap();

    // Target written directly, no outgoing save
    assert_eq!(platform.get(), Some(direct_blob));
    delete_private(target_id).unwrap();
}

// --- switch_force_for_tests() auto_refresh tests ---

#[tokio::test]
async fn test_swap_auto_refresh_writes_fresh_blob() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id);

    // Store an expired blob for the target
    save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = MockRefresher::success();
    let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

    switch_force_for_tests(
        &store, None, target_id, &platform, true, &refresher, &fetcher,
    )
    .await
    .unwrap();

    // The platform should have the refreshed blob, not the expired one
    let written = platform.get().unwrap();
    let parsed = crate::blob::CredentialBlob::from_json(&written).unwrap();
    assert_eq!(
        parsed.claude_ai_oauth.access_token,
        "sk-ant-oat01-refreshed"
    );

    delete_private(target_id).unwrap();
}

#[tokio::test]
async fn test_swap_auto_refresh_noop_for_fresh_blob() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id);

    let fresh = crate::testing::fresh_blob_json();
    save_private(target_id, &fresh).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = MockRefresher::success();
    let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

    switch_force_for_tests(
        &store, None, target_id, &platform, true, &refresher, &fetcher,
    )
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
    seed_account(&store, target_id);

    let expired = crate::testing::expired_blob_json();
    save_private(target_id, &expired).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = MockRefresher::success();
    let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

    // auto_refresh = false — should NOT call refresher
    switch_force_for_tests(
        &store, None, target_id, &platform, false, &refresher, &fetcher,
    )
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

    let result = switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        true,
        &refresher,
        &noop_fetcher(),
    )
    .await;
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

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        true,
        &refresher,
        &noop_fetcher(),
    )
    .await;
    assert!(result.is_err());

    // Current's private storage should be rolled back to original
    assert_eq!(load_private(current_id).unwrap(), "original_current");

    delete_private(current_id).unwrap();
    delete_private(target_id).unwrap();
}

// --- maybe_refresh_blob tests ---
//
// Audit fix for swap.rs:229: maybe_refresh_blob no longer persists
// to disk. It returns a `MaybeRefreshed` indicating whether the
// caller needs to save after verifying identity. The previous
// "saves to disk" test is gone — that contract was the bug; the
// new contract is verified at the switch_inner integration level.

#[tokio::test]
async fn test_swap_maybe_refresh_not_expired_returns_unchanged() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let blob = crate::testing::fresh_blob_json();
    let refresher = MockRefresher::success();

    let result = maybe_refresh_blob(&blob, &refresher).await.unwrap();
    assert_eq!(result, MaybeRefreshed::Unchanged);
}

#[tokio::test]
async fn test_swap_maybe_refresh_within_margin_triggers_refresh() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let blob = crate::testing::expiring_soon_blob_json();
    let refresher = MockRefresher::success();

    let result = maybe_refresh_blob(&blob, &refresher).await.unwrap();
    let MaybeRefreshed::Refreshed { blob: new_blob } = result else {
        panic!("expected Refreshed");
    };
    let parsed = crate::blob::CredentialBlob::from_json(&new_blob).unwrap();
    assert_eq!(
        parsed.claude_ai_oauth.access_token,
        "sk-ant-oat01-refreshed"
    );
}

#[tokio::test]
async fn test_swap_maybe_refresh_corrupt_input_errors() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let refresher = MockRefresher::success();

    let result = maybe_refresh_blob("not valid json", &refresher).await;
    assert!(matches!(result, Err(SwapError::CorruptBlob(_))));
}

#[tokio::test]
async fn test_swap_maybe_refresh_expired_refreshes() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let blob = crate::testing::expired_blob_json();
    let refresher = MockRefresher::success();

    let result = maybe_refresh_blob(&blob, &refresher).await.unwrap();
    let MaybeRefreshed::Refreshed { blob: new_blob } = result else {
        panic!("expected Refreshed");
    };
    let parsed = crate::blob::CredentialBlob::from_json(&new_blob).unwrap();
    assert_eq!(
        parsed.claude_ai_oauth.access_token,
        "sk-ant-oat01-refreshed"
    );
    assert_eq!(
        parsed.claude_ai_oauth.refresh_token,
        "sk-ant-ort01-refreshed"
    );
}

#[tokio::test]
async fn test_swap_maybe_refresh_refresh_failure_errors() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let blob = crate::testing::expired_blob_json();
    let refresher = MockRefresher::failing("network timeout");

    let result = maybe_refresh_blob(&blob, &refresher).await;
    assert!(matches!(result, Err(SwapError::RefreshFailed(_))));
}

#[tokio::test]
async fn test_swap_maybe_refresh_does_not_persist() {
    // Audit-fix regression guard: the function MUST NOT touch disk.
    // Persistence is now the caller's responsibility, gated on the
    // identity-verification step that runs after refresh.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let id = Uuid::new_v4();
    let blob = crate::testing::expired_blob_json();
    let refresher = MockRefresher::success();

    let _ = maybe_refresh_blob(&blob, &refresher).await.unwrap();

    // Nothing was saved under id — the slot is empty.
    assert!(
        load_private(id).is_err(),
        "maybe_refresh_blob must not persist anymore"
    );
}

// -- Group 11: Unix-only code gaps --

#[test]
fn test_save_private_no_permission_check_on_non_unix() {
    // save_private + load_private roundtrip works on every platform.
    // On Unix, perms are verified (0o600). On non-Unix, the whole
    // #[cfg(unix)] block is skipped and data integrity is all that matters.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let id = Uuid::new_v4();
    let blob = r#"{"platform":"agnostic"}"#;

    save_private(id, blob).unwrap();
    let loaded = load_private(id).unwrap();
    assert_eq!(loaded, blob);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(private_path(id))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "unix: enforced 0o600");
    }

    delete_private(id).unwrap();
}

#[test]
fn test_swap_lock_works_on_all_platforms() {
    // acquire_swap_lock() uses flock on Unix, OpenOptions exclusion on
    // Windows. The contract: returns Ok, creates the lock file, and
    // releases on drop. Verify the lock file exists after acquire.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();

    let lock_path = crate::paths::claudepot_data_dir().join(".swap.lock");
    let _guard = acquire_swap_lock().expect("acquire_swap_lock must work on all platforms");
    assert!(lock_path.exists(), "lock file created");
}

// -- Group 4: CLI swap DB rollback (3 tests) --

#[tokio::test]
async fn test_swap_db_failure_restores_cc_credentials() {
    // Drop the state table so set_active_cli() fails AFTER write_default
    // succeeded. The rollback reads load_private(current) — which, at that
    // point, contains the CC blob that was saved during phase 2 (what CC
    // had before the swap). That's the correct "outgoing" blob to restore.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let current_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();

    // Any older private content for current — not what rollback restores.
    save_private(current_id, "older_private_from_prior_swap").unwrap();
    save_private(target_id, "target_blob").unwrap();

    let platform = MockPlatform::new(Some("outgoing_cc_blob"));
    let refresher = DefaultRefresher;

    // Make set_active_cli fail while write_default remains working.
    store.corrupt_state_table_for_test();

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
    assert!(
        matches!(result, Err(SwapError::WriteFailed(_))),
        "expected WriteFailed from DB update, got {:?}",
        result
    );
    // Rollback restored the CC blob that was outgoing at swap start.
    assert_eq!(
        platform.get(),
        Some("outgoing_cc_blob".to_string()),
        "platform must be restored to outgoing CC credentials"
    );

    delete_private(current_id).unwrap();
    delete_private(target_id).unwrap();
}

// -- Identity verification (prevents mis-filed-blob corruption) --

#[tokio::test]
async fn test_swap_aborts_when_target_blob_belongs_to_different_account() {
    // Regression guard for the "wrong blob under wrong UUID" corruption
    // that motivated the verification. The target's stored blob reports
    // a different email than the target's DB record — switch_force_for_tests() must
    // abort with IdentityMismatch before writing CC.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id); // stored email: seed-{uuid}@example.com
    save_private(target_id, &crate::testing::fresh_blob_json()).unwrap();

    let platform = MockPlatform::new(None);
    let refresher = MockRefresher::success();
    // Fetcher claims the blob is for someone else — the divergence case.
    let fetcher = MockProfileFetcher::returning("intruder@example.com");

    let result = switch_force_for_tests(
        &store, None, target_id, &platform, false, &refresher, &fetcher,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(SwapError::IdentityMismatch {
                ref actual_email,
                ..
            }) if actual_email == "intruder@example.com"
        ),
        "expected IdentityMismatch, got {:?}",
        result
    );
    // Platform untouched — abort happened before write_default.
    assert_eq!(platform.get(), None);

    delete_private(target_id).unwrap();
}

#[tokio::test]
async fn test_swap_drift_on_outgoing_blob_skips_backup_but_completes() {
    // When CC's current blob represents a different account than the DB
    // thinks is active (drift from an external `claude auth login`),
    // the swap must NOT abort — that would leave the user stuck. Instead,
    // skip the outgoing backup (so we don't mis-file), but complete the
    // target swap. The target-blob verification still runs and aborts
    // on a real target mismatch.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let current_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    seed_account(&store, current_id);
    seed_account(&store, target_id);

    // Capture once: fresh_blob_json() uses Utc::now() internally, so
    // back-to-back calls return different millisecond timestamps and
    // the assert_eq below would flake ~30% of the time. Reuse the
    // captured JSON for both the target private slot and the CC
    // platform's current blob.
    let target_blob = crate::testing::fresh_blob_json();
    save_private(target_id, &target_blob).unwrap();
    // Pre-existing blob for current (a valid earlier backup). We'll
    // verify it stays UNCHANGED by this swap (we skipped the backup save).
    save_private(current_id, "original-current-backup").unwrap();

    let cc_blob = target_blob.clone();
    let platform = MockPlatform::new(Some(&cc_blob));
    let refresher = MockRefresher::success();

    // Single-valued fetcher: both verifications see target's seeded email.
    // → target check passes; outgoing check sees drift (cur_email differs).
    let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &fetcher,
    )
    .await;

    assert!(
        result.is_ok(),
        "drift case must NOT abort; got {:?}",
        result
    );

    // Platform received the target blob.
    assert_eq!(platform.get(), Some(cc_blob.clone()));
    // Current's private storage was NOT overwritten with the drifted CC
    // blob — still the original backup.
    assert_eq!(
        load_private(current_id).unwrap(),
        "original-current-backup",
        "outgoing backup must be skipped when CC drift is detected"
    );

    delete_private(current_id).unwrap();
    delete_private(target_id).unwrap();
}

#[tokio::test]
async fn test_swap_db_failure_with_no_outgoing() {
    // Full DB corruption (state table dropped) breaks the early
    // find_by_uuid lookup used by identity verification. The swap aborts
    // before touching the platform — safer than the old "write, then
    // fail at set_active_cli" pattern: if we can't verify the DB's
    // expectations, we don't mutate CC's state.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id);
    save_private(target_id, "target_only_blob").unwrap();

    let platform = MockPlatform::new(None);
    let refresher = DefaultRefresher;

    store.corrupt_state_table_for_test();

    let result = switch_force_for_tests(
        &store,
        None,
        target_id,
        &platform,
        false,
        &refresher,
        &noop_fetcher(),
    )
    .await;
    assert!(
        matches!(result, Err(SwapError::WriteFailed(_))),
        "DB failure must surface as WriteFailed, got {:?}",
        result
    );
    // Platform never written to — aborted before the mutation.
    assert_eq!(platform.get(), None);

    delete_private(target_id).unwrap();
}

#[tokio::test]
async fn test_swap_auto_refresh_then_db_failure_does_not_misfile() {
    // Audit-fix-aligned test (swap.rs:229): when auto_refresh runs
    // and a subsequent DB operation fails, the refreshed blob must
    // NOT be persisted to private storage. The previous shape saved
    // the refresh inside `maybe_refresh_blob` BEFORE identity
    // verification, which would cache mis-filed credentials if the
    // refresh somehow returned a different account's tokens.
    //
    // The new contract: refresh-in-memory → verify → persist-only-
    // on-success. A failure path (here, dropping the state table
    // breaks the early `find_by_uuid` call) leaves target's slot
    // holding the ORIGINAL bytes, never a refreshed-but-unverified
    // copy. Losing the refresh is fine — the next genuine CC
    // session will refresh and persist its own copy.
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let current_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();

    save_private(current_id, "older_private_from_prior_swap").unwrap();
    save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

    let platform = MockPlatform::new(Some("outgoing_cc_blob"));
    let refresher = MockRefresher::success();

    store.corrupt_state_table_for_test();

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        true,
        &refresher,
        &noop_fetcher(),
    )
    .await;
    assert!(
        matches!(result, Err(SwapError::WriteFailed(_))),
        "DB failure must surface as WriteFailed, got {:?}",
        result
    );
    // Platform must remain at its pre-swap value (no successful write).
    assert_eq!(platform.get(), Some("outgoing_cc_blob".to_string()));
    // Audit-fix invariant: target's private slot still has the ORIGINAL
    // expired blob, NOT the refreshed one. The refresh happened in
    // memory but was never committed because the DB error short-
    // circuited before the persist step.
    let target_priv = load_private(target_id).unwrap();
    assert!(
        !target_priv.contains("sk-ant-oat01-refreshed"),
        "refreshed token must NOT be persisted on a pre-verify DB failure; got: {target_priv}"
    );

    delete_private(current_id).unwrap();
    delete_private(target_id).unwrap();
}

// ------- Post-switch verification tests (added by audit round 2) -------

/// Post-switch read-back returns None after a write that claimed
/// success — swap must roll back and return IdentityVerificationFailed,
/// not declare the switch complete.
struct MockPlatformDropsOnRead {
    storage: std::sync::Mutex<Option<String>>,
}
impl MockPlatformDropsOnRead {
    fn new(initial: Option<&str>) -> Self {
        Self {
            storage: std::sync::Mutex::new(initial.map(String::from)),
        }
    }
    fn get(&self) -> Option<String> {
        self.storage.lock().unwrap().clone()
    }
}
#[async_trait::async_trait]
impl CliPlatform for MockPlatformDropsOnRead {
    async fn read_default(&self) -> Result<Option<String>, SwapError> {
        // First read (pre-write outgoing-check) returns whatever is
        // in storage. After a write happens, subsequent reads return
        // None as if CC's slot went empty under us.
        Ok(self.storage.lock().unwrap().clone())
    }
    async fn write_default(&self, _blob: &str) -> Result<(), SwapError> {
        *self.storage.lock().unwrap() = None;
        Ok(())
    }
    async fn touch_credfile(&self) -> Result<(), SwapError> {
        Ok(())
    }
    async fn clear_default(&self) -> Result<(), SwapError> {
        *self.storage.lock().unwrap() = None;
        Ok(())
    }
}

#[tokio::test]
async fn test_switch_aborts_on_post_switch_read_returns_none() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let target_id = Uuid::new_v4();
    seed_account(&store, target_id);
    save_private(target_id, &crate::testing::fresh_blob_json()).unwrap();

    let platform = MockPlatformDropsOnRead::new(None);
    let refresher = MockRefresher::success();
    let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

    let result = switch_force_for_tests(
        &store, None, target_id, &platform, false, &refresher, &fetcher,
    )
    .await;

    assert!(
        matches!(result, Err(SwapError::IdentityVerificationFailed(_))),
        "expected IdentityVerificationFailed when post-switch read returns None, got {result:?}"
    );
    // active_cli must NOT have been set on failure.
    assert!(store.active_cli_uuid().unwrap().is_none());
    // Rollback path had no prior blob — platform stays empty.
    assert!(platform.get().is_none());
    delete_private(target_id).unwrap();
}

/// Post-switch read-back returns a DIFFERENT blob than the one we
/// wrote (the fetcher says it authenticates as someone else). Swap
/// must return IdentityMismatch and restore the previous CC blob.
struct MockPlatformReadReplacesBlob {
    storage: std::sync::Mutex<Option<String>>,
    replace_on_read_after_write: std::sync::Mutex<Option<String>>,
    wrote: std::sync::Mutex<bool>,
}
impl MockPlatformReadReplacesBlob {
    fn new(initial: &str, replace_with: &str) -> Self {
        Self {
            storage: std::sync::Mutex::new(Some(initial.to_string())),
            replace_on_read_after_write: std::sync::Mutex::new(Some(replace_with.to_string())),
            wrote: std::sync::Mutex::new(false),
        }
    }
    fn get(&self) -> Option<String> {
        self.storage.lock().unwrap().clone()
    }
}
#[async_trait::async_trait]
impl CliPlatform for MockPlatformReadReplacesBlob {
    async fn read_default(&self) -> Result<Option<String>, SwapError> {
        if *self.wrote.lock().unwrap() {
            if let Some(r) = self.replace_on_read_after_write.lock().unwrap().take() {
                *self.storage.lock().unwrap() = Some(r);
            }
        }
        Ok(self.storage.lock().unwrap().clone())
    }
    async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
        *self.storage.lock().unwrap() = Some(blob.to_string());
        *self.wrote.lock().unwrap() = true;
        Ok(())
    }
    async fn touch_credfile(&self) -> Result<(), SwapError> {
        Ok(())
    }
}

#[tokio::test]
async fn test_switch_aborts_and_rolls_back_on_post_switch_identity_mismatch() {
    let _lock = crate::testing::lock_data_dir();
    let _env = setup_test_data_dir();
    let (store, _dir) = test_store();
    let current_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    seed_account(&store, current_id);
    seed_account(&store, target_id);

    let initial_cc_blob = crate::testing::fresh_blob_json();
    let replacement = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-intruder","refreshToken":"sk-ant-ort01-intruder","expiresAt":9999999999999,"scopes":["user:inference"],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}"#;

    save_private(current_id, &initial_cc_blob).unwrap();
    save_private(target_id, &crate::testing::fresh_blob_json()).unwrap();
    store.set_active_cli(current_id).unwrap();

    let platform = MockPlatformReadReplacesBlob::new(&initial_cc_blob, replacement);
    let refresher = MockRefresher::success();
    // First call (pre-write outgoing check) matches current_id;
    // Second call (pre-write target check) matches target_id;
    // Third call (post-write check) returns intruder email so it mismatches target.
    struct PostSwitchMismatchFetcher {
        calls: std::sync::Mutex<usize>,
        current_email: String,
        target_email: String,
    }
    #[async_trait::async_trait]
    impl super::ProfileFetcher for PostSwitchMismatchFetcher {
        async fn fetch_email(&self, _t: &str) -> Result<String, OAuthError> {
            let mut c = self.calls.lock().unwrap();
            *c += 1;
            Ok(match *c {
                1 => self.current_email.clone(),
                2 => self.target_email.clone(),
                _ => "intruder@example.com".into(),
            })
        }
    }
    let fetcher = PostSwitchMismatchFetcher {
        calls: std::sync::Mutex::new(0),
        current_email: format!("seed-{current_id}@example.com"),
        target_email: format!("seed-{target_id}@example.com"),
    };

    let result = switch_force_for_tests(
        &store,
        Some(current_id),
        target_id,
        &platform,
        false,
        &refresher,
        &fetcher,
    )
    .await;

    assert!(
        matches!(result, Err(SwapError::IdentityMismatch { .. })),
        "expected IdentityMismatch on post-switch verification, got {result:?}"
    );
    // active_cli must still be current — not flipped to target.
    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(current_id.to_string()),
        "active_cli must not flip on post-switch failure"
    );
    // Rollback restored the original CC blob (not the replacement).
    assert_eq!(platform.get().as_deref(), Some(initial_cc_blob.as_str()));

    delete_private(current_id).unwrap();
    delete_private(target_id).unwrap();
}
