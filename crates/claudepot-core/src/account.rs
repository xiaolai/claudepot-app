use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqlResult};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Account {
    pub uuid: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_cli_switch: Option<DateTime<Utc>>,
    pub last_desktop_switch: Option<DateTime<Utc>>,
    pub has_cli_credentials: bool,
    pub has_desktop_profile: bool,
}

pub struct AccountStore {
    db: Connection,
}

impl AccountStore {
    pub fn open(path: &std::path::Path) -> SqlResult<Self> {
        std::fs::create_dir_all(path.parent().unwrap_or(path))
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let db = Connection::open(path)?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    pub fn list(&self) -> SqlResult<Vec<Account>> {
        let mut stmt = self.db.prepare(
            "SELECT uuid, display_name, email, org_uuid, org_name, \
             subscription_type, rate_limit_tier, created_at, \
             last_cli_switch, last_desktop_switch, \
             has_cli_credentials, has_desktop_profile \
             FROM accounts ORDER BY display_name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Account {
                uuid: row.get::<_, String>(0)?.parse().unwrap_or_default(),
                display_name: row.get(1)?,
                email: row.get(2)?,
                org_uuid: row.get(3)?,
                org_name: row.get(4)?,
                subscription_type: row.get(5)?,
                rate_limit_tier: row.get(6)?,
                created_at: row
                    .get::<_, String>(7)?
                    .parse()
                    .unwrap_or_default(),
                last_cli_switch: row.get::<_, Option<String>>(8)?.and_then(|s| s.parse().ok()),
                last_desktop_switch: row.get::<_, Option<String>>(9)?.and_then(|s| s.parse().ok()),
                has_cli_credentials: row.get(10)?,
                has_desktop_profile: row.get(11)?,
            })
        })?;
        rows.collect()
    }

    pub fn insert(&self, account: &Account) -> SqlResult<()> {
        self.db.execute(
            "INSERT INTO accounts (uuid, display_name, email, org_uuid, org_name, \
             subscription_type, rate_limit_tier, created_at, \
             has_cli_credentials, has_desktop_profile) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                account.uuid.to_string(),
                account.display_name,
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

    pub fn active_cli(&self) -> SqlResult<Option<String>> {
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

    pub fn active_desktop(&self) -> SqlResult<Option<String>> {
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
}

use rusqlite::OptionalExtension;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    uuid TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    email TEXT,
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
