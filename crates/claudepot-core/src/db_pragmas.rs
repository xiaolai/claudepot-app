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

use rusqlite::{Connection, ErrorCode};
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

/// Retry schedule for the WAL-mode transition, in milliseconds.
///
/// Sums to ~3.2 s, comfortably inside [`BUSY_TIMEOUT`] — a caller that
/// already tolerates a 5-second wait on writer contention is not
/// surprised by this one, and bounding it below that keeps the open
/// path's worst case unchanged.
const WAL_SWITCH_BACKOFF_MS: &[u64] = &[2, 5, 10, 20, 40, 80, 160, 320, 640, 960, 960];

/// Apply Claudepot's standard SQLite pragmas to a fresh connection.
///
/// Idempotent: safe to call repeatedly on the same connection.
/// Designed to slot in right after `Connection::open` and before
/// any schema DDL — see store call sites for the full ordering
/// (open → pragmas → optional FK opt-in → schema → sidecar
/// materialization → chmod).
pub fn apply_standard_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    enter_wal_mode(conn)?;
    conn.execute_batch(&format!(
        "PRAGMA wal_autocheckpoint=1000;\n\
         PRAGMA journal_size_limit={WAL_SIZE_LIMIT_BYTES};"
    ))?;
    Ok(())
}

/// Is this the "someone else holds the file" error?
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(ffi, _)
            if matches!(ffi.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn journal_mode(conn: &Connection) -> rusqlite::Result<String> {
    conn.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
}

/// Put the connection into WAL mode, retrying while another process
/// holds the file.
///
/// # `busy_timeout` does not cover this, whatever it looks like
///
/// This helper used to be one `execute_batch` beginning `PRAGMA
/// journal_mode=WAL`, with a comment claiming `busy_timeout` was set
/// first precisely so that statement "can wait on a concurrent
/// claudepot process instead of returning `SQLITE_BUSY` immediately."
/// That is not what SQLite does. Switching **into** WAL needs a
/// momentary exclusive lock, and the busy handler is **not** invoked
/// for it — the statement fails immediately with `SQLITE_BUSY`
/// ("database is locked") no matter how long the timeout is.
///
/// Measured, not reasoned about: 8 threads opening one fresh
/// `boards.db` concurrently, over 40 rounds, produced 2 hard failures
/// out of 320 opens. That is the documented deployment for
/// `boards.db` — GUI, CLI and the MCP subprocess all open it directly
/// with no channel between them — and `sessions.db` and `corpus.db`
/// follow the same access pattern. The user-visible shape was a store
/// that intermittently refused to open with "database is locked".
///
/// Two properties make the retry cheap:
///
/// - **The transition happens once per file, ever.** Re-issuing the
///   pragma against a database already in WAL is a no-op that takes no
///   exclusive lock, so the early return below is the path taken on
///   every open after the first. Steady state costs one pragma read.
/// - **Losing the race is self-resolving.** Whoever won it left the
///   file in WAL, so the loser's next check succeeds rather than
///   needing the lock at all.
///
/// Exhausting the schedule returns the underlying busy error rather
/// than proceeding: a connection silently left in `delete` journal
/// mode would take the *store's* write lock for whole transactions
/// instead of WAL's per-writer one, which converts a rare open failure
/// into a permanent, invisible concurrency regression.
fn enter_wal_mode(conn: &Connection) -> rusqlite::Result<()> {
    let mut last_busy: Option<rusqlite::Error> = None;

    for (i, delay) in WAL_SWITCH_BACKOFF_MS.iter().enumerate() {
        // Already there — the common path, and the reason a retry loop
        // costs nothing in steady state.
        if journal_mode(conn)?.eq_ignore_ascii_case("wal") {
            return Ok(());
        }
        // `journal_mode` returns the resulting mode as a row, so this
        // needs the `_and_check` form; plain `pragma_update` rejects a
        // statement that returns results.
        match conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0)) {
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => return Ok(()),
            // Reported some other mode. Not an error per se — retry and
            // let the loop's own verification decide.
            Ok(_) => {}
            Err(e) if is_busy(&e) => last_busy = Some(e),
            Err(e) => return Err(e),
        }
        if i + 1 < WAL_SWITCH_BACKOFF_MS.len() {
            std::thread::sleep(Duration::from_millis(*delay));
        }
    }

    // One last look: the winner of the race may have landed between
    // our final attempt and here.
    if journal_mode(conn)?.eq_ignore_ascii_case("wal") {
        return Ok(());
    }
    Err(last_busy.unwrap_or_else(|| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("could not switch journal_mode to WAL".to_string()),
        )
    }))
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
        assert_eq!(
            pragma_i64(&conn, "journal_size_limit"),
            WAL_SIZE_LIMIT_BYTES
        );
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
        assert_eq!(
            pragma_i64(&conn, "journal_size_limit"),
            WAL_SIZE_LIMIT_BYTES
        );
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

    /// The regression this helper's retry loop exists for.
    ///
    /// `boards.db` is opened directly and concurrently by the GUI, the
    /// CLI and the MCP subprocess with no channel between them;
    /// `sessions.db` and `corpus.db` share that pattern. Before the
    /// retry, racing the *first* open of a file — the one that has to
    /// perform the `delete` → `wal` transition — failed outright with
    /// `SQLITE_BUSY`, because SQLite does not run the busy handler for
    /// that transition however large `busy_timeout` is.
    ///
    /// Measured at 2 failures per 320 opens before the fix, so the
    /// thread count and round count here are chosen to make a
    /// regression fail reliably rather than occasionally.
    #[test]
    fn concurrent_first_open_never_returns_busy() {
        for round in 0..40 {
            let dir = TempDir::new().expect("tempdir");
            let path = dir.path().join("concurrent.db");
            let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let path = path.clone();
                    let errors = std::sync::Arc::clone(&errors);
                    std::thread::spawn(move || {
                        let conn = Connection::open(&path).expect("open");
                        if let Err(e) = apply_standard_pragmas(&conn) {
                            errors.lock().unwrap().push(e.to_string());
                            return;
                        }
                        // And the connection really is in WAL — not
                        // silently left in `delete`, which would take
                        // whole-database write locks in production.
                        assert_eq!(pragma_string(&conn, "journal_mode").to_lowercase(), "wal");
                    })
                })
                .collect();
            for h in handles {
                h.join().expect("worker panicked");
            }

            let errors = errors.lock().unwrap();
            assert!(
                errors.is_empty(),
                "round {round}: concurrent open failed: {errors:?}"
            );
        }
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
