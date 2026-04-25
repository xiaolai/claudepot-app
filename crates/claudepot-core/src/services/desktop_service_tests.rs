//! Inline test module for `desktop_service.rs`. Lives in this sibling file
//! so `desktop_service.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "desktop_service_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use crate::account::Account;
use chrono::Utc;

fn setup() -> (AccountStore, tempfile::TempDir) {
    // reconcile walks paths::desktop_profile_dir(uuid), which
    // reads CLAUDEPOT_DATA_DIR. Use the shared testing helper so
    // other parallel tests don't fight over the env var.
    let env = crate::testing::setup_test_data_dir();
    let db = env.path().join("accounts.db");
    let store = AccountStore::open(&db).unwrap();
    (store, env)
}

fn make_account(email: &str) -> Account {
    Account {
        uuid: Uuid::new_v4(),
        email: email.to_string(),
        org_uuid: None,
        org_name: None,
        subscription_type: None,
        rate_limit_tier: None,
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: false,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
        verified_email: None,
        verified_at: None,
        verify_status: "never".to_string(),
    }
}

#[test]
fn test_reconcile_noop_on_clean_state() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let outcome = reconcile_flags(&store).unwrap();
    assert!(outcome.flag_flips.is_empty());
    assert!(!outcome.orphan_pointer_cleared);
}

#[test]
fn test_reconcile_flips_true_to_false_when_dir_missing() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let mut a = make_account("a@example.com");
    a.has_desktop_profile = true; // flag claims profile exists
    store.insert(&a).unwrap();
    // Do NOT create the dir on disk.

    let outcome = reconcile_flags(&store).unwrap();
    assert_eq!(outcome.flag_flips.len(), 1);
    assert!(!outcome.flag_flips[0].new_value);
    assert_eq!(outcome.flag_flips[0].email, "a@example.com");

    let after = store.find_by_uuid(a.uuid).unwrap().unwrap();
    assert!(!after.has_desktop_profile);
}

#[test]
fn test_reconcile_flips_false_to_true_when_dir_exists() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let a = make_account("a@example.com"); // has_desktop_profile = false
    store.insert(&a).unwrap();
    std::fs::create_dir_all(paths::desktop_profile_dir(a.uuid)).unwrap();

    let outcome = reconcile_flags(&store).unwrap();
    assert_eq!(outcome.flag_flips.len(), 1);
    assert!(outcome.flag_flips[0].new_value);

    let after = store.find_by_uuid(a.uuid).unwrap().unwrap();
    assert!(after.has_desktop_profile);
}

#[test]
fn test_reconcile_clears_orphan_active_pointer() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let a = make_account("a@example.com");
    store.insert(&a).unwrap();
    store.set_active_desktop(a.uuid).unwrap();
    store.remove(a.uuid).unwrap(); // remove without clearing pointer

    let outcome = reconcile_flags(&store).unwrap();
    assert!(outcome.orphan_pointer_cleared);
    assert!(store.active_desktop_uuid().unwrap().is_none());
}

#[test]
fn test_reconcile_preserves_valid_active_pointer() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let a = make_account("a@example.com");
    store.insert(&a).unwrap();
    store.set_active_desktop(a.uuid).unwrap();

    let outcome = reconcile_flags(&store).unwrap();
    assert!(!outcome.orphan_pointer_cleared);
    assert!(store.active_desktop_uuid().unwrap().is_some());
}

#[test]
fn test_reconcile_idempotent() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let mut a = make_account("a@example.com");
    a.has_desktop_profile = true;
    store.insert(&a).unwrap();

    // First pass: expect one flip.
    let first = reconcile_flags(&store).unwrap();
    assert_eq!(first.flag_flips.len(), 1);

    // Second pass: clean state.
    let second = reconcile_flags(&store).unwrap();
    assert!(second.flag_flips.is_empty());
}

// -- adopt / clear / sync tests -----------------------------------

use crate::desktop_identity::{LiveDesktopIdentity, ProbeMethod, VerifiedIdentity};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

struct TestPlatform {
    data_dir: PathBuf,
    items: Vec<&'static str>,
    running: AtomicBool,
}

#[async_trait::async_trait]
impl crate::desktop_backend::DesktopPlatform for TestPlatform {
    fn data_dir(&self) -> Option<PathBuf> { Some(self.data_dir.clone()) }
    fn session_items(&self) -> &[&str] { &self.items }
    async fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }
    async fn quit(&self) -> Result<(), crate::error::DesktopSwapError> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
    async fn launch(&self) -> Result<(), crate::error::DesktopSwapError> {
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn is_installed(&self) -> bool { true }
    async fn safe_storage_secret(
        &self,
    ) -> Result<Vec<u8>, crate::desktop_backend::DesktopKeyError> {
        // Adopt/clear never call safe_storage_secret directly —
        // they receive a pre-built VerifiedIdentity. Return
        // Unsupported so an accidental call is loud.
        Err(crate::desktop_backend::DesktopKeyError::Unsupported)
    }
}

fn platform_for(data_dir: PathBuf) -> TestPlatform {
    TestPlatform {
        data_dir,
        items: vec!["config.json", "Cookies"],
        running: AtomicBool::new(false),
    }
}

fn verified_for(email: &str, org_uuid: &str) -> VerifiedIdentity {
    VerifiedIdentity::from_live_for_testing(LiveDesktopIdentity {
        email: email.to_string(),
        org_uuid: org_uuid.to_string(),
        probe_method: ProbeMethod::Decrypted,
    })
}

fn populate_data_dir(data_dir: &std::path::Path) {
    std::fs::create_dir_all(data_dir).unwrap();
    std::fs::write(data_dir.join("config.json"), b"{\"test\":true}").unwrap();
    std::fs::write(data_dir.join("Cookies"), b"cookie-bytes").unwrap();
}

#[tokio::test]
async fn test_adopt_happy_path() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir);
    let vid = verified_for("alice@example.com", "org-xxx");

    let out = adopt_current(&platform, &store, acct.uuid, &vid, false).await.unwrap();
    assert_eq!(out.account_email, "alice@example.com");
    assert!(out.captured_items >= 2);

    // Flag + pointer updated.
    let after = store.find_by_uuid(acct.uuid).unwrap().unwrap();
    assert!(after.has_desktop_profile);
    assert!(after.is_desktop_active);

    // Sidecar present and parseable.
    let profile_dir = paths::desktop_profile_dir(acct.uuid);
    let sidecar = profile_dir.join("claudepot.profile.json");
    assert!(sidecar.exists(), "sidecar must be written");
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(parsed["captured_from_email"], "alice@example.com");
    assert_eq!(parsed["captured_verified"], true);
}

#[tokio::test]
async fn test_adopt_rejects_identity_mismatch() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir);
    // Live identity says we're signed in as BOB — must refuse.
    let vid = verified_for("bob@example.com", "org-xxx");

    let err = adopt_current(&platform, &store, acct.uuid, &vid, false).await.unwrap_err();
    assert!(matches!(err, AdoptError::IdentityMismatch { .. }));
    // No mutations — verify the flag didn't flip.
    let after = store.find_by_uuid(acct.uuid).unwrap().unwrap();
    assert!(!after.has_desktop_profile);
}

#[tokio::test]
async fn test_adopt_refuses_overwrite_without_flag() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();
    // Pre-create a profile dir so the adopt must bail unless overwrite=true.
    std::fs::create_dir_all(paths::desktop_profile_dir(acct.uuid)).unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir);
    let vid = verified_for("alice@example.com", "org-xxx");

    let err = adopt_current(&platform, &store, acct.uuid, &vid, false).await.unwrap_err();
    assert!(matches!(err, AdoptError::ProfileExists));
}

#[tokio::test]
async fn test_adopt_with_overwrite_replaces_profile() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();
    let profile_dir = paths::desktop_profile_dir(acct.uuid);
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(profile_dir.join("stale.txt"), b"stale").unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir);
    let vid = verified_for("alice@example.com", "org-xxx");

    adopt_current(&platform, &store, acct.uuid, &vid, true).await.unwrap();
    assert!(!profile_dir.join("stale.txt").exists(), "old content must be purged");
    assert!(profile_dir.join("config.json").exists());
}

#[tokio::test]
async fn test_clear_session_stashes_snapshot_by_default() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();
    store.set_active_desktop(acct.uuid).unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir.clone());

    let out = clear_session(&platform, &store, true).await.unwrap();
    assert!(out.snapshot_kept);
    assert_eq!(out.items_deleted, 2);
    assert_eq!(out.email.as_deref(), Some("alice@example.com"));

    // Items gone from data_dir.
    assert!(!data_dir.join("config.json").exists());
    assert!(!data_dir.join("Cookies").exists());
    // Profile dir has the snapshot.
    let profile_dir = paths::desktop_profile_dir(acct.uuid);
    assert!(profile_dir.join("config.json").exists());

    // Active pointer cleared.
    assert!(store.active_desktop_uuid().unwrap().is_none());
}

#[tokio::test]
async fn test_clear_session_keep_snapshot_false() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();
    store.set_active_desktop(acct.uuid).unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir.clone());

    let out = clear_session(&platform, &store, false).await.unwrap();
    assert!(!out.snapshot_kept);

    // No stashed snapshot.
    let profile_dir = paths::desktop_profile_dir(acct.uuid);
    assert!(!profile_dir.join("config.json").exists());
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn test_dpapi_check_is_noop_on_non_windows() {
    // On macOS/Linux the DPAPI invalidation mode doesn't exist,
    // so the precheck must always report valid.
    let _lock = crate::testing::lock_data_dir();
    let (_store, _env) = setup();
    let tmp = _env.path().join("Claude");
    std::fs::create_dir_all(&tmp).unwrap();
    let platform = platform_for(tmp);
    let ok = check_profile_dpapi_valid(&platform, Uuid::new_v4())
        .await
        .unwrap();
    assert!(ok, "non-Windows must always report DPAPI-valid");
}

#[tokio::test]
async fn test_clear_session_prunes_empty_network_dir() {
    // Windows-style nested items: session contains Network/Cookies.
    // After deletion the empty Network/ parent must be pruned.
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();

    let data_dir = _env.path().join("Claude");
    std::fs::create_dir_all(data_dir.join("Network")).unwrap();
    std::fs::write(data_dir.join("config.json"), b"{}").unwrap();
    std::fs::write(data_dir.join("Network/Cookies"), b"x").unwrap();

    let platform = TestPlatform {
        data_dir: data_dir.clone(),
        items: vec!["config.json", "Network/Cookies"],
        running: AtomicBool::new(false),
    };

    clear_session(&platform, &store, false).await.unwrap();
    // Network/ was empty after Cookies removal → pruned.
    assert!(!data_dir.join("Network").exists(), "empty Network/ must be pruned");
}

// -- switch (B-3 desktop_use preflight) tests ---------------------

#[tokio::test]
async fn test_switch_rejects_target_without_snapshot() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();
    // Deliberately do NOT create paths::desktop_profile_dir(acct.uuid).

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir);

    let err = switch(&platform, &store, acct.uuid, true).await.unwrap_err();
    // Verbatim error wording — UI copy is exact.
    let msg = err.to_string();
    assert_eq!(
        msg,
        "alice@example.com has no Desktop profile yet \u{2014} sign in via the Desktop app first",
    );
    assert!(matches!(err, SwitchError::NoSnapshot { .. }));
}

#[tokio::test]
async fn test_switch_rejects_unregistered_target() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir);

    let bogus = Uuid::new_v4();
    let err = switch(&platform, &store, bogus, true).await.unwrap_err();
    match err {
        SwitchError::NotFound(u) => assert_eq!(u, bogus),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn test_switch_happy_path_calls_swap_and_returns_outcome() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();

    // Outgoing account: has a snapshot on disk so swap can stash
    // the live data_dir into it without complaint.
    let outgoing = make_account("alice@example.com");
    store.insert(&outgoing).unwrap();
    std::fs::create_dir_all(paths::desktop_profile_dir(outgoing.uuid)).unwrap();
    store.set_active_desktop(outgoing.uuid).unwrap();

    // Target account: pre-populated profile dir so swap::switch
    // finds something to restore.
    let target = make_account("bob@example.com");
    store.insert(&target).unwrap();
    let target_profile_dir = paths::desktop_profile_dir(target.uuid);
    std::fs::create_dir_all(&target_profile_dir).unwrap();
    std::fs::write(target_profile_dir.join("config.json"), b"{\"target\":true}").unwrap();
    std::fs::write(target_profile_dir.join("Cookies"), b"target-cookies").unwrap();

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = platform_for(data_dir.clone());

    let out = switch(&platform, &store, target.uuid, true).await.unwrap();
    assert_eq!(out.email, "bob@example.com");
    assert_eq!(out.outgoing_email.as_deref(), Some("alice@example.com"));

    // active_desktop pointer now references target (swap::switch's
    // postcondition — proves we actually delegated).
    let active = store.active_desktop_uuid().unwrap().unwrap();
    assert_eq!(active.parse::<Uuid>().unwrap(), target.uuid);

    // Disk-side proof: data_dir holds the target's contents.
    assert_eq!(
        std::fs::read(data_dir.join("config.json")).unwrap(),
        b"{\"target\":true}",
    );
}

#[tokio::test]
async fn test_switch_does_not_quit_desktop_on_preflight_failure() {
    let _lock = crate::testing::lock_data_dir();
    let (store, _env) = setup();
    let acct = make_account("alice@example.com");
    store.insert(&acct).unwrap();
    // No snapshot on disk → preflight will reject.

    let data_dir = _env.path().join("Claude");
    populate_data_dir(&data_dir);
    let platform = TestPlatform {
        data_dir,
        items: vec!["config.json", "Cookies"],
        running: AtomicBool::new(true),
    };

    let err = switch(&platform, &store, acct.uuid, true).await.unwrap_err();
    assert!(matches!(err, SwitchError::NoSnapshot { .. }));
    // The whole point of running the preflight FIRST: Desktop must
    // still be running.
    assert!(
        platform.running.load(Ordering::SeqCst),
        "preflight failure must not quit Desktop",
    );
}
