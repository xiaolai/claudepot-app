//! SQLite-backed registry for named env secrets — the local vault.
//!
//! File: `~/.claudepot/env-vault.db` (overridable via
//! `CLAUDEPOT_DATA_DIR`), 0600 on Unix. The secret lives in the
//! `secret` column, mirroring `keys::store` — that module migrated
//! *away* from the OS Keychain to an at-rest 0600 SQLite column, so
//! the env vault follows the same de-facto pattern rather than the
//! older `keyring`-crate note in `rules/architecture.md`.
//!
//! Secrets are keyed by `name` (the env key, e.g. `OPENAI_API_KEY`)
//! — the user's mental model is "one named secret", not a uuid.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use super::error::VaultError;
use crate::env_vault::env_file::is_valid_key;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS env_secrets (
    name           TEXT PRIMARY KEY,
    secret_preview TEXT NOT NULL,
    secret         TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
"#;

/// Metadata for one vault secret. The plaintext `secret` is never on
/// this struct — it leaves the store only through [`VaultStore::reveal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSecret {
    pub name: String,
    pub secret_preview: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A short, non-reversible preview of a secret for display:
/// `abcd…wxyz`. Secrets of 8 chars or fewer are fully masked so a
/// short secret can't be reconstructed from the preview.
pub fn secret_preview(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "••••".to_string();
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{head}…{tail}")
}

pub struct VaultStore {
    db: Mutex<Connection>,
}

impl VaultStore {
    pub fn open(path: &Path) -> Result<Self, VaultError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(SCHEMA)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let secure = |p: &Path| -> Result<(), VaultError> {
                if p.exists() {
                    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600))?;
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
        self.db.lock().expect("env vault mutex poisoned")
    }

    /// All vault secrets, newest-updated first. No plaintext crosses.
    pub fn list(&self) -> Result<Vec<VaultSecret>, VaultError> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT name, secret_preview, created_at, updated_at \
             FROM env_secrets ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_secret)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Insert a new named secret. Errors if the name is not a valid
    /// env key, or a secret with that name already exists (use
    /// [`update`](Self::update) to change an existing one).
    pub fn insert(&self, name: &str, secret: &str) -> Result<VaultSecret, VaultError> {
        if !is_valid_key(name) {
            return Err(VaultError::InvalidName(name.to_string()));
        }
        let now = Utc::now();
        let record = VaultSecret {
            name: name.to_string(),
            secret_preview: secret_preview(secret),
            created_at: now,
            updated_at: now,
        };
        let result = self.db().execute(
            "INSERT INTO env_secrets (name, secret_preview, secret, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.name,
                record.secret_preview,
                secret,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
            ],
        );
        match result {
            Ok(_) => Ok(record),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(VaultError::DuplicateName(name.to_string()))
            }
            Err(e) => Err(VaultError::Sql(e)),
        }
    }

    /// Replace the secret value for an existing name, refreshing the
    /// preview and `updated_at`. Errors if the name doesn't exist.
    pub fn update(&self, name: &str, secret: &str) -> Result<VaultSecret, VaultError> {
        let now = Utc::now();
        let preview = secret_preview(secret);
        let changed = self.db().execute(
            "UPDATE env_secrets SET secret = ?1, secret_preview = ?2, updated_at = ?3 \
             WHERE name = ?4",
            params![secret, preview, now.to_rfc3339(), name],
        )?;
        if changed == 0 {
            return Err(VaultError::NotFound(name.to_string()));
        }
        self.get(name)
    }

    /// Metadata for one secret by name. Errors if not found.
    pub fn get(&self, name: &str) -> Result<VaultSecret, VaultError> {
        let db = self.db();
        let row = db
            .query_row(
                "SELECT name, secret_preview, created_at, updated_at \
                 FROM env_secrets WHERE name = ?1",
                params![name],
                row_to_secret,
            )
            .optional()?;
        row.ok_or_else(|| VaultError::NotFound(name.to_string()))
    }

    /// The plaintext secret for `name`. The single egress point for a
    /// secret value — callers (the Tauri layer) write it to the OS
    /// clipboard or into a `.env` file and zeroize their copy.
    pub fn reveal(&self, name: &str) -> Result<String, VaultError> {
        let db = self.db();
        let secret: Option<String> = db
            .query_row(
                "SELECT secret FROM env_secrets WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        secret.ok_or_else(|| VaultError::NotFound(name.to_string()))
    }

    /// Delete a secret by name. Errors if it didn't exist.
    pub fn delete(&self, name: &str) -> Result<(), VaultError> {
        let deleted = self
            .db()
            .execute("DELETE FROM env_secrets WHERE name = ?1", params![name])?;
        if deleted == 0 {
            return Err(VaultError::NotFound(name.to_string()));
        }
        Ok(())
    }
}

fn row_to_secret(row: &rusqlite::Row) -> rusqlite::Result<VaultSecret> {
    let created: String = row.get(2)?;
    let updated: String = row.get(3)?;
    Ok(VaultSecret {
        name: row.get(0)?,
        secret_preview: row.get(1)?,
        created_at: parse_ts(&created, 2)?,
        updated_at: parse_ts(&updated, 3)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (VaultStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = VaultStore::open(&dir.path().join("env-vault.db")).unwrap();
        (store, dir)
    }

    #[test]
    fn secret_preview_masks_short_and_truncates_long() {
        assert_eq!(secret_preview(""), "••••");
        assert_eq!(secret_preview("12345678"), "••••");
        assert_eq!(secret_preview("sk-ant-api03-abcdef"), "sk-a…cdef");
    }

    #[test]
    fn insert_and_list_round_trips() {
        let (store, _d) = tmp_store();
        let rec = store.insert("OPENAI_API_KEY", "sk-proj-secret-value").unwrap();
        assert_eq!(rec.name, "OPENAI_API_KEY");
        assert_eq!(rec.secret_preview, "sk-p…alue");
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "OPENAI_API_KEY");
        assert_eq!(store.reveal("OPENAI_API_KEY").unwrap(), "sk-proj-secret-value");
    }

    #[test]
    fn insert_rejects_invalid_name() {
        let (store, _d) = tmp_store();
        assert!(matches!(
            store.insert("0BAD", "v"),
            Err(VaultError::InvalidName(_))
        ));
        assert!(matches!(
            store.insert("HAS SPACE", "v"),
            Err(VaultError::InvalidName(_))
        ));
    }

    #[test]
    fn insert_rejects_duplicate_name() {
        let (store, _d) = tmp_store();
        store.insert("KEY", "first-value-here").unwrap();
        assert!(matches!(
            store.insert("KEY", "second-value-here"),
            Err(VaultError::DuplicateName(_))
        ));
    }

    #[test]
    fn update_replaces_value_and_refreshes_preview() {
        let (store, _d) = tmp_store();
        store.insert("KEY", "old-secret-value").unwrap();
        let updated = store.update("KEY", "brand-new-secret").unwrap();
        assert_eq!(updated.secret_preview, "bran…cret");
        assert_eq!(store.reveal("KEY").unwrap(), "brand-new-secret");
        assert!(updated.updated_at >= updated.created_at);
    }

    #[test]
    fn update_errors_on_unknown_name() {
        let (store, _d) = tmp_store();
        assert!(matches!(
            store.update("MISSING", "v"),
            Err(VaultError::NotFound(_))
        ));
    }

    #[test]
    fn get_and_reveal_error_on_unknown_name() {
        let (store, _d) = tmp_store();
        assert!(matches!(store.get("MISSING"), Err(VaultError::NotFound(_))));
        assert!(matches!(
            store.reveal("MISSING"),
            Err(VaultError::NotFound(_))
        ));
    }

    #[test]
    fn delete_removes_and_errors_when_absent() {
        let (store, _d) = tmp_store();
        store.insert("KEY", "the-secret-value").unwrap();
        store.delete("KEY").unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(matches!(store.delete("KEY"), Err(VaultError::NotFound(_))));
    }

    #[test]
    fn list_orders_by_updated_at_desc() {
        let (store, _d) = tmp_store();
        store.insert("FIRST", "first-secret-value").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.insert("SECOND", "second-secret-value").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        // Touching FIRST moves it to the front.
        store.update("FIRST", "first-secret-updated").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list[0].name, "FIRST");
        assert_eq!(list[1].name, "SECOND");
    }

    #[cfg(unix)]
    #[test]
    fn db_file_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env-vault.db");
        let _store = VaultStore::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
