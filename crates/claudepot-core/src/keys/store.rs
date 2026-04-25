//! SQLite-backed registry for API keys and OAuth tokens.
//!
//! File: `~/.claudepot/keys.db` (overridable via `CLAUDEPOT_DATA_DIR`).
//! Two independent tables; no FK into `accounts.db` since it is a
//! separate database file. `account_uuid` is a soft reference the
//! higher layers join at read time.

use super::error::KeyError;
use super::types::{ApiKey, OauthToken};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use uuid::Uuid;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS api_keys (
    uuid              TEXT PRIMARY KEY,
    label             TEXT NOT NULL,
    token_preview     TEXT NOT NULL,
    account_uuid      TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    last_probed_at    TEXT,
    last_probe_status TEXT,
    secret            TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS oauth_tokens (
    uuid              TEXT PRIMARY KEY,
    label             TEXT NOT NULL,
    token_preview     TEXT NOT NULL,
    account_uuid      TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    last_probed_at    TEXT,
    last_probe_status TEXT,
    secret            TEXT NOT NULL DEFAULT ''
);
"#;

/// Idempotently add the `secret` column to a pre-existing table, then
/// auto-purge any row whose secret is blank. Older builds stored the
/// secret in the OS Keychain, so the DB row alone is unrecoverable —
/// a blank-secret row is a stranded artifact and is dropped on open.
fn migrate_add_secret_and_purge(
    db: &Connection,
    table: &str,
) -> Result<(), KeyError> {
    let add_col = format!("ALTER TABLE {table} ADD COLUMN secret TEXT NOT NULL DEFAULT ''");
    match db.execute(&add_col, []) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
            if msg.contains("duplicate column name") => {}
        Err(e) => return Err(KeyError::Sql(e)),
    }
    let purge = format!("DELETE FROM {table} WHERE secret IS NULL OR secret = ''");
    db.execute(&purge, [])?;
    Ok(())
}

pub struct KeyStore {
    db: Mutex<Connection>,
}

impl KeyStore {
    pub fn open(path: &Path) -> Result<Self, KeyError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                KeyError::Sql(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?;
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(SCHEMA)?;
        migrate_add_secret_and_purge(&db, "api_keys")?;
        migrate_add_secret_and_purge(&db, "oauth_tokens")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let secure = |p: &Path| -> Result<(), KeyError> {
                if p.exists() {
                    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600))
                        .map_err(|e| {
                            KeyError::Sql(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
                        })?;
                }
                Ok(())
            };
            secure(path)?;
            secure(&path.with_extension("db-wal"))?;
            secure(&path.with_extension("db-shm"))?;
        }

        Ok(Self { db: Mutex::new(db) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().expect("keys store mutex poisoned")
    }

    // ──────────────────────── API keys ────────────────────────

    pub fn list_api_keys(&self) -> Result<Vec<ApiKey>, KeyError> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT uuid, label, token_preview, account_uuid, \
                    created_at, last_probed_at, last_probe_status \
             FROM api_keys ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_api_key)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn insert_api_key(
        &self,
        label: &str,
        token_preview: &str,
        account_uuid: Uuid,
        secret: &str,
    ) -> Result<ApiKey, KeyError> {
        let key = ApiKey {
            uuid: Uuid::new_v4(),
            label: label.to_string(),
            token_preview: token_preview.to_string(),
            account_uuid,
            created_at: Utc::now(),
            last_probed_at: None,
            last_probe_status: None,
        };
        self.db().execute(
            "INSERT INTO api_keys \
             (uuid, label, token_preview, account_uuid, created_at, last_probed_at, last_probe_status, secret) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6)",
            params![
                key.uuid.to_string(),
                key.label,
                key.token_preview,
                key.account_uuid.to_string(),
                key.created_at.to_rfc3339(),
                secret,
            ],
        )?;
        Ok(key)
    }

    /// Look up a single API key by uuid. Mirrors `find_oauth_token` —
    /// the IPC `key_api_copy` path needs both the row metadata
    /// (label / preview, for the receipt) and the secret in one
    /// pass. `Ok(None)` means no row; not found is not an error
    /// here so callers can decide whether to treat it as one.
    pub fn find_api_key(&self, uuid: Uuid) -> Result<Option<ApiKey>, KeyError> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT uuid, label, token_preview, account_uuid, \
                    created_at, last_probed_at, last_probe_status \
             FROM api_keys WHERE uuid = ?1",
        )?;
        let row = stmt
            .query_row(params![uuid.to_string()], row_to_api_key)
            .optional()?;
        Ok(row)
    }

    pub fn find_api_secret(&self, uuid: Uuid) -> Result<String, KeyError> {
        let db = self.db();
        let secret: Option<String> = db
            .query_row(
                "SELECT secret FROM api_keys WHERE uuid = ?1",
                params![uuid.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        secret
            .filter(|s| !s.is_empty())
            .ok_or_else(|| KeyError::NotFound(uuid.to_string()))
    }

    pub fn remove_api_key(&self, uuid: Uuid) -> Result<(), KeyError> {
        let deleted = self.db().execute(
            "DELETE FROM api_keys WHERE uuid = ?1",
            params![uuid.to_string()],
        )?;
        if deleted == 0 {
            return Err(KeyError::NotFound(uuid.to_string()));
        }
        Ok(())
    }

    pub fn rename_api_key(&self, uuid: Uuid, label: &str) -> Result<(), KeyError> {
        let updated = self.db().execute(
            "UPDATE api_keys SET label = ?1 WHERE uuid = ?2",
            params![label, uuid.to_string()],
        )?;
        if updated == 0 {
            return Err(KeyError::NotFound(uuid.to_string()));
        }
        Ok(())
    }

    // API keys have no probe command today — per-key usage isn't
    // available through the public API. The `last_probed_at` /
    // `last_probe_status` columns on the row remain reserved so a
    // future probe path can fill them without a schema migration.

    // ──────────────────────── OAuth tokens ────────────────────────

    pub fn list_oauth_tokens(&self) -> Result<Vec<OauthToken>, KeyError> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT uuid, label, token_preview, account_uuid, \
                    created_at, last_probed_at, last_probe_status \
             FROM oauth_tokens ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_oauth_token)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn find_oauth_token(&self, uuid: Uuid) -> Result<Option<OauthToken>, KeyError> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT uuid, label, token_preview, account_uuid, \
                    created_at, last_probed_at, last_probe_status \
             FROM oauth_tokens WHERE uuid = ?1",
        )?;
        let row = stmt
            .query_row(params![uuid.to_string()], row_to_oauth_token)
            .optional()?;
        Ok(row)
    }

    pub fn insert_oauth_token(
        &self,
        label: &str,
        token_preview: &str,
        account_uuid: Uuid,
        secret: &str,
    ) -> Result<OauthToken, KeyError> {
        let token = OauthToken {
            uuid: Uuid::new_v4(),
            label: label.to_string(),
            token_preview: token_preview.to_string(),
            account_uuid,
            created_at: Utc::now(),
            last_probed_at: None,
            last_probe_status: None,
        };
        self.db().execute(
            "INSERT INTO oauth_tokens \
             (uuid, label, token_preview, account_uuid, created_at, last_probed_at, last_probe_status, secret) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6)",
            params![
                token.uuid.to_string(),
                token.label,
                token.token_preview,
                token.account_uuid.to_string(),
                token.created_at.to_rfc3339(),
                secret,
            ],
        )?;
        Ok(token)
    }

    pub fn find_oauth_secret(&self, uuid: Uuid) -> Result<String, KeyError> {
        let db = self.db();
        let secret: Option<String> = db
            .query_row(
                "SELECT secret FROM oauth_tokens WHERE uuid = ?1",
                params![uuid.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        secret
            .filter(|s| !s.is_empty())
            .ok_or_else(|| KeyError::NotFound(uuid.to_string()))
    }

    pub fn remove_oauth_token(&self, uuid: Uuid) -> Result<(), KeyError> {
        let deleted = self.db().execute(
            "DELETE FROM oauth_tokens WHERE uuid = ?1",
            params![uuid.to_string()],
        )?;
        if deleted == 0 {
            return Err(KeyError::NotFound(uuid.to_string()));
        }
        Ok(())
    }

    pub fn rename_oauth_token(&self, uuid: Uuid, label: &str) -> Result<(), KeyError> {
        let updated = self.db().execute(
            "UPDATE oauth_tokens SET label = ?1 WHERE uuid = ?2",
            params![label, uuid.to_string()],
        )?;
        if updated == 0 {
            return Err(KeyError::NotFound(uuid.to_string()));
        }
        Ok(())
    }

    pub fn update_oauth_token_probe(
        &self,
        uuid: Uuid,
        status: &str,
    ) -> Result<(), KeyError> {
        let now = Utc::now().to_rfc3339();
        self.db().execute(
            "UPDATE oauth_tokens SET last_probed_at = ?1, last_probe_status = ?2 WHERE uuid = ?3",
            params![now, status, uuid.to_string()],
        )?;
        Ok(())
    }
}

fn parse_uuid(s: &str, col: usize) -> rusqlite::Result<Uuid> {
    s.parse::<Uuid>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            col,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("bad uuid: {e}"),
            )),
        )
    })
}

fn parse_ts(s: &str, col: usize) -> rusqlite::Result<DateTime<Utc>> {
    s.parse::<DateTime<Utc>>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            col,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("bad timestamp: {e}"),
            )),
        )
    })
}

fn row_to_api_key(row: &rusqlite::Row) -> rusqlite::Result<ApiKey> {
    let uuid_s: String = row.get(0)?;
    let account_s: String = row.get(3)?;
    let created_s: String = row.get(4)?;
    Ok(ApiKey {
        uuid: parse_uuid(&uuid_s, 0)?,
        label: row.get(1)?,
        token_preview: row.get(2)?,
        account_uuid: parse_uuid(&account_s, 3)?,
        created_at: parse_ts(&created_s, 4)?,
        last_probed_at: row
            .get::<_, Option<String>>(5)?
            .and_then(|s| s.parse().ok()),
        last_probe_status: row.get::<_, Option<String>>(6)?,
    })
}

fn row_to_oauth_token(row: &rusqlite::Row) -> rusqlite::Result<OauthToken> {
    let uuid_s: String = row.get(0)?;
    let account_s: String = row.get(3)?;
    let created_s: String = row.get(4)?;
    Ok(OauthToken {
        uuid: parse_uuid(&uuid_s, 0)?,
        label: row.get(1)?,
        token_preview: row.get(2)?,
        account_uuid: parse_uuid(&account_s, 3)?,
        created_at: parse_ts(&created_s, 4)?,
        last_probed_at: row
            .get::<_, Option<String>>(5)?
            .and_then(|s| s.parse().ok()),
        last_probe_status: row.get::<_, Option<String>>(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (KeyStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::open(&dir.path().join("keys.db")).unwrap();
        (store, dir)
    }

    #[test]
    fn insert_and_list_api_key() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let key = store
            .insert_api_key(
                "work",
                "sk-ant-api03-Abc…xyz",
                account,
                "sk-ant-api03-abcdef",
            )
            .unwrap();
        let list = store.list_api_keys().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uuid, key.uuid);
        assert_eq!(list[0].account_uuid, account);
        assert_eq!(store.find_api_secret(key.uuid).unwrap(), "sk-ant-api03-abcdef");
    }

    #[test]
    fn insert_and_list_oauth_token_requires_account() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let token = store
            .insert_oauth_token(
                "home",
                "sk-ant-oat01-Aaa…zzz",
                account,
                "sk-ant-oat01-full-token",
            )
            .unwrap();
        let list = store.list_oauth_tokens().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uuid, token.uuid);
        assert_eq!(list[0].account_uuid, account);
        assert_eq!(
            store.find_oauth_secret(token.uuid).unwrap(),
            "sk-ant-oat01-full-token"
        );
    }

    #[test]
    fn find_secret_returns_not_found_for_unknown_uuid() {
        let (store, _dir) = tmp_store();
        assert!(matches!(
            store.find_api_secret(Uuid::new_v4()),
            Err(KeyError::NotFound(_))
        ));
        assert!(matches!(
            store.find_oauth_secret(Uuid::new_v4()),
            Err(KeyError::NotFound(_))
        ));
    }

    #[test]
    fn remove_returns_not_found_for_unknown_uuid() {
        let (store, _dir) = tmp_store();
        let result = store.remove_api_key(Uuid::new_v4());
        assert!(matches!(result, Err(KeyError::NotFound(_))));
    }

    #[test]
    fn rename_api_key_persists_new_label() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let key = store
            .insert_api_key("old", "sk-ant-api03-a…z", account, "sk-ant-api03-a")
            .unwrap();
        store.rename_api_key(key.uuid, "new").unwrap();
        let list = store.list_api_keys().unwrap();
        assert_eq!(list[0].label, "new");
    }

    #[test]
    fn rename_oauth_token_persists_new_label() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let tok = store
            .insert_oauth_token("old", "sk-ant-oat01-a…z", account, "sk-ant-oat01-a")
            .unwrap();
        store.rename_oauth_token(tok.uuid, "new").unwrap();
        let list = store.list_oauth_tokens().unwrap();
        assert_eq!(list[0].label, "new");
    }

    #[test]
    fn rename_returns_not_found_for_unknown_uuid() {
        let (store, _dir) = tmp_store();
        assert!(matches!(
            store.rename_api_key(Uuid::new_v4(), "x"),
            Err(KeyError::NotFound(_))
        ));
        assert!(matches!(
            store.rename_oauth_token(Uuid::new_v4(), "x"),
            Err(KeyError::NotFound(_))
        ));
    }

    #[test]
    fn update_probe_persists_status() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let token = store
            .insert_oauth_token("t", "sk-ant-oat01-a…z", account, "sk-ant-oat01-x")
            .unwrap();
        store.update_oauth_token_probe(token.uuid, "ok").unwrap();
        let found = store.find_oauth_token(token.uuid).unwrap().unwrap();
        assert_eq!(found.last_probe_status.as_deref(), Some("ok"));
        assert!(found.last_probed_at.is_some());
    }

    #[test]
    fn ordering_is_newest_first() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let _a = store
            .insert_api_key("first", "sk-ant-api03-a…z", account, "sk-ant-api03-a")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = store
            .insert_api_key("second", "sk-ant-api03-b…y", account, "sk-ant-api03-b")
            .unwrap();
        let list = store.list_api_keys().unwrap();
        assert_eq!(list[0].uuid, b.uuid, "newest row should be first");
    }

    /// An older build stored secrets in the OS Keychain and left the
    /// DB row without a secret column. On migration, `secret` is
    /// added with default `''`, and blank-secret rows are auto-purged
    /// so the UI never shows an unrecoverable row.
    #[test]
    fn migration_autopurges_rows_with_blank_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        // Simulate the pre-migration shape: tables exist with no
        // secret column, one row inserted.
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE api_keys (
                    uuid TEXT PRIMARY KEY, label TEXT NOT NULL,
                    token_preview TEXT NOT NULL, account_uuid TEXT NOT NULL,
                    created_at TEXT NOT NULL, last_probed_at TEXT,
                    last_probe_status TEXT
                );
                CREATE TABLE oauth_tokens (
                    uuid TEXT PRIMARY KEY, label TEXT NOT NULL,
                    token_preview TEXT NOT NULL, account_uuid TEXT NOT NULL,
                    created_at TEXT NOT NULL, last_probed_at TEXT,
                    last_probe_status TEXT
                );",
            )
            .unwrap();
            db.execute(
                "INSERT INTO api_keys (uuid, label, token_preview, account_uuid, created_at) \
                 VALUES (?1, 'legacy', 'sk…x', ?2, '2025-01-01T00:00:00Z')",
                params![Uuid::new_v4().to_string(), Uuid::new_v4().to_string()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO oauth_tokens (uuid, label, token_preview, account_uuid, created_at) \
                 VALUES (?1, 'legacy', 'sk…x', ?2, '2025-01-01T00:00:00Z')",
                params![Uuid::new_v4().to_string(), Uuid::new_v4().to_string()],
            )
            .unwrap();
        }
        // Reopen via the real path — migration runs, purges orphans.
        let store = KeyStore::open(&path).unwrap();
        assert_eq!(store.list_api_keys().unwrap().len(), 0);
        assert_eq!(store.list_oauth_tokens().unwrap().len(), 0);
    }
}
