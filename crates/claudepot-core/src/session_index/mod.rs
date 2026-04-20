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

mod codec;
pub mod diff;
pub mod error;
pub mod schema;

pub use error::SessionIndexError;

use chrono::Utc;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::session::{scan_session, SessionRow};

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
    ///
    /// If the DB file exists but is corrupt (`SQLITE_NOTADB`,
    /// `SQLITE_CORRUPT`), the bad file is moved aside as
    /// `sessions.db.corrupt-<epoch_ms>` and a fresh one is created.
    /// The session index is a pure cache — wipe-and-rebuild is
    /// always a safe recovery here.
    pub fn open(path: &Path) -> Result<Self, SessionIndexError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // `Connection::open` is lazy — it doesn't validate the file
        // header until the first query. Wrap the full initialization
        // sequence so corruption detected mid-init (on PRAGMA or
        // schema apply) also triggers the quarantine path.
        let db = match Self::init_connection(path) {
            Ok(c) => c,
            Err(SessionIndexError::Sql(e)) if is_corrupt_error(&e) => {
                quarantine_corrupt_db(path)?;
                Self::init_connection(path)?
            }
            Err(e) => return Err(e),
        };

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

    /// Run the full open / pragma / schema / touch dance. Extracted
    /// so the outer `open()` can retry this whole sequence after
    /// quarantining a corrupt DB — corruption can first surface on
    /// PRAGMA or schema DDL, not just at `Connection::open`.
    fn init_connection(path: &Path) -> Result<Connection, SessionIndexError> {
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        apply_schema(&db)?;
        // Force WAL/SHM sidecars to materialize NOW so the chmod
        // loop in open() can narrow their perms. Without this, the
        // sidecars don't exist yet and later writes create them with
        // the process umask (typically 0644) — leaking prompt text
        // and token totals to other local users.
        db.execute_batch(
            "BEGIN IMMEDIATE; INSERT OR IGNORE INTO meta (k, v) VALUES ('_touch','1'); \
             DELETE FROM meta WHERE k='_touch'; COMMIT;",
        )?;
        Ok(db)
    }

    /// Internal accessor. Kept `pub(crate)` so sibling helpers (the
    /// diff-and-refresh logic, eventually FTS) can share the lock
    /// without re-wrapping.
    ///
    /// Recovers from poisoning by taking the inner guard — SQLite's
    /// on-disk state is transactionally consistent even if a Rust
    /// panic blew up a caller mid-operation, so there's nothing to
    /// roll back at this layer. Project rules ("no `expect` in core")
    /// make the previous `.expect(...)` a hard violation.
    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|p| p.into_inner())
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

    /// Converge the cache with `<config_dir>/projects/`. Walks the
    /// filesystem, diffs against the cache, re-scans only the files
    /// whose `(size, mtime_ns)` moved, and applies the result in a
    /// single SQLite transaction.
    ///
    /// Cold first run parses everything; subsequent runs cost one
    /// `stat` per file plus a tiny SQL read. Per-file errors are
    /// collected into `RefreshStats.failed` with their path and error
    /// string so callers can surface partial degradation instead of
    /// masquerading a broken transcript as a clean refresh.
    pub fn refresh(&self, config_dir: &Path) -> Result<RefreshStats, SessionIndexError> {
        let started_at = std::time::Instant::now();
        let walk = codec::walk_fs(config_dir)?;

        // Snapshot the DB side of the diff under a short-lived lock
        // so the rayon scan that follows runs without holding it.
        let db_tuples = {
            let db = self.db();
            codec::load_db_tuples(&db)?
        };

        let fs_tuples: Vec<diff::IndexTuple> =
            walk.entries.iter().map(|e| e.tuple.clone()).collect();
        let plan = diff::diff_fs_vs_db(&fs_tuples, &db_tuples);

        // Build a `file_path -> &FsEntry` lookup so the upsert loop
        // can recover the slug + absolute path for each scan.
        let by_path: std::collections::HashMap<&str, &codec::FsEntry> = walk
            .entries
            .iter()
            .map(|e| (e.tuple.file_path.as_str(), e))
            .collect();

        // Parallel re-scan for the delta. Per-file errors are
        // captured with their path + message so the caller can log /
        // surface them; the file will also be retried on the next
        // refresh because its tuple stays absent or different.
        let scan_results: Vec<Result<SessionRow, (std::path::PathBuf, String)>> = plan
            .to_upsert
            .par_iter()
            .filter_map(|path_key| {
                by_path.get(path_key.as_str()).map(|entry| {
                    scan_session(&entry.slug, &entry.path)
                        .map_err(|e| (entry.path.clone(), e.to_string()))
                })
            })
            .collect();

        let mut scanned: Vec<SessionRow> = Vec::with_capacity(scan_results.len());
        let mut failed: Vec<(std::path::PathBuf, String)> = walk.stat_failed;
        for r in scan_results {
            match r {
                Ok(row) => scanned.push(row),
                Err((path, msg)) => {
                    tracing::warn!(path = %path.display(), error = %msg, "session_index: scan failed");
                    failed.push((path, msg));
                }
            }
        }

        // Single write transaction: upserts + deletes. If anything
        // fails, nothing is committed and the cache stays consistent.
        let indexed_at_ms = Utc::now().timestamp_millis();
        let scanned_count = scanned.len();
        let deleted_count = plan.to_delete.len();
        {
            let mut db = self.db();
            let tx = db.transaction()?;
            for row in &scanned {
                codec::upsert_row(&tx, row, indexed_at_ms)?;
            }
            for gone in &plan.to_delete {
                codec::delete_row(&tx, gone)?;
            }
            tx.commit()?;
        }

        let elapsed = started_at.elapsed();
        tracing::info!(
            scanned = scanned_count,
            deleted = deleted_count,
            failed = failed.len(),
            total_on_disk = walk.entries.len(),
            elapsed_ms = elapsed.as_millis() as u64,
            "session_index: refresh complete"
        );

        Ok(RefreshStats {
            scanned: scanned_count,
            deleted: deleted_count,
            total_on_disk: walk.entries.len(),
            failed,
            elapsed,
        })
    }

    /// Refresh the cache against `config_dir` and return every row,
    /// newest-first. This is the replacement for
    /// `session::list_all_sessions` — same output contract, but the
    /// fold cost is paid only on the delta.
    pub fn list_all(&self, config_dir: &Path) -> Result<Vec<SessionRow>, SessionIndexError> {
        self.refresh(config_dir)?;
        let db = self.db();
        codec::load_all_rows(&db)
    }

    /// Truncate the cache. Intended as the escape hatch for cases the
    /// `(size, mtime)` guard can't see — filesystems with coarse
    /// mtime resolution, clock skew, a JSONL edited in-place with
    /// `truncate` + identical byte count. Caller should follow with
    /// `list_all` / `refresh` to repopulate.
    ///
    /// Does not drop the DB file or touch the schema — just the rows.
    pub fn rebuild(&self) -> Result<(), SessionIndexError> {
        let db = self.db();
        db.execute("DELETE FROM sessions", [])?;
        Ok(())
    }
}

/// Summary of a single `refresh` call. Exposed for diagnostics and
/// future progress-UI integration; not wired to the frontend yet.
#[derive(Debug, Clone, Default)]
pub struct RefreshStats {
    /// Number of transcripts that were (re-)parsed this pass.
    pub scanned: usize,
    /// Number of cache rows removed because the file vanished on disk.
    pub deleted: usize,
    /// Number of transcripts visible on disk after the walk.
    pub total_on_disk: usize,
    /// Per-file failures — both stat() errors during the walk and
    /// scan failures during the parallel fold. Each entry is
    /// `(path, error_string)`. A non-empty list means the cache is
    /// incomplete; callers can surface this via the UI or logs.
    pub failed: Vec<(std::path::PathBuf, String)>,
    pub elapsed: std::time::Duration,
}

fn apply_schema(db: &Connection) -> Result<(), SessionIndexError> {
    db.execute_batch(schema::SCHEMA)?;
    db.execute(
        "INSERT OR IGNORE INTO meta (k, v) VALUES ('schema_version', ?1)",
        params![schema::SCHEMA_VERSION],
    )?;
    Ok(())
}

/// Detect the two rusqlite error codes that mean "the file isn't a
/// usable SQLite database". Anything else (I/O, locking, etc.)
/// propagates — we don't want to quarantine a DB just because the
/// disk is full.
fn is_corrupt_error(err: &rusqlite::Error) -> bool {
    if let rusqlite::Error::SqliteFailure(info, _) = err {
        matches!(
            info.code,
            rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
        )
    } else {
        false
    }
}

/// Move a corrupt DB (plus any WAL/SHM sidecars) aside so `open()`
/// can create fresh files without risking data loss — the index is
/// a cache, so "recovered" just means "rebuild from disk on next
/// refresh". The quarantined file is preserved (not deleted) so the
/// user can hand it to support if they care.
fn quarantine_corrupt_db(path: &Path) -> Result<(), SessionIndexError> {
    let stamp = chrono::Utc::now().timestamp_millis();
    let corrupt_path = path.with_extension(format!("db.corrupt-{stamp}"));
    tracing::warn!(
        from = %path.display(),
        to = %corrupt_path.display(),
        "session_index: quarantining corrupt DB and rebuilding"
    );
    std::fs::rename(path, &corrupt_path)?;
    for sidecar_ext in ["db-wal", "db-shm"] {
        let side = path.with_extension(sidecar_ext);
        if side.exists() {
            let _ = std::fs::remove_file(side);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // A single well-formed user/assistant pair. Reused across several
    // tests so the scan produces a predictable SessionRow shape.
    fn sample_lines(cwd: &str, session_id: &str) -> Vec<String> {
        vec![
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":"hi"}},"timestamp":"2026-04-10T10:00:00Z","cwd":"{cwd}","sessionId":"{session_id}"}}"#
            ),
            format!(
                r#"{{"type":"assistant","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"hey"}}],"usage":{{"input_tokens":1,"output_tokens":1}}}},"timestamp":"2026-04-10T10:00:01Z","cwd":"{cwd}","sessionId":"{session_id}"}}"#
            ),
        ]
    }

    fn write_session(cfg: &std::path::Path, slug: &str, id: &str, lines: &[String]) -> PathBuf {
        let dir = cfg.join("projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.jsonl"));
        let mut f = std::fs::File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        path
    }

    fn open_index() -> (SessionIndex, TempDir) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        (idx, tmp)
    }

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

    #[cfg(unix)]
    #[test]
    fn open_sets_0600_on_wal_and_shm_sidecars() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");
        let _idx = SessionIndex::open(&path).unwrap();

        // WAL mode + the touch write in open() both force sidecar
        // creation. If either is missing, prompt text leaks to other
        // local users.
        let wal = path.with_extension("db-wal");
        let shm = path.with_extension("db-shm");
        assert!(wal.exists(), "WAL sidecar must exist after open");
        assert!(shm.exists(), "SHM sidecar must exist after open");
        for p in [wal, shm] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "sidecar {} must be 0600", p.display());
        }
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

    // -----------------------------------------------------------------
    // refresh() tests
    // -----------------------------------------------------------------

    #[test]
    fn refresh_empty_projects_dir_is_noop() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 0);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.total_on_disk, 0);
        assert_eq!(idx.row_count().unwrap(), 0);
    }

    #[test]
    fn refresh_cold_parses_all_files() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path_a = write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        let path_b = write_session(cfg.path(), "-b", "S2", &sample_lines("/b", "S2"));

        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 2);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.total_on_disk, 2);
        assert_eq!(idx.row_count().unwrap(), 2);

        // Verify one row round-tripped correctly.
        let db = idx.db();
        let row = codec::get_row_by_path(&db, &path_a.to_string_lossy())
            .unwrap()
            .expect("row a should be cached");
        assert_eq!(row.session_id, "S1");
        assert_eq!(row.project_path, "/a");
        assert_eq!(row.user_message_count, 1);
        assert_eq!(row.assistant_message_count, 1);
        drop(db);

        let db = idx.db();
        let row = codec::get_row_by_path(&db, &path_b.to_string_lossy())
            .unwrap()
            .expect("row b should be cached");
        assert_eq!(row.session_id, "S2");
    }

    #[test]
    fn refresh_warm_is_a_noop() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();

        // Snapshot the indexed_at_ms so we can prove the warm refresh
        // didn't actually re-upsert. scanned==0 alone only proves the
        // diff returned zero, not that the row was left untouched.
        let before_indexed_at = read_indexed_at_ms(&idx, &path.to_string_lossy());

        // Same-millisecond successive refreshes would pass anyway;
        // sleep a tick so "indexed_at_ms unchanged" is a real test.
        std::thread::sleep(std::time::Duration::from_millis(5));

        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 0, "warm refresh must not re-scan");
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.failed.len(), 0);
        assert_eq!(stats.total_on_disk, 1);
        assert_eq!(idx.row_count().unwrap(), 1);

        let after_indexed_at = read_indexed_at_ms(&idx, &path.to_string_lossy());
        assert_eq!(
            before_indexed_at, after_indexed_at,
            "warm refresh must NOT rewrite the row (indexed_at_ms drift proves an upsert)"
        );
    }

    fn read_indexed_at_ms(idx: &SessionIndex, file_path: &str) -> i64 {
        let db = idx.db();
        db.query_row(
            "SELECT indexed_at_ms FROM sessions WHERE file_path = ?1",
            [file_path],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    }

    #[test]
    fn refresh_rescans_when_mtime_changes() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();

        // Append a new user line. Different size AND mtime, so the
        // guard trips either way.
        let extra = format!(
            r#"{{"type":"user","message":{{"role":"user","content":"second turn"}},"timestamp":"2026-04-10T10:01:00Z","cwd":"/a","sessionId":"S1"}}"#
        );
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{extra}").unwrap();
        drop(f);

        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 1);
        assert_eq!(stats.deleted, 0);

        let db = idx.db();
        let row = codec::get_row_by_path(&db, &path.to_string_lossy())
            .unwrap()
            .unwrap();
        assert_eq!(row.user_message_count, 2, "second turn must be visible");
    }

    #[test]
    fn refresh_deletes_rows_for_missing_files() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();
        assert_eq!(idx.row_count().unwrap(), 1);

        std::fs::remove_file(&path).unwrap();
        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 0);
        assert_eq!(stats.deleted, 1);
        assert_eq!(idx.row_count().unwrap(), 0);
    }

    #[test]
    fn refresh_tolerates_malformed_jsonl() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        // One good file, one with junk lines. Neither should abort.
        write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        let bad = vec!["{garbage".to_string(), "also bad".to_string()];
        write_session(cfg.path(), "-b", "S2", &bad);

        let stats = idx.refresh(cfg.path()).unwrap();
        // Malformed lines still count as events; the scan succeeds.
        assert_eq!(stats.scanned, 2);
        assert_eq!(idx.row_count().unwrap(), 2);
    }

    // -----------------------------------------------------------------
    // list_all() tests
    // -----------------------------------------------------------------

    #[test]
    fn list_all_returns_newest_first() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        // S1 at 2026-04-01, S2 at 2026-04-20 — S2 should come first.
        let older = format!(
            r#"{{"type":"user","message":{{"role":"user","content":"old"}},"timestamp":"2026-04-01T00:00:00Z","cwd":"/a","sessionId":"S1"}}"#
        );
        let newer = format!(
            r#"{{"type":"user","message":{{"role":"user","content":"new"}},"timestamp":"2026-04-20T00:00:00Z","cwd":"/b","sessionId":"S2"}}"#
        );
        write_session(cfg.path(), "-a", "S1", &vec![older]);
        write_session(cfg.path(), "-b", "S2", &vec![newer]);

        let rows = idx.list_all(cfg.path()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].session_id, "S2");
        assert_eq!(rows[1].session_id, "S1");
    }

    #[test]
    fn list_all_round_trips_null_and_empty_optional_fields() {
        // A single malformed JSONL line: no valid events, no cwd, no
        // timestamps, no models, no git_branch, no cc_version, no
        // display_slug. Every Optional<_> field should round-trip as
        // None, not be silently defaulted to a bogus non-None.
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let junk = vec!["{not valid json".to_string()];
        let path = write_session(cfg.path(), "-junk", "J1", &junk);

        let rows = idx.list_all(cfg.path()).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.session_id, "J1");
        assert_eq!(r.event_count, 1, "malformed line still counts as an event");
        assert_eq!(r.user_message_count, 0);
        assert_eq!(r.assistant_message_count, 0);
        assert!(r.first_user_prompt.is_none());
        assert!(r.models.is_empty());
        assert!(r.first_ts.is_none());
        assert!(r.last_ts.is_none());
        assert!(r.git_branch.is_none());
        assert!(r.cc_version.is_none());
        assert!(r.display_slug.is_none());
        assert!(!r.has_error);
        // file_size + mtime still populate from fs::metadata.
        assert!(r.file_size_bytes > 0);
        assert!(r.last_modified.is_some());
        // project_path falls back to unsanitize(slug) when no cwd.
        assert!(r.project_from_transcript == false);
        // Path round-trips byte-exactly.
        assert_eq!(r.file_path, path);
    }

    #[test]
    fn list_all_round_trips_non_ascii_text() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        // Chinese + emoji + accented Latin. If any column gets
        // round-tripped through a latin-1 path or corrupts the bytes,
        // the assertion will fire.
        let line = r#"{"type":"user","message":{"role":"user","content":"修复 build 🐛 café"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/á","gitBranch":"feature/中文-branch","sessionId":"N1"}"#;
        write_session(cfg.path(), "-accented", "N1", &vec![line.to_string()]);

        let rows = idx.list_all(cfg.path()).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.first_user_prompt.as_deref(), Some("修复 build 🐛 café"));
        assert_eq!(r.project_path, "/á");
        assert_eq!(r.git_branch.as_deref(), Some("feature/中文-branch"));
    }

    #[test]
    fn refresh_redacts_sk_ant_tokens_in_first_user_prompt() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let prompt = r#"debug this token: sk-ant-oat01-AbC_xyZ-123 please"#;
        let line = format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{prompt}"}},"timestamp":"2026-04-10T10:00:00Z","cwd":"/x","sessionId":"S1"}}"#
        );
        write_session(cfg.path(), "-x", "S1", &vec![line]);

        let rows = idx.list_all(cfg.path()).unwrap();
        assert_eq!(rows.len(), 1);
        let stored = rows[0].first_user_prompt.as_deref().unwrap();
        assert!(
            !stored.contains("sk-ant-oat01-AbC_xyZ-123"),
            "raw token must NOT survive into the cache; got {stored:?}"
        );
        assert!(
            stored.contains("sk-ant-****"),
            "redacted form must appear; got {stored:?}"
        );
    }

    #[test]
    fn list_all_round_trips_all_row_fields() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let user = r#"{"type":"user","message":{"role":"user","content":"Fix the build"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/repo/foo","gitBranch":"main","version":"2.1.97","sessionId":"AAA","slug":"brave-otter"}"#;
        let asst = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"OK"}],"usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":200}},"timestamp":"2026-04-10T10:00:05Z","cwd":"/repo/foo","sessionId":"AAA"}"#;
        write_session(
            cfg.path(),
            "-repo-foo",
            "AAA",
            &vec![user.to_string(), asst.to_string()],
        );

        let rows = idx.list_all(cfg.path()).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.session_id, "AAA");
        assert_eq!(r.project_path, "/repo/foo");
        assert!(r.project_from_transcript);
        assert_eq!(r.first_user_prompt.as_deref(), Some("Fix the build"));
        assert_eq!(r.models, vec!["claude-opus-4-7".to_string()]);
        assert_eq!(r.tokens.input, 100);
        assert_eq!(r.tokens.output, 50);
        assert_eq!(r.tokens.cache_creation, 10);
        assert_eq!(r.tokens.cache_read, 200);
        assert_eq!(r.git_branch.as_deref(), Some("main"));
        assert_eq!(r.cc_version.as_deref(), Some("2.1.97"));
        assert_eq!(r.display_slug.as_deref(), Some("brave-otter"));
        assert!(r.last_modified.is_some(), "mtime round-trips");
    }

    // -----------------------------------------------------------------
    // rebuild() tests
    // -----------------------------------------------------------------

    #[test]
    fn open_quarantines_corrupt_db_and_creates_fresh() {
        use std::io::Write as _;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");
        // Plant a non-SQLite file at the expected path.
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"this is not a sqlite database").unwrap();
        drop(f);

        let idx = SessionIndex::open(&path).expect("open should recover");
        assert_eq!(idx.row_count().unwrap(), 0);
        assert_eq!(
            idx.schema_version().unwrap().as_deref(),
            Some(schema::SCHEMA_VERSION)
        );

        // A quarantined file must exist alongside for manual forensics.
        let siblings: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            siblings.iter().any(|n| n.starts_with("sessions.db.corrupt-")),
            "quarantined file must exist; saw {siblings:?}"
        );
    }

    #[test]
    fn rebuild_empty_is_noop() {
        let (idx, _tmp) = open_index();
        idx.rebuild().unwrap();
        assert_eq!(idx.row_count().unwrap(), 0);
    }

    #[test]
    fn rebuild_clears_rows_then_refresh_repopulates() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        write_session(cfg.path(), "-b", "S2", &sample_lines("/b", "S2"));
        idx.refresh(cfg.path()).unwrap();
        assert_eq!(idx.row_count().unwrap(), 2);

        idx.rebuild().unwrap();
        assert_eq!(idx.row_count().unwrap(), 0);

        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 2, "all files re-scan after rebuild");
        assert_eq!(idx.row_count().unwrap(), 2);
    }

    #[test]
    fn refresh_handles_second_file_after_initial_cache() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        write_session(cfg.path(), "-a", "S1", &sample_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();

        write_session(cfg.path(), "-b", "S2", &sample_lines("/b", "S2"));
        let stats = idx.refresh(cfg.path()).unwrap();
        assert_eq!(stats.scanned, 1, "only the new file re-scans");
        assert_eq!(stats.deleted, 0);
        assert_eq!(idx.row_count().unwrap(), 2);
    }
}
