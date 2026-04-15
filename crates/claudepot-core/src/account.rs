use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use std::sync::{Mutex, MutexGuard};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Account {
    pub uuid: Uuid,
    pub email: String,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_cli_switch: Option<DateTime<Utc>>,
    pub last_desktop_switch: Option<DateTime<Utc>>,
    pub has_cli_credentials: bool,
    pub has_desktop_profile: bool,
    /// Computed: is this the active CLI account?
    pub is_cli_active: bool,
    /// Computed: is this the active Desktop account?
    pub is_desktop_active: bool,
    /// Last observed `/api/oauth/profile` email for this UUID's blob.
    /// `None` until a verification pass has run. When this differs from
    /// `email`, the slot is misfiled — the stored blob authenticates as
    /// someone other than the account label says.
    pub verified_email: Option<String>,
    /// Timestamp of the verification run that produced `verified_email` /
    /// `verify_status`. `None` means no verification has ever run.
    pub verified_at: Option<DateTime<Utc>>,
    /// Outcome of the last verification run. `"never"`, `"ok"`, `"drift"`,
    /// `"rejected"` (token refused by server), or `"network_error"`.
    pub verify_status: String,
}

// VerifyOutcome + AccountStore::update_verification extracted to
// `crate::account_verification`. Re-exported from the crate root so
// `claudepot_core::account::VerifyOutcome` still resolves.
pub use crate::account_verification::VerifyOutcome;

pub struct AccountStore {
    /// rusqlite::Connection is !Send on its own. Wrapping in Mutex makes the
    /// store Send + Sync so it can cross await points in Tauri commands.
    /// Contention is effectively zero — each CLI / GUI action is serialized.
    db: Mutex<Connection>,
}

impl AccountStore {
    /// Internal accessor — kept `pub(crate)` so sibling modules inside
    /// `claudepot-core` (e.g. `account_verification`) can run their own
    /// SQL without duplicating the lock/poisoning handling.
    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().expect("account store mutex poisoned")
    }

    /// Test-only helper: drop the accounts table so subsequent queries fail.
    /// Used to verify error-path handling in higher-level services.
    #[cfg(test)]
    pub(crate) fn corrupt_for_test(&self) {
        self.db().execute("DROP TABLE accounts", []).unwrap();
    }

    /// Test-only helper: drop the state table so active-pointer writes fail
    /// while accounts-table operations continue to work.
    #[cfg(test)]
    pub(crate) fn corrupt_state_table_for_test(&self) {
        self.db().execute("DROP TABLE state", []).unwrap();
    }
}

impl AccountStore {
    pub fn open(path: &std::path::Path) -> SqlResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        // Wait up to 5 s on writer contention before returning SQLITE_BUSY.
        // Without this, simultaneous CLI + GUI access on the same db file
        // would fail immediately with "database is locked".
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(SCHEMA)?;
        Self::migrate_add_verification_columns(&db)?;

        // Set 0600 permissions on the DB file (contains account metadata)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            // Also secure the WAL and SHM files if they exist
            let wal = path.with_extension("db-wal");
            let shm = path.with_extension("db-shm");
            if wal.exists() {
                std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            }
            if shm.exists() {
                std::fs::set_permissions(&shm, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            }
        }

        Ok(Self { db: Mutex::new(db) })
    }

    /// Additive migration: add `verified_email`, `verified_at`,
    /// `verify_status` columns to an existing `accounts` table. Idempotent —
    /// skips columns that already exist by consulting `PRAGMA table_info`.
    fn migrate_add_verification_columns(db: &Connection) -> SqlResult<()> {
        let mut existing: Vec<String> = Vec::new();
        {
            let mut stmt = db.prepare("PRAGMA table_info(accounts)")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for r in rows {
                existing.push(r?);
            }
        }
        let has = |col: &str| existing.iter().any(|c| c == col);
        if !has("verified_email") {
            db.execute("ALTER TABLE accounts ADD COLUMN verified_email TEXT", [])?;
        }
        if !has("verified_at") {
            db.execute("ALTER TABLE accounts ADD COLUMN verified_at TEXT", [])?;
        }
        if !has("verify_status") {
            db.execute(
                "ALTER TABLE accounts ADD COLUMN verify_status TEXT NOT NULL DEFAULT 'never'",
                [],
            )?;
        }
        Ok(())
    }

    fn row_to_account(
        row: &rusqlite::Row,
        active_cli: &Option<String>,
        active_desktop: &Option<String>,
    ) -> rusqlite::Result<Account> {
        let uuid_str: String = row.get(0)?;
        let uuid: Uuid = uuid_str.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bad UUID: {e}"),
                )),
            )
        })?;
        let created_str: String = row.get(6)?;
        let created_at: DateTime<Utc> = created_str.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bad timestamp: {e}"),
                )),
            )
        })?;
        Ok(Account {
            uuid,
            email: row.get(1)?,
            org_uuid: row.get(2)?,
            org_name: row.get(3)?,
            subscription_type: row.get(4)?,
            rate_limit_tier: row.get(5)?,
            created_at,
            last_cli_switch: row
                .get::<_, Option<String>>(7)?
                .and_then(|s| s.parse().ok()),
            last_desktop_switch: row
                .get::<_, Option<String>>(8)?
                .and_then(|s| s.parse().ok()),
            has_cli_credentials: row.get(9)?,
            has_desktop_profile: row.get(10)?,
            is_cli_active: active_cli.as_ref() == Some(&uuid_str),
            is_desktop_active: active_desktop.as_ref() == Some(&uuid_str),
            verified_email: row.get::<_, Option<String>>(11)?,
            verified_at: row
                .get::<_, Option<String>>(12)?
                .and_then(|s| s.parse().ok()),
            verify_status: row
                .get::<_, Option<String>>(13)?
                .unwrap_or_else(|| "never".to_string()),
        })
    }

    // `update_verification` moved to `crate::account_verification` —
    // it's a sibling `impl AccountStore` block in that file.

    pub fn list(&self) -> SqlResult<Vec<Account>> {
        let active_cli = self.active_cli_uuid()?;
        let active_desktop = self.active_desktop_uuid()?;

        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT uuid, email, org_uuid, org_name, \
             subscription_type, rate_limit_tier, created_at, \
             last_cli_switch, last_desktop_switch, \
             has_cli_credentials, has_desktop_profile, \
             verified_email, verified_at, verify_status \
             FROM accounts ORDER BY email",
        )?;
        let rows = stmt.query_map([], |row| {
            Self::row_to_account(row, &active_cli, &active_desktop)
        })?;
        rows.collect()
    }

    pub fn find_by_email(&self, email: &str) -> SqlResult<Option<Account>> {
        let active_cli = self.active_cli_uuid()?;
        let active_desktop = self.active_desktop_uuid()?;

        self.db()
            .query_row(
                "SELECT uuid, email, org_uuid, org_name, \
                 subscription_type, rate_limit_tier, created_at, \
                 last_cli_switch, last_desktop_switch, \
                 has_cli_credentials, has_desktop_profile, \
                 verified_email, verified_at, verify_status \
                 FROM accounts WHERE email = ?1",
                params![email],
                |row| Self::row_to_account(row, &active_cli, &active_desktop),
            )
            .optional()
    }

    pub fn find_by_uuid(&self, uuid: Uuid) -> SqlResult<Option<Account>> {
        let active_cli = self.active_cli_uuid()?;
        let active_desktop = self.active_desktop_uuid()?;

        self.db()
            .query_row(
                "SELECT uuid, email, org_uuid, org_name, \
                 subscription_type, rate_limit_tier, created_at, \
                 last_cli_switch, last_desktop_switch, \
                 has_cli_credentials, has_desktop_profile, \
                 verified_email, verified_at, verify_status \
                 FROM accounts WHERE uuid = ?1",
                params![uuid.to_string()],
                |row| Self::row_to_account(row, &active_cli, &active_desktop),
            )
            .optional()
    }

    pub fn insert(&self, account: &Account) -> SqlResult<()> {
        self.db().execute(
            "INSERT INTO accounts (uuid, email, org_uuid, org_name, \
             subscription_type, rate_limit_tier, created_at, \
             has_cli_credentials, has_desktop_profile, \
             verified_email, verified_at, verify_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                account.uuid.to_string(),
                account.email,
                account.org_uuid,
                account.org_name,
                account.subscription_type,
                account.rate_limit_tier,
                account.created_at.to_rfc3339(),
                account.has_cli_credentials,
                account.has_desktop_profile,
                account.verified_email,
                account.verified_at.map(|t| t.to_rfc3339()),
                account.verify_status,
            ],
        )?;
        Ok(())
    }

    pub fn remove(&self, uuid: Uuid) -> SqlResult<()> {
        self.db().execute(
            "DELETE FROM accounts WHERE uuid = ?1",
            params![uuid.to_string()],
        )?;
        Ok(())
    }

    pub fn active_cli_uuid(&self) -> SqlResult<Option<String>> {
        self.active_uuid("active_cli")
    }

    pub fn set_active_cli(&self, uuid: Uuid) -> SqlResult<()> {
        self.set_active("active_cli", "last_cli_switch", uuid)
    }

    pub fn clear_active_cli(&self) -> SqlResult<()> {
        self.clear_active("active_cli")
    }

    pub fn active_desktop_uuid(&self) -> SqlResult<Option<String>> {
        self.active_uuid("active_desktop")
    }

    pub fn set_active_desktop(&self, uuid: Uuid) -> SqlResult<()> {
        self.set_active("active_desktop", "last_desktop_switch", uuid)
    }

    pub fn clear_active_desktop(&self) -> SqlResult<()> {
        self.clear_active("active_desktop")
    }

    fn active_uuid(&self, key: &str) -> SqlResult<Option<String>> {
        self.db()
            .query_row(
                &format!("SELECT value FROM state WHERE key = '{key}'"),
                [],
                |r| r.get(0),
            )
            .optional()
    }

    fn set_active(&self, key: &str, ts_column: &str, uuid: Uuid) -> SqlResult<()> {
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        let sql = format!("UPDATE accounts SET {ts_column} = ?1 WHERE uuid = ?2");
        let updated = tx.execute(&sql, params![Utc::now().to_rfc3339(), uuid.to_string()])?;
        if updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.execute(
            &format!("INSERT OR REPLACE INTO state (key, value) VALUES ('{key}', ?1)"),
            params![uuid.to_string()],
        )?;
        tx.commit()
    }

    fn clear_active(&self, key: &str) -> SqlResult<()> {
        self.db()
            .execute(&format!("DELETE FROM state WHERE key = '{key}'"), [])?;
        Ok(())
    }

    pub fn update_credentials_flag(&self, uuid: Uuid, has: bool) -> SqlResult<()> {
        self.db().execute(
            "UPDATE accounts SET has_cli_credentials = ?1 WHERE uuid = ?2",
            params![has, uuid.to_string()],
        )?;
        Ok(())
    }

    pub fn update_desktop_profile_flag(&self, uuid: Uuid, has: bool) -> SqlResult<()> {
        self.db().execute(
            "UPDATE accounts SET has_desktop_profile = ?1 WHERE uuid = ?2",
            params![has, uuid.to_string()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    uuid TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    org_uuid TEXT,
    org_name TEXT,
    subscription_type TEXT,
    rate_limit_tier TEXT,
    created_at TEXT NOT NULL,
    last_cli_switch TEXT,
    last_desktop_switch TEXT,
    has_cli_credentials INTEGER NOT NULL DEFAULT 0,
    has_desktop_profile INTEGER NOT NULL DEFAULT 0,
    verified_email TEXT,
    verified_at TEXT,
    verify_status TEXT NOT NULL DEFAULT 'never'
);

CREATE TABLE IF NOT EXISTS state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";
