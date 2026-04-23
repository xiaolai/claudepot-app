//! Claude Desktop service — Phase 1: reconcile only.
//!
//! Later phases add adopt / clear / sync_from_current. Those require
//! verified identity (Phase 2 crypto) and are explicitly out of scope
//! here — see `dev-docs/desktop-feature-overhaul-plan.md` §Rollout.

use crate::account::AccountStore;
use crate::paths;
use uuid::Uuid;

/// Result of a `reconcile_flags` pass.
#[derive(Debug, Default, Clone)]
pub struct ReconcileOutcome {
    /// One entry per account whose `has_desktop_profile` flag flipped.
    pub flag_flips: Vec<FlagFlip>,
    /// True when `state.active_desktop` pointed at a UUID that does
    /// not (or no longer) correspond to a registered account, and we
    /// cleared it.
    pub orphan_pointer_cleared: bool,
}

#[derive(Debug, Clone)]
pub struct FlagFlip {
    pub email: String,
    pub uuid: Uuid,
    pub new_value: bool,
}

/// Bring `accounts.has_desktop_profile` into alignment with the
/// filesystem. Every mismatch flips the DB flag to match on-disk
/// truth (the flag is a cached view of the snapshot dir's
/// existence). Also clears `state.active_desktop` when it points at
/// a UUID that no longer has a registered account.
///
/// Idempotent — a clean state returns an empty [`ReconcileOutcome`].
/// Safe to run in the hot path of `account_list` (all writes are
/// O(1) per changed row).
pub fn reconcile_flags(store: &AccountStore) -> Result<ReconcileOutcome, rusqlite::Error> {
    let accounts = store.list()?;
    let mut flips = Vec::new();

    for a in &accounts {
        let on_disk = paths::desktop_profile_dir(a.uuid).exists();
        if a.has_desktop_profile != on_disk {
            store.update_desktop_profile_flag(a.uuid, on_disk)?;
            flips.push(FlagFlip {
                email: a.email.clone(),
                uuid: a.uuid,
                new_value: on_disk,
            });
        }
    }

    // Orphan pointer: state.active_desktop holds a UUID but no
    // registered account matches. Possible after an out-of-band DB
    // edit, a test harness glitch, or a race where an account was
    // removed while the pointer update failed.
    let orphan_cleared = match store.active_desktop_uuid()? {
        Some(uuid_str) => {
            let is_orphan = match Uuid::parse_str(&uuid_str) {
                // Un-parseable UUIDs are definitionally orphaned.
                Err(_) => true,
                Ok(u) => !accounts.iter().any(|a| a.uuid == u),
            };
            if is_orphan {
                store.clear_active_desktop()?;
                true
            } else {
                false
            }
        }
        None => false,
    };

    Ok(ReconcileOutcome {
        flag_flips: flips,
        orphan_pointer_cleared: orphan_cleared,
    })
}

#[cfg(test)]
mod tests {
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
}
