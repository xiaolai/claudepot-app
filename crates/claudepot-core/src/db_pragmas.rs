//! Standard SQLite pragmas for Claudepot's SQLite-backed stores.
//!
//! Every store in `claudepot-core` that opens a long-lived
//! `Connection` should call [`apply_standard_pragmas`] immediately
//! after `Connection::open`. The helper centralizes the pragmas
//! that prevent the `*.db-wal` files from growing unbounded — see
//! the 2026-05-24 incident where `sessions.db-wal` reached 6.3 GB.
//!
//! Deliberately omitted from this helper:
//!
//! - `synchronous` — left at SQLite's default (FULL). Credential
//!   stores (`accounts.db`, `keys.db`, `env-vault.db`) need that
//!   durability; demoting to NORMAL is a per-store decision, not a
//!   global one.
//! - `foreign_keys` — opt-in per store. Currently only
//!   `session_index` and `shared_memory` use FK enforcement;
//!   forcing it on globally could activate dormant constraints
//!   in stores that later gain FK schemas without review.

use rusqlite::Connection;
use std::time::Duration;

/// Cap each WAL file at 64 MB after every successful checkpoint.
///
/// Not a hard runtime cap — a busy writer with a blocking reader
/// can push the WAL briefly past this between checkpoints — but a
/// floor that any closed/idle DB settles to. Combined with the
/// startup checkpoint in [`crate::db_housekeeping`], it bounds
/// the worst-case WAL footprint to a few times this value.
pub(crate) const WAL_SIZE_LIMIT_BYTES: i64 = 64 * 1024 * 1024;

/// Project-standard 5-second wait on writer contention before
/// returning `SQLITE_BUSY`. Already applied store-by-store; the
/// helper sets it centrally so new stores inherit it for free.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Apply Claudepot's standard SQLite pragmas to a fresh connection.
///
/// Idempotent: safe to call repeatedly on the same connection.
/// Designed to slot in right after `Connection::open` and before
/// any schema DDL — see store call sites for the full ordering
/// (open → pragmas → optional FK opt-in → schema → sidecar
/// materialization → chmod).
///
/// `busy_timeout` is set first so the subsequent `PRAGMA
/// journal_mode=WAL` (which needs a momentary exclusive lock to
/// switch modes) can wait on a concurrent claudepot process
/// instead of returning `SQLITE_BUSY` immediately.
pub fn apply_standard_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(&format!(
        "PRAGMA journal_mode=WAL;\n\
         PRAGMA wal_autocheckpoint=1000;\n\
         PRAGMA journal_size_limit={WAL_SIZE_LIMIT_BYTES};"
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    // WAL mode requires a file-backed DB. `Connection::open_in_memory()`
    // silently downgrades to `memory` journal_mode, so every assertion
    // here uses a tempdir-backed file.
    fn open_temp() -> (TempDir, Connection) {
        let dir = TempDir::new().expect("tempdir");
        let conn = Connection::open(dir.path().join("test.db")).expect("open");
        (dir, conn)
    }

    fn pragma_i64(conn: &Connection, name: &str) -> i64 {
        conn.pragma_query_value(None, name, |r| r.get(0))
            .unwrap_or_else(|e| panic!("pragma {name}: {e}"))
    }

    fn pragma_string(conn: &Connection, name: &str) -> String {
        conn.pragma_query_value(None, name, |r| r.get(0))
            .unwrap_or_else(|e| panic!("pragma {name}: {e}"))
    }

    #[test]
    fn test_apply_standard_pragmas_sets_journal_mode_wal() {
        let (_dir, conn) = open_temp();
        apply_standard_pragmas(&conn).unwrap();
        assert_eq!(pragma_string(&conn, "journal_mode").to_lowercase(), "wal");
    }

    #[test]
    fn test_apply_standard_pragmas_sets_journal_size_limit() {
        let (_dir, conn) = open_temp();
        apply_standard_pragmas(&conn).unwrap();
        assert_eq!(pragma_i64(&conn, "journal_size_limit"), WAL_SIZE_LIMIT_BYTES);
    }

    #[test]
    fn test_apply_standard_pragmas_sets_wal_autocheckpoint() {
        let (_dir, conn) = open_temp();
        apply_standard_pragmas(&conn).unwrap();
        assert_eq!(pragma_i64(&conn, "wal_autocheckpoint"), 1000);
    }

    #[test]
    fn test_apply_standard_pragmas_sets_busy_timeout() {
        let (_dir, conn) = open_temp();
        apply_standard_pragmas(&conn).unwrap();
        assert_eq!(pragma_i64(&conn, "busy_timeout"), 5000);
    }

    #[test]
    fn test_apply_standard_pragmas_is_idempotent() {
        let (_dir, conn) = open_temp();
        apply_standard_pragmas(&conn).unwrap();
        apply_standard_pragmas(&conn).unwrap();
        assert_eq!(pragma_string(&conn, "journal_mode").to_lowercase(), "wal");
        assert_eq!(pragma_i64(&conn, "journal_size_limit"), WAL_SIZE_LIMIT_BYTES);
    }

    #[test]
    fn test_apply_standard_pragmas_does_not_touch_synchronous() {
        // The helper deliberately leaves `synchronous` at SQLite's
        // default. Credential stores (accounts, keys, env-vault) need
        // FULL durability; demoting globally would be unsafe.
        let (_dir, conn) = open_temp();
        let before = pragma_i64(&conn, "synchronous");
        apply_standard_pragmas(&conn).unwrap();
        let after = pragma_i64(&conn, "synchronous");
        assert_eq!(before, after);
    }

    #[test]
    fn test_apply_standard_pragmas_does_not_touch_foreign_keys() {
        // FK enforcement is opt-in per store. The helper must not
        // force it on — see `session_index` for the explicit opt-in.
        let (_dir, conn) = open_temp();
        let before = pragma_i64(&conn, "foreign_keys");
        apply_standard_pragmas(&conn).unwrap();
        let after = pragma_i64(&conn, "foreign_keys");
        assert_eq!(before, after);
    }
}
