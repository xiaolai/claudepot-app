//! Persistent SQLite cache for the Sessions tab.
//!
//! Motivation: a cold `session::list_all_sessions` re-parses every
//! `.jsonl` under `~/.claude/projects/` on every call. At ~5 GB /
//! ~6 k files that's tens of seconds of CPU per tab click. This
//! module owns a `sessions.db` alongside `accounts.db` that caches
//! the fold result per file and only re-scans when `(size, mtime)`
//! changes.
//!
//! Layout
//! ------
//! - `error.rs`  — user-facing error variants
//! - `schema.rs` — DDL + `SCHEMA_VERSION` constant
//! - `diff.rs`   — pure `diff_fs_vs_db` function (Task 3)
//! - `mod.rs`    — `SessionIndex` handle + open / refresh / list_all /
//!                 rebuild
//!
//! Thread model mirrors `AccountStore`: a single `Mutex<Connection>`,
//! serialized writes, WAL so readers don't block. Contention is
//! effectively zero because GUI + CLI both fan in through one process
//! at a time.
//!
//! Safety note — this cache never contains credentials. Prompts and
//! transcript metadata are in scope though, so the DB file is chmod
//! 0600 on Unix (matching `accounts.db`).

pub mod diff;
pub mod error;
pub mod schema;

pub use error::SessionIndexError;

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Handle to the persistent session index.
///
/// Opens (or creates) `sessions.db` at the given path, applies the
/// current schema, and hands back a `Send + Sync` handle usable from
/// Tauri command handlers.
pub struct SessionIndex {
    /// `rusqlite::Connection` is `!Send` on its own; the mutex makes
    /// the struct crossable across `await` points. We never hold the
    /// lock across blocking I/O that isn't SQLite-bound, so contention
    /// stays minimal.
    db: Mutex<Connection>,
}

impl SessionIndex {
    /// Open the index at `path` (e.g. `~/.claudepot/sessions.db`).
    /// Creates the parent directory, applies the schema, and enforces
    /// 0600 perms on Unix. Idempotent — re-opening an existing DB is
    /// a no-op save for the schema check.
    pub fn open(path: &Path) -> Result<Self, SessionIndexError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        // Match accounts.db: wait up to 5 s under contention so CLI +
        // GUI co-access doesn't immediately SQLITE_BUSY.
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        apply_schema(&db)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, mode.clone())?;
            for sidecar in [path.with_extension("db-wal"), path.with_extension("db-shm")] {
                if sidecar.exists() {
                    std::fs::set_permissions(&sidecar, mode.clone())?;
                }
            }
        }

        Ok(Self { db: Mutex::new(db) })
    }

    /// Internal accessor. Kept `pub(crate)` so sibling helpers (the
    /// diff-and-refresh logic, eventually FTS) can share the lock
    /// without re-wrapping.
    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().expect("session index mutex poisoned")
    }

    /// Return the stored `meta.schema_version`. Primarily a test hook
    /// for now; future migrations will branch on it.
    pub fn schema_version(&self) -> Result<Option<String>, SessionIndexError> {
        let db = self.db();
        let v: Option<String> = db
            .query_row(
                "SELECT v FROM meta WHERE k = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .ok();
        Ok(v)
    }

    /// Row count in the `sessions` table. Test + diagnostics hook.
    pub fn row_count(&self) -> Result<i64, SessionIndexError> {
        let db = self.db();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        Ok(n)
    }
}

fn apply_schema(db: &Connection) -> Result<(), SessionIndexError> {
    db.execute_batch(schema::SCHEMA)?;
    db.execute(
        "INSERT OR IGNORE INTO meta (k, v) VALUES ('schema_version', ?1)",
        params![schema::SCHEMA_VERSION],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_creates_file_and_tables() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("sessions.db");
        let idx = SessionIndex::open(&path).unwrap();

        assert!(path.exists(), "db file should exist");
        assert_eq!(idx.row_count().unwrap(), 0);
        assert_eq!(
            idx.schema_version().unwrap().as_deref(),
            Some(schema::SCHEMA_VERSION)
        );
    }

    #[test]
    fn open_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");
        let first = SessionIndex::open(&path).unwrap();
        drop(first);
        let second = SessionIndex::open(&path).unwrap();
        assert_eq!(
            second.schema_version().unwrap().as_deref(),
            Some(schema::SCHEMA_VERSION)
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_sets_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");
        let _idx = SessionIndex::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "db file must be 0600 on Unix");
    }

    #[test]
    fn schema_tables_exist() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");
        let idx = SessionIndex::open(&path).unwrap();
        let db = idx.db();
        let names: Vec<String> = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(names.contains(&"meta".to_string()));
        assert!(names.contains(&"sessions".to_string()));
    }
}
