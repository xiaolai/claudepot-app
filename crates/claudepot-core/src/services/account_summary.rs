//! Listing-time aggregation for the Accounts surface.
//!
//! `AccountSummaryView` is the single shape produced by enumerating
//! the store and gathering the per-row I/O the GUI's account list
//! needs: token health (Keychain read), desktop profile presence
//! (filesystem stat), plus the derived health flags. It exists so
//! the Tauri DTO mapping (`From<&AccountSummaryView> for
//! AccountSummary`) can stay a pure field copy without smuggling
//! Keychain or filesystem calls into a `From` impl.
//!
//! Keychain access is intentionally **sequential**: macOS surfaces
//! one unlock dialog per locked-keychain access, so parallel
//! `swap::load_private` calls would stack dialogs on the user. The
//! ordering here mirrors the previous in-place loop in
//! `dto.rs::AccountSummary::from`.

use crate::account::{Account, AccountStore};
use crate::paths;
use crate::services::account_service::{token_health, TokenHealth};

/// Aggregated, listing-time view of one account row. Pairs the raw
/// [`Account`] with the listing-time I/O results so the DTO layer
/// can map without re-running Keychain / filesystem calls.
#[derive(Debug)]
pub struct AccountSummaryView {
    pub account: Account,
    pub token_health: TokenHealth,
    /// True iff the stored credential blob exists and parses. Mirrors
    /// reality, not the DB flag — used to gate the "Use CLI" button.
    pub credentials_healthy: bool,
    /// Per-file-on-disk truth for the Desktop profile snapshot dir.
    /// Computed via `paths::desktop_profile_dir(uuid).exists()`.
    /// Differs from `account.has_desktop_profile` only when the DB
    /// flag has drifted from disk.
    pub desktop_profile_on_disk: bool,
    /// `account.verify_status == "drift"`. Pre-computed so the DTO
    /// boundary doesn't have to know the verify-status vocabulary.
    pub drift: bool,
}

/// Enumerate the store and gather per-row token health + desktop
/// profile presence. Sequential to keep macOS Keychain unlock
/// dialogs from stacking.
///
/// Errors here are sqlite errors from `AccountStore::list`. The
/// per-row `token_health` and `paths::desktop_profile_dir(...).exists()`
/// calls already swallow their own I/O failures into a status string
/// or a `false`, so this signature stays narrow.
pub fn list_summaries(store: &AccountStore) -> Result<Vec<AccountSummaryView>, rusqlite::Error> {
    let accounts = store.list()?;
    let mut out = Vec::with_capacity(accounts.len());
    for account in accounts {
        let health = token_health(account.uuid, account.has_cli_credentials);
        // A stored blob is "healthy" if it exists and parses. Any other
        // status ("missing", "corrupt blob", "no credentials") means the
        // swap can't succeed — the UI should gate on this, not the DB flag.
        let credentials_healthy = health.status.starts_with("valid") || health.status == "expired";
        // Cheap on-disk check per plan v2 §D18: just exists(), no
        // recursive walk.
        let desktop_profile_on_disk = paths::desktop_profile_dir(account.uuid).exists();
        // Derive from verify_status, not `verified_email != email`.
        // update_verification() intentionally preserves verified_email
        // across rejected/network_error so history isn't wiped by a
        // blip — comparing emails would spuriously paint stale
        // history as drift.
        let drift = account.verify_status == "drift";
        out.push(AccountSummaryView {
            account,
            token_health: health,
            credentials_healthy,
            desktop_profile_on_disk,
            drift,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_verification::VerifyOutcome;
    use crate::cli_backend::swap;
    use crate::testing::{fresh_blob_json, make_account, setup_test_data_dir, test_store};

    fn insert(store: &AccountStore, account: &Account) {
        store.insert(account).unwrap();
    }

    #[test]
    fn test_list_summaries_empty_store() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();

        let views = list_summaries(&store).unwrap();
        assert!(views.is_empty());
    }

    #[test]
    fn test_list_summaries_no_credentials_yields_no_credentials_status() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let mut acct = make_account("nocred@example.com");
        acct.has_cli_credentials = false;
        insert(&store, &acct);

        let views = list_summaries(&store).unwrap();
        assert_eq!(views.len(), 1);
        let v = &views[0];
        assert_eq!(v.token_health.status, "no credentials");
        assert!(!v.credentials_healthy);
        assert!(v.token_health.remaining_mins.is_none());
    }

    #[test]
    fn test_list_summaries_drift_flag_derives_from_verify_status() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();

        // Two accounts: one clean, one drifted.
        let clean = make_account("clean@example.com");
        let drifted = make_account("drift@example.com");
        insert(&store, &clean);
        insert(&store, &drifted);
        store
            .update_verification(
                drifted.uuid,
                &VerifyOutcome::Drift {
                    stored_email: drifted.email.clone(),
                    actual_email: "other@example.com".to_string(),
                },
            )
            .unwrap();

        let views = list_summaries(&store).unwrap();
        let by_email: std::collections::HashMap<_, _> =
            views.iter().map(|v| (v.account.email.clone(), v)).collect();

        assert!(!by_email["clean@example.com"].drift);
        assert!(by_email["drift@example.com"].drift);
        assert_eq!(by_email["drift@example.com"].account.verify_status, "drift");
    }

    #[test]
    fn test_list_summaries_desktop_profile_on_disk_reflects_filesystem() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();

        let with_profile = make_account("hasdesk@example.com");
        let without_profile = make_account("nodesk@example.com");
        insert(&store, &with_profile);
        insert(&store, &without_profile);

        // Create a desktop profile dir for one account only. The DB
        // flag stays false on both; the test asserts that
        // `desktop_profile_on_disk` reflects the filesystem, not the
        // DB flag.
        let dir = paths::desktop_profile_dir(with_profile.uuid);
        std::fs::create_dir_all(&dir).unwrap();

        let views = list_summaries(&store).unwrap();
        let by_email: std::collections::HashMap<_, _> =
            views.iter().map(|v| (v.account.email.clone(), v)).collect();

        assert!(by_email["hasdesk@example.com"].desktop_profile_on_disk);
        assert!(!by_email["nodesk@example.com"].desktop_profile_on_disk);
    }

    #[test]
    fn test_list_summaries_credentials_healthy_with_valid_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let acct = make_account("valid@example.com");
        insert(&store, &acct);
        swap::save_private(acct.uuid, &fresh_blob_json()).unwrap();

        let views = list_summaries(&store).unwrap();
        assert_eq!(views.len(), 1);
        assert!(views[0].credentials_healthy);
        assert!(views[0].token_health.status.contains("valid"));

        swap::delete_private(acct.uuid).unwrap();
    }
}
