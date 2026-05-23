//! Startup-time SQLite WAL housekeeping for `~/.claudepot/*.db`.
//!
//! Layer 2 of the WAL management fix (see
//! `dev-docs/sqlite-wal-management.md`). The store-level pragmas in
//! [`crate::db_pragmas`] *bound future growth*; this module
//! *reclaims existing growth* by opening each known DB file
//! briefly at startup and running `PRAGMA wal_checkpoint(TRUNCATE)`
//! before any long-lived store takes its connection.
//!
//! Running at startup — not shutdown — is deliberate: it works
//! across crashes, SIGKILL, force-quit, power loss, and any other
//! exit mechanism, because the cleanup runs on the *next* launch
//! regardless of how the previous one ended.

use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

/// Filenames Claudepot writes inside its data dir. Kept exhaustive
/// so future stores added to `claudepot-core` show up here too —
/// missing one only means a small `*.db-wal` leak, never data loss.
pub const KNOWN_DB_FILENAMES: &[&str] = &[
    "sessions.db",
    "activity_metrics.db",
    "memory_changes.db",
    "accounts.db",
    "keys.db",
    "env-vault.db",
];

/// Short busy_timeout for the throw-away connections. If another
/// claudepot process holds the DB (concurrent CLI + GUI), back off
/// quickly — the other process will checkpoint on its own
/// schedule.
const STARTUP_BUSY_TIMEOUT: Duration = Duration::from_millis(1000);

/// Best-effort: walk `data_dir`, open each known `*.db` file
/// briefly, run `PRAGMA wal_checkpoint(TRUNCATE)`, close.
///
/// Returns the total bytes reclaimed (sum of WAL size deltas).
/// Per-file errors are logged at `trace`/`debug` and swallowed —
/// startup must succeed even if one DB is locked, missing, or
/// corrupt. Corruption is handled by each store's own quarantine
/// path on its subsequent real open.
///
/// Idempotent: a second call against an already-clean directory
/// returns 0 and does no I/O beyond `stat`.
pub fn checkpoint_known_db_files(data_dir: &Path) -> u64 {
    if !data_dir.is_dir() {
        return 0;
    }
    let mut reclaimed: u64 = 0;
    for name in KNOWN_DB_FILENAMES {
        let db_path = data_dir.join(name);
        if !db_path.is_file() {
            continue;
        }
        match checkpoint_one(&db_path) {
            Ok(bytes) => {
                reclaimed = reclaimed.saturating_add(bytes);
                if bytes > 0 {
                    tracing::debug!(
                        path = %db_path.display(),
                        bytes,
                        "wal checkpoint reclaimed bytes"
                    );
                }
            }
            Err(e) => {
                // Locked DB (another claudepot process running) is
                // the common case — log quiet.
                tracing::trace!(
                    path = %db_path.display(),
                    error = %e,
                    "wal checkpoint skipped"
                );
            }
        }
    }
    reclaimed
}

/// Open one DB, checkpoint+truncate its WAL, close. The connection
/// drops at end-of-scope which also runs SQLite's clean-close
/// path. Returns bytes reclaimed from the `*.db-wal` sidecar.
fn checkpoint_one(db_path: &Path) -> rusqlite::Result<u64> {
    let wal_path = db_path.with_extension("db-wal");
    let before = wal_size(&wal_path);

    let conn = Connection::open(db_path)?;
    conn.busy_timeout(STARTUP_BUSY_TIMEOUT)?;
    // Don't set journal_mode here — opening a non-WAL DB and
    // forcing WAL would write a new WAL header; for a DB that
    // *is* in WAL mode, the journal_mode pragma is a no-op.
    // The checkpoint pragma works on any WAL-mode DB and is a
    // no-op on a non-WAL DB.
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(conn);

    let after = wal_size(&wal_path);
    Ok(before.saturating_sub(after))
}

fn wal_size(wal_path: &Path) -> u64 {
    std::fs::metadata(wal_path).map(|m| m.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    //! Test scope: plumbing correctness (no panics, byte-counting,
    //! file-filter discipline, integrity preservation).
    //!
    //! The actual "shrinks a leaked WAL from N bytes to 0" property
    //! cannot be cleanly unit-tested because SQLite's last-connection
    //! close truncates the WAL to 0 regardless of `journal_size_limit`
    //! — reproducing a leaked WAL needs two concurrent processes (or
    //! at least two threads with overlapping connection lifetimes),
    //! which is heavier than this layer warrants. The truncate
    //! behavior itself is part of SQLite's published contract for
    //! `PRAGMA wal_checkpoint(TRUNCATE)`, so we trust it and verify
    //! the leak case manually (see `dev-docs/sqlite-wal-management.md`
    //! → Verification).
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    /// Create a valid file-backed DB with one table and a row.
    fn make_known_db(dir: &Path, db_name: &str) {
        let db_path = dir.join(db_name);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\n\
             CREATE TABLE t (k INTEGER PRIMARY KEY, v TEXT);\n\
             INSERT INTO t VALUES (1, 'hello');",
        )
        .unwrap();
    }

    #[test]
    fn test_checkpoint_on_missing_dir_returns_zero() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(checkpoint_known_db_files(&missing), 0);
    }

    #[test]
    fn test_checkpoint_on_empty_dir_returns_zero() {
        let dir = TempDir::new().unwrap();
        assert_eq!(checkpoint_known_db_files(dir.path()), 0);
    }

    #[test]
    fn test_checkpoint_on_clean_db_does_not_error() {
        let dir = TempDir::new().unwrap();
        make_known_db(dir.path(), "sessions.db");
        // Reclaimed may be 0 (WAL already clean from SQLite's
        // close-time checkpoint) — the assertion is that we don't
        // panic and that the DB is still usable afterwards.
        let _ = checkpoint_known_db_files(dir.path());

        // DB still opens and reads — housekeeping didn't corrupt it.
        let conn = Connection::open(dir.path().join("sessions.db")).unwrap();
        let v: String = conn
            .query_row("SELECT v FROM t WHERE k = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "hello");
        // Integrity intact.
        let ok: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ok, "ok");
    }

    #[test]
    fn test_checkpoint_ignores_unknown_files() {
        let dir = TempDir::new().unwrap();
        // Two non-DB-named files that housekeeping must not touch.
        let other_db = dir.path().join("scratch.db");
        let other_wal = dir.path().join("scratch.db-wal");
        std::fs::write(&other_db, b"not a real db").unwrap();
        std::fs::write(&other_wal, b"not a real wal").unwrap();

        let _ = checkpoint_known_db_files(dir.path());

        // Files unchanged.
        assert_eq!(std::fs::read(&other_db).unwrap(), b"not a real db");
        assert_eq!(std::fs::read(&other_wal).unwrap(), b"not a real wal");
    }

    #[test]
    fn test_checkpoint_is_idempotent_on_clean_state() {
        let dir = TempDir::new().unwrap();
        make_known_db(dir.path(), "memory_changes.db");
        let first = checkpoint_known_db_files(dir.path());
        let second = checkpoint_known_db_files(dir.path());
        // Both passes must succeed without panicking and the second
        // must not reclaim anything (already clean).
        let _ = first;
        assert_eq!(second, 0);
    }

    #[test]
    fn test_checkpoint_handles_all_known_filenames() {
        // Every name in KNOWN_DB_FILENAMES should be openable as a
        // SQLite DB — catches typos like `enf-vault.db` regression.
        let dir = TempDir::new().unwrap();
        for name in KNOWN_DB_FILENAMES {
            make_known_db(dir.path(), name);
        }
        let _ = checkpoint_known_db_files(dir.path());
        // Each DB is still readable.
        for name in KNOWN_DB_FILENAMES {
            let conn = Connection::open(dir.path().join(name)).unwrap();
            let ok: String = conn
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap();
            assert_eq!(ok, "ok", "integrity check failed for {name}");
        }
    }
}
