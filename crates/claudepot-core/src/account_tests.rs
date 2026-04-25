//! Inline test module for `account.rs`. Lives in this sibling file
//! so `account.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "account_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;

fn test_store() -> (AccountStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let store = AccountStore::open(&db).unwrap();
    (store, dir)
}

fn make_account(email: &str) -> Account {
    Account {
        uuid: Uuid::new_v4(),
        email: email.to_string(),
        org_uuid: Some("org-123".to_string()),
        org_name: Some("Test Org".to_string()),
        subscription_type: Some("pro".to_string()),
        rate_limit_tier: Some("default".to_string()),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
        verified_email: None,
        verified_at: None,
        verify_status: "never".to_string(),
    }
}

#[test]
fn test_store_open_creates_tables() {
    let (store, _dir) = test_store();
    let accounts = store.list().unwrap();
    assert!(accounts.is_empty());
}

#[test]
fn test_store_insert_and_find_by_email() {
    let (store, _dir) = test_store();
    let account = make_account("alice@example.com");
    let uuid = account.uuid;
    store.insert(&account).unwrap();

    let found = store.find_by_email("alice@example.com").unwrap().unwrap();
    assert_eq!(found.uuid, uuid);
    assert_eq!(found.email, "alice@example.com");
    assert_eq!(found.org_name.as_deref(), Some("Test Org"));
    assert!(found.has_cli_credentials);
    assert!(!found.has_desktop_profile);
}

#[test]
fn test_store_insert_and_find_by_uuid() {
    let (store, _dir) = test_store();
    let account = make_account("bob@example.com");
    let uuid = account.uuid;
    store.insert(&account).unwrap();

    let found = store.find_by_uuid(uuid).unwrap().unwrap();
    assert_eq!(found.email, "bob@example.com");
}

#[test]
fn test_store_insert_duplicate_email_fails() {
    let (store, _dir) = test_store();
    store.insert(&make_account("dup@example.com")).unwrap();
    let result = store.insert(&make_account("dup@example.com"));
    assert!(result.is_err());
}

#[test]
fn test_store_list_ordered_by_email() {
    let (store, _dir) = test_store();
    store.insert(&make_account("charlie@example.com")).unwrap();
    store.insert(&make_account("alice@example.com")).unwrap();
    store.insert(&make_account("bob@example.com")).unwrap();

    let list = store.list().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].email, "alice@example.com");
    assert_eq!(list[1].email, "bob@example.com");
    assert_eq!(list[2].email, "charlie@example.com");
}

#[test]
fn test_store_remove_deletes_account() {
    let (store, _dir) = test_store();
    let account = make_account("remove@example.com");
    let uuid = account.uuid;
    store.insert(&account).unwrap();

    store.remove(uuid).unwrap();
    assert!(store.find_by_uuid(uuid).unwrap().is_none());
}

#[test]
fn test_store_set_active_cli_and_read() {
    let (store, _dir) = test_store();
    let account = make_account("cli@example.com");
    let uuid = account.uuid;
    store.insert(&account).unwrap();

    store.set_active_cli(uuid).unwrap();
    assert_eq!(store.active_cli_uuid().unwrap(), Some(uuid.to_string()));
}

#[test]
fn test_store_active_cli_reflected_in_list() {
    let (store, _dir) = test_store();
    let a = make_account("a@example.com");
    let b = make_account("b@example.com");
    let a_uuid = a.uuid;
    store.insert(&a).unwrap();
    store.insert(&b).unwrap();

    store.set_active_cli(a_uuid).unwrap();
    let list = store.list().unwrap();
    let a_found = list.iter().find(|x| x.uuid == a_uuid).unwrap();
    assert!(a_found.is_cli_active);
    let b_found = list.iter().find(|x| x.uuid != a_uuid).unwrap();
    assert!(!b_found.is_cli_active);
}

#[test]
fn test_store_clear_active_cli() {
    let (store, _dir) = test_store();
    let account = make_account("clear@example.com");
    store.insert(&account).unwrap();
    store.set_active_cli(account.uuid).unwrap();

    store.clear_active_cli().unwrap();
    assert!(store.active_cli_uuid().unwrap().is_none());
}

#[test]
fn test_store_set_active_desktop_and_read() {
    let (store, _dir) = test_store();
    let account = make_account("desk@example.com");
    let uuid = account.uuid;
    store.insert(&account).unwrap();

    store.set_active_desktop(uuid).unwrap();
    assert_eq!(store.active_desktop_uuid().unwrap(), Some(uuid.to_string()));
}

#[test]
fn test_store_clear_active_desktop() {
    let (store, _dir) = test_store();
    let account = make_account("desk2@example.com");
    store.insert(&account).unwrap();
    store.set_active_desktop(account.uuid).unwrap();

    store.clear_active_desktop().unwrap();
    assert!(store.active_desktop_uuid().unwrap().is_none());
}

#[test]
fn test_store_update_credentials_flag() {
    let (store, _dir) = test_store();
    let mut account = make_account("flag@example.com");
    account.has_cli_credentials = false;
    store.insert(&account).unwrap();

    store.update_credentials_flag(account.uuid, true).unwrap();
    let found = store.find_by_uuid(account.uuid).unwrap().unwrap();
    assert!(found.has_cli_credentials);
}

#[test]
fn test_store_update_desktop_profile_flag() {
    let (store, _dir) = test_store();
    let account = make_account("profile@example.com");
    store.insert(&account).unwrap();

    store
        .update_desktop_profile_flag(account.uuid, true)
        .unwrap();
    let found = store.find_by_uuid(account.uuid).unwrap().unwrap();
    assert!(found.has_desktop_profile);
}

#[test]
fn test_store_set_active_cli_updates_last_switch() {
    let (store, _dir) = test_store();
    let account = make_account("switch@example.com");
    store.insert(&account).unwrap();

    store.set_active_cli(account.uuid).unwrap();
    let found = store.find_by_uuid(account.uuid).unwrap().unwrap();
    assert!(found.last_cli_switch.is_some());
}

#[test]
fn test_store_find_by_email_not_found() {
    let (store, _dir) = test_store();
    assert!(store.find_by_email("nobody@example.com").unwrap().is_none());
}

// -- Group 6: transactional set_active --

#[test]
fn test_set_active_cli_nonexistent_uuid_rolls_back() {
    // set_active_cli with an unknown UUID must NOT commit an orphan
    // state pointer. The transaction rolls back on zero affected rows
    // and returns an error; state.active_cli stays unchanged.
    let (store, _dir) = test_store();
    let orphan_uuid = Uuid::new_v4();

    let before = store.active_cli_uuid().unwrap();
    let result = store.set_active_cli(orphan_uuid);
    let after = store.active_cli_uuid().unwrap();

    assert!(before.is_none(), "no active_cli before");
    assert!(
        matches!(result, Err(rusqlite::Error::QueryReturnedNoRows)),
        "expected zero-row error, got {:?}",
        result
    );
    assert_eq!(after, None, "state must not be updated for orphan UUID");
}

#[test]
fn test_set_active_cli_transaction_both_updated() {
    // Positive path: both the state table and accounts.last_cli_switch
    // must be updated atomically by set_active_cli.
    let (store, _dir) = test_store();
    let account = make_account("atomic@example.com");
    store.insert(&account).unwrap();

    store.set_active_cli(account.uuid).unwrap();

    assert_eq!(
        store.active_cli_uuid().unwrap(),
        Some(account.uuid.to_string()),
        "state.active_cli updated"
    );
    let row = store.find_by_uuid(account.uuid).unwrap().unwrap();
    assert!(
        row.last_cli_switch.is_some(),
        "accounts.last_cli_switch updated in the same transaction"
    );
}

#[test]
fn test_set_active_cli_same_uuid_is_noop() {
    // Regression: sync_from_current_cc and login_and_reimport call
    // set_active_cli on every tick, often with the already-active
    // UUID. Before the idempotent guard, each call pushed
    // last_cli_switch forward to Utc::now() — the GUI then showed
    // "CLI switch just now" even when nothing changed. set_active
    // must now leave the timestamp alone when the pointer already
    // matches the incoming UUID.
    let (store, _dir) = test_store();
    let account = make_account("noop@example.com");
    store.insert(&account).unwrap();

    store.set_active_cli(account.uuid).unwrap();
    let first = store
        .find_by_uuid(account.uuid)
        .unwrap()
        .unwrap()
        .last_cli_switch
        .expect("first set populates timestamp");

    // Pause so a spurious write would produce a strictly-greater
    // timestamp (the idempotent guard should prevent this).
    std::thread::sleep(std::time::Duration::from_millis(20));

    store.set_active_cli(account.uuid).unwrap();
    let second = store
        .find_by_uuid(account.uuid)
        .unwrap()
        .unwrap()
        .last_cli_switch
        .expect("second set leaves timestamp populated");

    assert_eq!(
        first, second,
        "set_active_cli(same_uuid) must not bump last_cli_switch"
    );
}

#[test]
fn test_set_active_cli_different_uuid_bumps_timestamp() {
    // Complementary guard: when the pointer does change, the
    // timestamp MUST move. Otherwise a real swap would look
    // indistinguishable from the idle sync path.
    let (store, _dir) = test_store();
    let a = make_account("a@example.com");
    let b = make_account("b@example.com");
    store.insert(&a).unwrap();
    store.insert(&b).unwrap();

    store.set_active_cli(a.uuid).unwrap();
    let t_a = store
        .find_by_uuid(a.uuid)
        .unwrap()
        .unwrap()
        .last_cli_switch
        .expect("timestamp after first set");

    std::thread::sleep(std::time::Duration::from_millis(20));

    store.set_active_cli(b.uuid).unwrap();
    let t_b = store
        .find_by_uuid(b.uuid)
        .unwrap()
        .unwrap()
        .last_cli_switch
        .expect("timestamp after swap to b");

    assert!(
        t_b > t_a,
        "set_active_cli(new_uuid) must bump last_cli_switch for the new target"
    );
}

// --- find_by_org_uuid (Desktop org-UUID fast-path primitive) ---

fn make_account_with_org(email: &str, org: Option<&str>) -> Account {
    let mut a = make_account(email);
    a.org_uuid = org.map(String::from);
    a
}

#[test]
fn test_find_by_org_uuid_no_match_returns_none() {
    let (store, _dir) = test_store();
    let wanted = Uuid::new_v4();
    store
        .insert(&make_account_with_org(
            "a@example.com",
            Some(&Uuid::new_v4().to_string()),
        ))
        .unwrap();
    assert!(store.find_by_org_uuid(wanted).unwrap().is_none());
}

#[test]
fn test_find_by_org_uuid_unique_match_returns_account() {
    let (store, _dir) = test_store();
    let org = Uuid::new_v4();
    let a = make_account_with_org("a@example.com", Some(&org.to_string()));
    store.insert(&a).unwrap();
    store
        .insert(&make_account_with_org(
            "b@example.com",
            Some(&Uuid::new_v4().to_string()),
        ))
        .unwrap();

    let found = store.find_by_org_uuid(org).unwrap().expect("unique match");
    assert_eq!(found.email, "a@example.com");
}

#[test]
fn test_find_by_org_uuid_ambiguous_returns_none() {
    // Two accounts in the same org → ambiguous. We must not
    // pick one arbitrarily — callers rely on None to force the
    // slow-path identity probe.
    let (store, _dir) = test_store();
    let org = Uuid::new_v4();
    store
        .insert(&make_account_with_org(
            "a@example.com",
            Some(&org.to_string()),
        ))
        .unwrap();
    store
        .insert(&make_account_with_org(
            "b@example.com",
            Some(&org.to_string()),
        ))
        .unwrap();

    assert!(store.find_by_org_uuid(org).unwrap().is_none());
}

#[test]
fn test_find_by_org_uuid_null_org_uuid_is_skipped() {
    // A row with NULL org_uuid must never collide with a lookup —
    // the SQL equality is already NULL-safe (NULL = X is NULL, not
    // true), but we lock it down explicitly so a future rewrite
    // using IS NOT DISTINCT FROM doesn't regress.
    let (store, _dir) = test_store();
    let org = Uuid::new_v4();
    store
        .insert(&make_account_with_org("null@example.com", None))
        .unwrap();
    let a = make_account_with_org("a@example.com", Some(&org.to_string()));
    store.insert(&a).unwrap();

    let found = store.find_by_org_uuid(org).unwrap().expect("unique");
    assert_eq!(found.email, "a@example.com");
}

#[test]
fn test_find_by_org_uuid_surfaces_active_pointer() {
    // Returned Account must reflect active_cli / active_desktop
    // (consistent with find_by_uuid + find_by_email).
    let (store, _dir) = test_store();
    let org = Uuid::new_v4();
    let a = make_account_with_org("a@example.com", Some(&org.to_string()));
    store.insert(&a).unwrap();
    store.set_active_cli(a.uuid).unwrap();

    let found = store.find_by_org_uuid(org).unwrap().unwrap();
    assert!(found.is_cli_active);
    assert!(!found.is_desktop_active);
}
