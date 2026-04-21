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
    last_probe_status TEXT
);

CREATE TABLE IF NOT EXISTS oauth_tokens (
    uuid              TEXT PRIMARY KEY,
    label             TEXT NOT NULL,
    token_preview     TEXT NOT NULL,
    account_uuid      TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    last_probed_at    TEXT,
    last_probe_status TEXT
);
"#;

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
             (uuid, label, token_preview, account_uuid, created_at, last_probed_at, last_probe_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
            params![
                key.uuid.to_string(),
                key.label,
                key.token_preview,
                key.account_uuid.to_string(),
                key.created_at.to_rfc3339(),
            ],
        )?;
        Ok(key)
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
             (uuid, label, token_preview, account_uuid, created_at, last_probed_at, last_probe_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
            params![
                token.uuid.to_string(),
                token.label,
                token.token_preview,
                token.account_uuid.to_string(),
                token.created_at.to_rfc3339(),
            ],
        )?;
        Ok(token)
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
            .insert_api_key("work", "sk-ant-api03-Abc…xyz", account)
            .unwrap();
        let list = store.list_api_keys().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uuid, key.uuid);
        assert_eq!(list[0].account_uuid, account);
    }

    #[test]
    fn insert_and_list_oauth_token_requires_account() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let token = store
            .insert_oauth_token("home", "sk-ant-oat01-Aaa…zzz", account)
            .unwrap();
        let list = store.list_oauth_tokens().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uuid, token.uuid);
        assert_eq!(list[0].account_uuid, account);
    }

    #[test]
    fn remove_returns_not_found_for_unknown_uuid() {
        let (store, _dir) = tmp_store();
        let result = store.remove_api_key(Uuid::new_v4());
        assert!(matches!(result, Err(KeyError::NotFound(_))));
    }

    #[test]
    fn update_probe_persists_status() {
        let (store, _dir) = tmp_store();
        let account = Uuid::new_v4();
        let token = store
            .insert_oauth_token("t", "sk-ant-oat01-a…z", account)
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
            .insert_api_key("first", "sk-ant-api03-a…z", account)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = store
            .insert_api_key("second", "sk-ant-api03-b…y", account)
            .unwrap();
        let list = store.list_api_keys().unwrap();
        assert_eq!(list[0].uuid, b.uuid, "newest row should be first");
    }
}
