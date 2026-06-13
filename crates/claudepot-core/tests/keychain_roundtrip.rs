//! Real-keychain round-trip for `cli_backend::storage` — `#[ignore]`
//! by default per `.claude/rules/rust-conventions.md` ("Integration
//! tests that touch the Keychain ... are #[ignore] by default").
//!
//! These exercise the actual `/usr/bin/security` subprocess contract
//! against the login keychain: `add-generic-password -U` with the
//! read-back verification, `find-generic-password` exit-code mapping,
//! and `delete-generic-password` idempotency — the layer the unit
//! tests in `storage.rs` can only pin as pure decision functions.
//!
//! Run on the test host (see CLAUDE.md "Test on test-host"):
//!
//! ```bash
//! cargo test -p claudepot-core --test keychain_roundtrip -- --ignored
//! # or remotely:
//! ssh <user>@<host> "security unlock-keychain -p <password> \
//!   ~/Library/Keychains/login.keychain-db" && \
//!   cargo test -p claudepot-core --test keychain_roundtrip -- --ignored
//! ```
//!
//! Items are namespaced under service `com.claudepot.credentials`
//! with a fresh UUIDv4 account per test, and each test deletes what
//! it created — a failed assertion can leave at most one orphaned
//! test item.

#![cfg(target_os = "macos")]

use claudepot_core::cli_backend::storage;
use claudepot_core::error::SwapError;
use uuid::Uuid;

/// Force the keyring-only backend so no test ever touches file
/// storage (and the Auto-mode import/fallback machinery stays out of
/// the picture — that policy is unit-tested in `storage.rs`).
fn force_keyring_backend() {
    std::env::set_var("CLAUDEPOT_CREDENTIAL_BACKEND", "keyring");
}

#[tokio::test]
#[ignore = "real login keychain; run on test-host with the keychain unlocked"]
async fn test_keychain_save_load_delete_round_trip() {
    force_keyring_backend();
    let account_id = Uuid::new_v4();
    let blob = format!("{{\"test\":\"claudepot-roundtrip\",\"id\":\"{account_id}\"}}");

    // save → read-back verification happens inside save_to_keyring.
    storage::save(account_id, &blob).await.expect("save");

    // load returns exactly what was written.
    let loaded = storage::load(account_id).await.expect("load");
    assert_eq!(loaded, blob);

    // `add-generic-password -U` is update-or-create: a second save
    // with different content must overwrite, not duplicate or fail.
    let blob2 = format!("{{\"test\":\"claudepot-roundtrip-2\",\"id\":\"{account_id}\"}}");
    storage::save(account_id, &blob2).await.expect("re-save");
    let loaded2 = storage::load(account_id).await.expect("re-load");
    assert_eq!(loaded2, blob2);

    // delete, then the item must be gone (clean miss → typed error).
    storage::delete(account_id).await.expect("delete");
    let err = storage::load(account_id)
        .await
        .expect_err("load after delete");
    assert!(
        matches!(err, SwapError::NoStoredCredentials(id) if id == account_id),
        "err={err:?}"
    );
}

#[tokio::test]
#[ignore = "real login keychain; run on test-host with the keychain unlocked"]
async fn test_keychain_delete_missing_item_is_idempotent() {
    force_keyring_backend();
    // Never-saved uuid: exit 44 from delete-generic-password must map
    // to Ok — delete is an idempotent contract.
    let account_id = Uuid::new_v4();
    storage::delete(account_id).await.expect("first delete");
    storage::delete(account_id).await.expect("second delete");
}

#[tokio::test]
#[ignore = "real login keychain; run on test-host with the keychain unlocked"]
async fn test_keychain_load_missing_item_is_clean_typed_miss() {
    force_keyring_backend();
    let account_id = Uuid::new_v4();
    let err = storage::load(account_id).await.expect_err("missing item");
    assert!(
        matches!(err, SwapError::NoStoredCredentials(id) if id == account_id),
        "err={err:?}"
    );
}
