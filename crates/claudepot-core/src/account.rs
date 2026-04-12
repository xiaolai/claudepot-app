use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
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
}

pub struct AccountStore {
    db: Connection,
}

impl AccountStore {
    pub fn open(path: &std::path::Path) -> SqlResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.execute_batch(SCHEMA)?;

        // Set 0600 permissions on the DB file (contains account metadata)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            // Also secure the WAL and SHM files if they exist
            let wal = path.with_extension("db-wal");
            let shm = path.with_extension("db-shm");
            if wal.exists() {
                let _ = std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o600));
            }
            if shm.exists() {
                let _ = std::fs::set_permissions(&shm, std::fs::Permissions::from_mode(0o600));
            }
        }

        Ok(Self { db })
    }

    fn row_to_account(
        row: &rusqlite::Row,
        active_cli: &Option<String>,
        active_desktop: &Option<String>,
    ) -> rusqlite::Result<Account> {
        let uuid_str: String = row.get(0)?;
        let uuid: Uuid = uuid_str.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0, rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad UUID: {e}")))
            )
        })?;
        let created_str: String = row.get(6)?;
        let created_at: DateTime<Utc> = created_str.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                6, rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad timestamp: {e}")))
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
            last_cli_switch: row.get::<_, Option<String>>(7)?.and_then(|s| s.parse().ok()),
            last_desktop_switch: row.get::<_, Option<String>>(8)?.and_then(|s| s.parse().ok()),
            has_cli_credentials: row.get(9)?,
            has_desktop_profile: row.get(10)?,
            is_cli_active: active_cli.as_ref() == Some(&uuid_str),
            is_desktop_active: active_desktop.as_ref() == Some(&uuid_str),
        })
    }

    pub fn list(&self) -> SqlResult<Vec<Account>> {
        let active_cli = self.active_cli_uuid()?;
        let active_desktop = self.active_desktop_uuid()?;

        let mut stmt = self.db.prepare(
            "SELECT uuid, email, org_uuid, org_name, \
             subscription_type, rate_limit_tier, created_at, \
             last_cli_switch, last_desktop_switch, \
             has_cli_credentials, has_desktop_profile \
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

        self.db
            .query_row(
                "SELECT uuid, email, org_uuid, org_name, \
                 subscription_type, rate_limit_tier, created_at, \
                 last_cli_switch, last_desktop_switch, \
                 has_cli_credentials, has_desktop_profile \
                 FROM accounts WHERE email = ?1",
                params![email],
                |row| Self::row_to_account(row, &active_cli, &active_desktop),
            )
            .optional()
    }

    pub fn find_by_uuid(&self, uuid: Uuid) -> SqlResult<Option<Account>> {
        let active_cli = self.active_cli_uuid()?;
        let active_desktop = self.active_desktop_uuid()?;

        self.db
            .query_row(
                "SELECT uuid, email, org_uuid, org_name, \
                 subscription_type, rate_limit_tier, created_at, \
                 last_cli_switch, last_desktop_switch, \
                 has_cli_credentials, has_desktop_profile \
                 FROM accounts WHERE uuid = ?1",
                params![uuid.to_string()],
                |row| Self::row_to_account(row, &active_cli, &active_desktop),
            )
            .optional()
    }

    pub fn insert(&self, account: &Account) -> SqlResult<()> {
        self.db.execute(
            "INSERT INTO accounts (uuid, email, org_uuid, org_name, \
             subscription_type, rate_limit_tier, created_at, \
             has_cli_credentials, has_desktop_profile) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
            ],
        )?;
        Ok(())
    }

    pub fn remove(&self, uuid: Uuid) -> SqlResult<()> {
        self.db
            .execute("DELETE FROM accounts WHERE uuid = ?1", params![uuid.to_string()])?;
        Ok(())
    }

    pub fn active_cli_uuid(&self) -> SqlResult<Option<String>> {
        self.db
            .query_row("SELECT value FROM state WHERE key = 'active_cli'", [], |r| {
                r.get(0)
            })
            .optional()
    }

    pub fn set_active_cli(&self, uuid: Uuid) -> SqlResult<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO state (key, value) VALUES ('active_cli', ?1)",
            params![uuid.to_string()],
        )?;
        self.db.execute(
            "UPDATE accounts SET last_cli_switch = ?1 WHERE uuid = ?2",
            params![Utc::now().to_rfc3339(), uuid.to_string()],
        )?;
        Ok(())
    }

    pub fn clear_active_cli(&self) -> SqlResult<()> {
        self.db.execute("DELETE FROM state WHERE key = 'active_cli'", [])?;
        Ok(())
    }

    pub fn active_desktop_uuid(&self) -> SqlResult<Option<String>> {
        self.db
            .query_row(
                "SELECT value FROM state WHERE key = 'active_desktop'",
                [],
                |r| r.get(0),
            )
            .optional()
    }

    pub fn set_active_desktop(&self, uuid: Uuid) -> SqlResult<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO state (key, value) VALUES ('active_desktop', ?1)",
            params![uuid.to_string()],
        )?;
        self.db.execute(
            "UPDATE accounts SET last_desktop_switch = ?1 WHERE uuid = ?2",
            params![Utc::now().to_rfc3339(), uuid.to_string()],
        )?;
        Ok(())
    }

    pub fn clear_active_desktop(&self) -> SqlResult<()> {
        self.db.execute("DELETE FROM state WHERE key = 'active_desktop'", [])?;
        Ok(())
    }

    pub fn update_credentials_flag(&self, uuid: Uuid, has: bool) -> SqlResult<()> {
        self.db.execute(
            "UPDATE accounts SET has_cli_credentials = ?1 WHERE uuid = ?2",
            params![has, uuid.to_string()],
        )?;
        Ok(())
    }

    pub fn update_desktop_profile_flag(&self, uuid: Uuid, has: bool) -> SqlResult<()> {
        self.db.execute(
            "UPDATE accounts SET has_desktop_profile = ?1 WHERE uuid = ?2",
            params![has, uuid.to_string()],
        )?;
        Ok(())
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
    has_desktop_profile INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";
