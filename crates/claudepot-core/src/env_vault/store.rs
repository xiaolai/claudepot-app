//! SQLite-backed registry for named env secrets — the local vault.
//!
//! File: `~/.claudepot/env-vault.db` (overridable via
//! `CLAUDEPOT_DATA_DIR`), 0600 on Unix and user-only DACL on Windows
//! (via `crate::secure_perms`). The secret lives in the
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
use crate::db_pragmas::apply_standard_pragmas;
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
/// `abcd…wxyz`. The preview persists in the DB, crosses IPC, and
/// renders in the UI, so disclosure scales with length:
///
/// * under 16 chars — fully masked (`••••`). The old 4+4 rule left a
///   9-char secret with a single masked character; short passwords
///   and PIN-like values must never be near-fully disclosed.
/// * 16+ chars — head + tail of `min(4, len / 8)` chars per side, so
///   at most 25% of the secret is revealed and at least 12 chars stay
///   masked. Real API keys (40+ chars) keep the familiar 4+4 shape.
///
/// Mirrors `keys::format::safe_generic_preview` — change both
/// together.
///
/// Never collects the whole secret into an owned buffer — only the
/// short head and tail are ever materialized, so the plaintext
/// doesn't get a second heap copy that outlives the call.
pub fn secret_preview(secret: &str) -> String {
    let char_count = secret.chars().count();
    if char_count < 16 {
        return "••••".to_string();
    }
    let per_side = 4.min(char_count / 8);
    let head: String = secret.chars().take(per_side).collect();
    let tail: String = secret.chars().skip(char_count - per_side).collect();
    format!("{head}…{tail}")
}

/// Recompute every stored `secret_preview` that no longer matches the
/// current preview policy. The preview column persists across
/// releases, so rows written under the old over-revealing 4+4 rule
/// (a 9-char secret kept only 1 char masked) would otherwise keep
/// disclosing short secrets in the UI until the next `update`.
/// Idempotent; the vault is small, so a full pass per open is cheap.
fn normalize_previews(db: &Connection) -> Result<(), VaultError> {
    let mut stale: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = db.prepare("SELECT name, secret, secret_preview FROM env_secrets")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (name, secret, stored) = row?;
            let want = secret_preview(&secret);
            if want != stored {
                stale.push((name, want));
            }
            // `secret` drops here — this module is the value's
            // at-rest home; no copy leaves the function.
        }
    }
    for (name, preview) in stale {
        db.execute(
            "UPDATE env_secrets SET secret_preview = ?1 WHERE name = ?2",
            params![preview, name],
        )?;
    }
    Ok(())
}

pub struct VaultStore {
    db: Mutex<Connection>,
}

impl VaultStore {
    pub fn open(path: &Path) -> Result<Self, VaultError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Pre-create the DB file with user-only perms BEFORE rusqlite
        // opens it at umask defaults — same M9 fix as
        // `session_index::SessionIndex::open`. The WAL/SHM sidecars
        // inherit the main file's mode, so this closes the window
        // that matters.
        crate::secure_perms::precreate_user_only(path);

        let db = Connection::open(path)?;
        apply_standard_pragmas(&db)?;
        db.execute_batch(SCHEMA)?;
        normalize_previews(&db)?;

        // 0600 on Unix, user-only DACL on Windows — backstop for the
        // pre-existing-file case the pre-create doesn't touch.
        crate::secure_perms::harden_user_only(path)?;
        crate::secure_perms::harden_user_only(&path.with_extension("db-wal"))?;
        crate::secure_perms::harden_user_only(&path.with_extension("db-shm"))?;

        Ok(Self { db: Mutex::new(db) })
    }

    /// Recover from a poisoned mutex via [`crate::sync::recover_lock`];
    /// see that helper for the project-wide poisoning policy.
    fn db(&self) -> MutexGuard<'_, Connection> {
        crate::sync::recover_lock(&self.db, "env vault")
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
        // The audit case: the old 4+4 rule rendered a 9-char secret
        // as `1234…6789` — 8 of 9 chars disclosed. Anything under 16
        // chars is now fully masked.
        assert_eq!(secret_preview("123456789"), "••••");
        assert_eq!(secret_preview("123456789012345"), "••••");
        // 16–23 chars → 2+2; 24–31 → 3+3; 32+ → 4+4.
        assert_eq!(secret_preview("1234567890123456"), "12…56");
        assert_eq!(secret_preview("sk-ant-api03-abcdef"), "sk…ef");
        assert_eq!(secret_preview("abcdefghijklmnopqrstuvwx"), "abc…vwx");
        assert_eq!(
            secret_preview("abcdefghijklmnopqrstuvwxyz012345"),
            "abcd…2345"
        );
    }

    #[test]
    fn secret_preview_never_reveals_more_than_a_quarter() {
        // Behavioral lock for the disclosure budget: at every length,
        // the preview reveals at most len/4 chars and leaves at least
        // 12 masked — or is fully masked below 16.
        for len in 1..=64usize {
            let secret: String = "abcdefgh".chars().cycle().take(len).collect();
            let preview = secret_preview(&secret);
            if len < 16 {
                assert_eq!(preview, "••••", "len {len} must be fully masked");
                continue;
            }
            let revealed = preview.chars().filter(|c| *c != '…').count();
            assert!(
                revealed * 4 <= len,
                "len {len}: revealed {revealed} chars exceeds 25%"
            );
            assert!(
                len - revealed >= 12,
                "len {len}: only {} chars masked",
                len - revealed
            );
        }
    }

    #[test]
    fn insert_and_list_round_trips() {
        let (store, _d) = tmp_store();
        let rec = store
            .insert("OPENAI_API_KEY", "sk-proj-secret-value")
            .unwrap();
        assert_eq!(rec.name, "OPENAI_API_KEY");
        // 20 chars → 2+2 under the length-scaled disclosure rule.
        assert_eq!(rec.secret_preview, "sk…ue");
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "OPENAI_API_KEY");
        assert_eq!(
            store.reveal("OPENAI_API_KEY").unwrap(),
            "sk-proj-secret-value"
        );
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
        // 16 chars → 2+2 under the length-scaled disclosure rule.
        assert_eq!(updated.secret_preview, "br…et");
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

    /// Rows persisted under the old 4+4 preview rule must be
    /// re-masked on open — the stored preview is what the UI shows,
    /// so a short secret written by an older build would otherwise
    /// keep leaking 8 of its chars forever.
    #[test]
    fn open_normalizes_stale_previews_from_older_builds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env-vault.db");
        {
            let store = VaultStore::open(&path).unwrap();
            store.insert("SHORT_PIN", "123456789").unwrap();
            // Simulate the old build's over-revealing stored preview.
            store
                .db()
                .execute(
                    "UPDATE env_secrets SET secret_preview = '1234…6789' \
                     WHERE name = 'SHORT_PIN'",
                    [],
                )
                .unwrap();
        }
        let store = VaultStore::open(&path).unwrap();
        assert_eq!(store.get("SHORT_PIN").unwrap().secret_preview, "••••");
        // And the secret itself is untouched.
        assert_eq!(store.reveal("SHORT_PIN").unwrap(), "123456789");
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
