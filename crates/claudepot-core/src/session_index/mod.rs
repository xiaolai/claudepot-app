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
mod turns;

pub use turns::TurnCandidate;

pub use error::SessionIndexError;

use chrono::Utc;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::artifact_usage::{model::UsageEvent, store as usage_store};
use crate::session::{scan_session, SessionRow, TurnRecord};

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
            .query_row("SELECT v FROM meta WHERE k = 'schema_version'", [], |r| {
                r.get(0)
            })
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
        type ScanOk = (SessionRow, Vec<UsageEvent>, Vec<TurnRecord>);
        let scan_results: Vec<Result<ScanOk, (std::path::PathBuf, String)>> = plan
            .to_upsert
            .par_iter()
            .filter_map(|path_key| {
                by_path.get(path_key.as_str()).map(|entry| {
                    scan_session(&entry.slug, &entry.path)
                        .map(|s| (s.row, s.usage, s.turns))
                        .map_err(|e| (entry.path.clone(), e.to_string()))
                })
            })
            .collect();

        let mut scanned: Vec<ScanOk> = Vec::with_capacity(scan_results.len());
        let mut failed: Vec<(std::path::PathBuf, String)> = walk.stat_failed;
        for r in scan_results {
            match r {
                Ok(pair) => scanned.push(pair),
                Err((path, msg)) => {
                    tracing::warn!(path = %path.display(), error = %msg, "session_index: scan failed");
                    failed.push((path, msg));
                }
            }
        }

        // Single write transaction: upserts + deletes (sessions table)
        // plus usage_event delete-and-reinsert (per re-scanned file).
        // GC of stale raw events runs in the same transaction so a
        // single refresh leaves the cache fully consistent.
        let indexed_at_ms = Utc::now().timestamp_millis();
        let usage_cutoff_ms = indexed_at_ms - 30 * 86_400_000;
        let scanned_count = scanned.len();
        let deleted_count = plan.to_delete.len();
        let mut usage_events_written = 0usize;
        {
            let mut db = self.db();
            let tx = db.transaction()?;
            for (row, events, turns) in &scanned {
                codec::upsert_row(&tx, row, indexed_at_ms)?;
                let file_path = row.file_path.to_string_lossy();
                // Per-turn rows: replace-all in the same transaction so
                // the cache is internally consistent. A re-scan that
                // grew the transcript by 5 turns ends up with exactly
                // those 5 new rows; one that shrank it (slim) ends up
                // with the new shorter set.
                turns::replace_turns(&tx, &file_path, turns)?;
                // Order matters: subtract the existing per-day counts
                // BEFORE deleting the raw events that produced them,
                // otherwise the ensuing inserts double-bump the daily
                // rollup on every re-scan.
                usage_store::subtract_daily_for_file(&tx, &file_path)?;
                usage_store::delete_events_for_file(&tx, &file_path)?;
                for ev in events {
                    usage_store::insert_event(&tx, ev, &file_path, &row.project_path)?;
                    usage_events_written += 1;
                }
            }
            for gone in &plan.to_delete {
                codec::delete_row(&tx, gone)?;
                usage_store::subtract_daily_for_file(&tx, gone)?;
                usage_store::delete_events_for_file(&tx, gone)?;
            }
            // GC raw events older than 30 days. The daily rollup is
            // unaffected; counters survive eviction.
            usage_store::gc_events_older_than(&tx, usage_cutoff_ms)?;
            tx.commit()?;
        }

        let elapsed = started_at.elapsed();
        tracing::info!(
            scanned = scanned_count,
            deleted = deleted_count,
            failed = failed.len(),
            total_on_disk = walk.entries.len(),
            usage_events = usage_events_written,
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

    /// Read every persisted per-turn row for one transcript file,
    /// ordered by `turn_index`. This is the consumer surface for
    /// per-turn dashboards (top-N costliest prompts, per-turn
    /// pacing). Empty for transcripts that haven't been re-scanned
    /// since this table was added — consumers should treat absence
    /// the same as "no data yet" rather than "no turns ever ran."
    ///
    /// Does not refresh the cache; pair with `list_all` (or accept
    /// stale data) at call sites that want fresh numbers.
    pub fn turns_for(&self, file_path: &str) -> Result<Vec<TurnRecord>, SessionIndexError> {
        let db = self.db();
        turns::load_turns(&db, file_path)
    }

    /// Coarse top-K-by-token-sum across the install. Used by the
    /// `usage_local::top_costly_turns` consumer to seed a model-aware
    /// re-rank in Rust. Open-ended bounds on either end of `window`
    /// translate to "no constraint on that side." Returns up to
    /// `pool_limit` rows; consumer typically passes `final_n × 50`
    /// so the re-rank can correct for cross-model rate divergences
    /// without touching the whole table.
    pub fn turn_candidates(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        pool_limit: usize,
    ) -> Result<Vec<turns::TurnCandidate>, SessionIndexError> {
        let db = self.db();
        turns::fetch_turn_candidates(&db, from_ms, to_ms, pool_limit)
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

    // -----------------------------------------------------------------
    // artifact_usage public API — wraps the queries in
    // `claudepot_core::artifact_usage` so callers don't need raw access
    // to the underlying connection. Locking matches the rest of the
    // index: short-lived guard, no I/O held across await.
    // -----------------------------------------------------------------

    pub fn usage_for_artifact(
        &self,
        kind: crate::artifact_usage::ArtifactKind,
        artifact_key: &str,
        now_ms: i64,
    ) -> Result<crate::artifact_usage::UsageStats, SessionIndexError> {
        let db = self.db();
        crate::artifact_usage::usage_for_artifact(&db, kind, artifact_key, now_ms)
            .map_err(SessionIndexError::Sql)
    }

    /// Batch fetch — single mutex acquisition for all keys, used by
    /// the Config-tree renderer to populate every visible artifact's
    /// badge in one round-trip without N independent IPC calls.
    pub fn usage_batch(
        &self,
        keys: &[(crate::artifact_usage::ArtifactKind, String)],
        now_ms: i64,
    ) -> Result<
        Vec<(
            (crate::artifact_usage::ArtifactKind, String),
            crate::artifact_usage::UsageStats,
        )>,
        SessionIndexError,
    > {
        let db = self.db();
        crate::artifact_usage::batch_usage(&db, keys, now_ms).map_err(SessionIndexError::Sql)
    }

    pub fn usage_top(
        &self,
        kind: Option<crate::artifact_usage::ArtifactKind>,
        limit: usize,
        now_ms: i64,
    ) -> Result<Vec<crate::artifact_usage::UsageListRow>, SessionIndexError> {
        let db = self.db();
        crate::artifact_usage::list_top_used(&db, kind, limit, now_ms)
            .map_err(SessionIndexError::Sql)
    }

    pub fn usage_known_keys(
        &self,
    ) -> Result<Vec<(crate::artifact_usage::ArtifactKind, String)>, SessionIndexError> {
        let db = self.db();
        crate::artifact_usage::list_all_known(&db).map_err(SessionIndexError::Sql)
    }

    /// Truncate the cache. Intended as the escape hatch for cases the
    /// `(size, mtime)` guard can't see — filesystems with coarse
    /// mtime resolution, clock skew, a JSONL edited in-place with
    /// `truncate` + identical byte count. Caller should follow with
    /// `list_all` / `refresh` to repopulate.
    ///
    /// Does not drop the DB file or touch the schema — just the rows.
    /// Also truncates `usage_event` and `usage_daily` so the next
    /// `refresh()` rebuilds the rollups from disk truth.
    pub fn rebuild(&self) -> Result<(), SessionIndexError> {
        let db = self.db();
        db.execute("DELETE FROM sessions", [])?;
        usage_store::truncate_all(&db)?;
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
    // v2 (additive): artifact usage tables share `sessions.db` because
    // the source data — JSONL transcripts — is the same. Refresh writes
    // both in one transaction.
    //
    // Read the prior version BEFORE the bump so we can tell a fresh DB
    // from a v1→v2 upgrade. On upgrade we invalidate the sessions
    // cache so the next `refresh()` re-scans every transcript and
    // populates usage tables — without this, existing users keep their
    // stale `(size, mtime_ns)` rows and never produce usage events
    // until each JSONL changes naturally.
    let prior_version: Option<String> = db
        .query_row("SELECT v FROM meta WHERE k = 'schema_version'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok();

    db.execute_batch(crate::artifact_usage::schema::SCHEMA)?;
    db.execute(
        "INSERT OR IGNORE INTO meta (k, v) VALUES ('schema_version', ?1)",
        params![crate::artifact_usage::schema::SCHEMA_VERSION],
    )?;
    // Forward migrate any v1 row to v2 (the meta key is INSERT OR
    // IGNORE so existing rows aren't bumped automatically).
    db.execute(
        "UPDATE meta SET v = ?1 WHERE k = 'schema_version' AND v < ?1",
        params![crate::artifact_usage::schema::SCHEMA_VERSION],
    )?;

    let current_version = crate::artifact_usage::schema::SCHEMA_VERSION;
    let needs_upgrade_rescan = matches!(
        prior_version.as_deref(),
        Some(v) if v != current_version
    );
    if needs_upgrade_rescan {
        // Drop cached session rows so the next refresh re-scans every
        // transcript and repopulates every co-located table that
        // depends on per-line JSONL extraction (artifact_usage,
        // session_turns). Cheaper than walking every existing row to
        // emit events — a cold scan of ~6 k JSONL files takes ~10 s,
        // well under any user-perceptible threshold for an upgrade
        // event. The session_turns rows are dropped in the same pass
        // because `delete_row` cascades them; without that, stale
        // turn rows for files that vanished from disk would linger.
        db.execute("DELETE FROM sessions", [])?;
        db.execute("DELETE FROM session_turns", [])?;
    }
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
            Some(crate::artifact_usage::schema::SCHEMA_VERSION)
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
            Some(crate::artifact_usage::schema::SCHEMA_VERSION)
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
        // session_turns is the per-turn token-detail table — must
        // exist alongside the rest of the schema for the cost
        // surface's drill-down queries to work.
        assert!(
            names.contains(&"session_turns".to_string()),
            "session_turns table must be created at schema time, not lazily on first insert"
        );
    }

    #[test]
    fn refresh_populates_per_turn_records() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        // Two assistant turns in one transcript with distinct token
        // counts so we can verify per-turn data round-trips.
        let lines = vec![
            r#"{"type":"user","message":{"role":"user","content":"first ask"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"a1"}],"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":2000}},"timestamp":"2026-04-10T10:00:01Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"user","message":{"role":"user","content":"second ask"},"timestamp":"2026-04-10T10:01:00Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text","text":"a2"}],"usage":{"input_tokens":300,"output_tokens":80,"cache_creation_input_tokens":150}},"timestamp":"2026-04-10T10:01:02Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
        ];
        let path = write_session(cfg.path(), "-p", "S1", &lines);
        idx.refresh(cfg.path()).unwrap();

        let turns = idx.turns_for(&path.to_string_lossy()).unwrap();
        assert_eq!(turns.len(), 2);
        // First assistant turn — Opus, prompt = "first ask".
        assert_eq!(turns[0].turn_index, 0);
        assert_eq!(turns[0].model, "claude-opus-4-7");
        assert_eq!(turns[0].tokens.input, 100);
        assert_eq!(turns[0].tokens.output, 50);
        assert_eq!(turns[0].tokens.cache_read, 2000);
        assert_eq!(turns[0].user_prompt_preview.as_deref(), Some("first ask"));
        // Second turn — Sonnet, prompt switched to "second ask".
        assert_eq!(turns[1].turn_index, 1);
        assert_eq!(turns[1].model, "claude-sonnet-4-6");
        assert_eq!(turns[1].tokens.input, 300);
        assert_eq!(turns[1].tokens.output, 80);
        assert_eq!(turns[1].tokens.cache_creation, 150);
        assert_eq!(turns[1].user_prompt_preview.as_deref(), Some("second ask"));
        // Aggregate row must equal the sum of per-turn tokens — the
        // two paths read from the same JSONL line and must agree.
        let db = idx.db();
        let row = codec::get_row_by_path(&db, &path.to_string_lossy())
            .unwrap()
            .unwrap();
        assert_eq!(row.tokens.input, 400);
        assert_eq!(row.tokens.output, 130);
        assert_eq!(row.tokens.cache_creation, 150);
        assert_eq!(row.tokens.cache_read, 2000);
    }

    #[test]
    fn refresh_replaces_turns_on_rescan() {
        // Re-scanning a file (mtime changes) must rebuild the per-turn
        // rowset for that file from scratch — leftover rows from a
        // longer prior version of the transcript would corrupt
        // downstream "top N" queries.
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-p", "S1", &sample_lines("/p", "S1"));
        idx.refresh(cfg.path()).unwrap();
        assert_eq!(idx.turns_for(&path.to_string_lossy()).unwrap().len(), 1);

        // Rewrite with two assistant turns this time, then bump mtime
        // forward so the (size, mtime, inode) guard fires a re-scan.
        let new_lines = vec![
            r#"{"type":"user","message":{"role":"user","content":"q1"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"a1"}],"usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-04-10T10:00:01Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"a2"}],"usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-04-10T10:00:02Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
        ];
        // Bump mtime by one second to force a re-scan even on
        // filesystems that round mtime to seconds.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut f = std::fs::File::create(&path).unwrap();
        for l in &new_lines {
            writeln!(f, "{l}").unwrap();
        }
        let new_mtime = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        let _ = filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(new_mtime));

        idx.refresh(cfg.path()).unwrap();
        let turns = idx.turns_for(&path.to_string_lossy()).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].turn_index, 0);
        assert_eq!(turns[1].turn_index, 1);
    }

    #[test]
    fn refresh_redacts_sk_ant_tokens_in_turn_prompts() {
        // The per-turn `user_prompt_preview` must be redacted at write
        // time — same rule as `first_user_prompt` on the session row.
        // A user pasting a token mid-conversation could end up only in
        // the per-turn store; the session-level redaction wouldn't
        // catch it.
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let lines = vec![
            r#"{"type":"user","message":{"role":"user","content":"safe first ask"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"a"}],"usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-04-10T10:00:01Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"user","message":{"role":"user","content":"now my key sk-ant-oat01-AbC123_xyz"},"timestamp":"2026-04-10T10:00:02Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"b"}],"usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-04-10T10:00:03Z","cwd":"/p","sessionId":"S1"}"#.to_string(),
        ];
        let path = write_session(cfg.path(), "-p", "S1", &lines);
        idx.refresh(cfg.path()).unwrap();

        let turns = idx.turns_for(&path.to_string_lossy()).unwrap();
        assert_eq!(turns.len(), 2);
        let preview = turns[1].user_prompt_preview.as_deref().unwrap();
        assert!(
            !preview.contains("sk-ant-oat01-AbC123_xyz"),
            "raw token must not survive into session_turns: {preview}"
        );
        assert!(preview.contains("sk-ant-****"));
    }

    #[test]
    fn delete_row_cascades_to_turns() {
        // When a transcript vanishes from disk, the cache must drop
        // both its session row AND its per-turn rows. Otherwise turn
        // rows accumulate orphaned forever.
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-p", "S1", &sample_lines("/p", "S1"));
        idx.refresh(cfg.path()).unwrap();
        assert_eq!(idx.turns_for(&path.to_string_lossy()).unwrap().len(), 1);

        std::fs::remove_file(&path).unwrap();
        idx.refresh(cfg.path()).unwrap();
        assert_eq!(idx.turns_for(&path.to_string_lossy()).unwrap().len(), 0);
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
        write_session(cfg.path(), "-a", "S1", &[older]);
        write_session(cfg.path(), "-b", "S2", &[newer]);

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
        assert!(!r.project_from_transcript);
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
        write_session(cfg.path(), "-accented", "N1", &[line.to_string()]);

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
        write_session(cfg.path(), "-x", "S1", &[line]);

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
            &[user.to_string(), asst.to_string()],
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
            Some(crate::artifact_usage::schema::SCHEMA_VERSION)
        );

        // A quarantined file must exist alongside for manual forensics.
        let siblings: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            siblings
                .iter()
                .any(|n| n.starts_with("sessions.db.corrupt-")),
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

    // -----------------------------------------------------------------
    // artifact_usage integration: a JSONL with one of each event family
    // refreshes into the right counters.
    // -----------------------------------------------------------------

    fn artifact_session_lines(cwd: &str, sid: &str) -> Vec<String> {
        // Five lines: hello user, slash command, invoked_skills attachment,
        // hook_success attachment, an Agent tool_use.
        vec![
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":"hi"}},"timestamp":"2026-04-10T10:00:00Z","cwd":"{cwd}","sessionId":"{sid}"}}"#
            ),
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":"<command-name>/foo:bar</command-name>"}},"timestamp":"2026-04-10T10:00:01Z","cwd":"{cwd}","sessionId":"{sid}"}}"#
            ),
            format!(
                r#"{{"type":"attachment","timestamp":"2026-04-10T10:00:02Z","sessionId":"{sid}","attachment":{{"type":"invoked_skills","skills":[{{"name":"x","path":"plugin:foo:x"}}]}}}}"#
            ),
            format!(
                r#"{{"type":"attachment","timestamp":"2026-04-10T10:00:03Z","sessionId":"{sid}","attachment":{{"type":"hook_success","hookName":"PreToolUse:Bash","command":"node /h.js","durationMs":42,"exitCode":0}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","timestamp":"2026-04-10T10:00:04Z","sessionId":"{sid}","message":{{"content":[{{"type":"tool_use","id":"toolu_X","name":"Agent","input":{{"subagent_type":"Explore"}}}}]}}}}"#
            ),
        ]
    }

    fn count_usage_rows(idx: &SessionIndex) -> i64 {
        let db = idx.db();
        db.query_row("SELECT COUNT(*) FROM usage_event", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    }

    #[test]
    fn refresh_writes_one_usage_event_per_extracted_kind() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        write_session(cfg.path(), "-a", "S1", &artifact_session_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();
        // One slash command + one skill + one hook + one agent = 4 rows.
        assert_eq!(count_usage_rows(&idx), 4);
    }

    #[test]
    fn rebuild_truncates_usage_tables() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        write_session(cfg.path(), "-a", "S1", &artifact_session_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();
        assert!(count_usage_rows(&idx) > 0);
        idx.rebuild().unwrap();
        assert_eq!(count_usage_rows(&idx), 0);
        let db = idx.db();
        let daily: i64 = db
            .query_row("SELECT COUNT(*) FROM usage_daily", [], |r| r.get(0))
            .unwrap();
        assert_eq!(daily, 0, "rebuild must clear daily rollup too");
    }

    #[test]
    fn refresh_re_scan_replaces_usage_for_same_file() {
        // A second refresh after a file change must not duplicate rows
        // for that file. The subtract→delete→insert sequence in
        // refresh is what guarantees this.
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-a", "S1", &artifact_session_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();
        let first = count_usage_rows(&idx);

        // Append one more event line to trigger re-scan.
        let extra = format!(
            r#"{{"type":"attachment","timestamp":"2026-04-10T10:00:05Z","sessionId":"S1","attachment":{{"type":"hook_success","hookName":"PreToolUse:Bash","command":"node /h.js","durationMs":51,"exitCode":0}}}}"#
        );
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{extra}").unwrap();
        drop(f);

        idx.refresh(cfg.path()).unwrap();
        let second = count_usage_rows(&idx);
        assert_eq!(
            second,
            first + 1,
            "re-scan must replace the file's events, not duplicate them"
        );
    }

    #[test]
    fn refresh_re_scan_does_not_inflate_daily_counts() {
        // Stronger regression than the row-count test: walks the full
        // index API to confirm `usage_for_artifact` returns the same
        // 30d count after re-scanning a file with no event changes.
        // A bug in subtract_daily_for_file would surface as 2× counts
        // here while leaving the raw row count unchanged.
        use crate::artifact_usage::ArtifactKind;
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-a", "S1", &artifact_session_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let first = idx
            .usage_for_artifact(ArtifactKind::Hook, "node /h.js", now_ms)
            .unwrap()
            .count_30d;

        // Force a re-scan by bumping mtime — same content, no new events.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(later)).unwrap();

        idx.refresh(cfg.path()).unwrap();
        let second = idx
            .usage_for_artifact(ArtifactKind::Hook, "node /h.js", now_ms)
            .unwrap()
            .count_30d;
        assert_eq!(
            second, first,
            "re-scan with unchanged content must leave the 30d count unchanged"
        );
    }

    #[test]
    fn schema_v1_to_v2_upgrade_invalidates_sessions_so_usage_can_repopulate() {
        // Simulate the field condition: a sessions.db that pre-dates the
        // artifact_usage tables — i.e. v1 schema with rows in `sessions`
        // and no `usage_event` / `usage_daily` tables. After re-opening
        // (which runs apply_schema and detects the v1→v2 upgrade), the
        // sessions table should be empty so the next refresh re-scans
        // every file from cold and produces usage events.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");

        // Hand-craft a v1-shaped DB: just the v1 sessions table + a
        // schema_version=1 meta row. Skip the usage tables on purpose.
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(schema::SCHEMA).unwrap();
            db.execute(
                "INSERT OR REPLACE INTO meta (k, v) VALUES ('schema_version', '1')",
                [],
            )
            .unwrap();
            // Plant one row so we can prove the upgrade clears it.
            db.execute(
                "INSERT INTO sessions (
                    file_path, slug, session_id, file_size_bytes,
                    file_mtime_ns, file_inode, project_path,
                    project_from_transcript, event_count, message_count,
                    user_message_count, assistant_message_count,
                    models_json, tokens_input, tokens_output,
                    tokens_cache_creation, tokens_cache_read, has_error,
                    is_sidechain, indexed_at_ms
                 ) VALUES (
                    '/legacy.jsonl', '-legacy', 'OLD', 1, 1, 1, '/x',
                    0, 0, 0, 0, 0, '[]', 0, 0, 0, 0, 0, 0, 0
                 )",
                [],
            )
            .unwrap();
        }

        // Open via SessionIndex — apply_schema runs, sees v1, invalidates.
        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(
            idx.row_count().unwrap(),
            0,
            "v1→v2 upgrade must clear sessions so refresh re-populates usage"
        );
        assert_eq!(
            idx.schema_version().unwrap().as_deref(),
            Some(crate::artifact_usage::schema::SCHEMA_VERSION),
            "schema_version must advance to v2 after upgrade"
        );
    }

    #[test]
    fn schema_v2_to_v3_upgrade_clears_sessions_and_session_turns() {
        // Field condition: a sessions.db from before the session_turns
        // table existed (i.e. schema_version='2' but no per-turn rows).
        // After re-opening, both `sessions` and `session_turns` must be
        // empty so the next refresh re-scans every file from cold and
        // populates per-turn data for historical transcripts. Without
        // the bump, top_costly_turns would silently miss every session
        // older than this release.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");

        // Hand-craft a v2 DB with a session row + a stale turn row.
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(schema::SCHEMA).unwrap();
            db.execute_batch(crate::artifact_usage::schema::SCHEMA)
                .unwrap();
            db.execute(
                "INSERT OR REPLACE INTO meta (k, v) VALUES ('schema_version', '2')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO sessions (
                    file_path, slug, session_id, file_size_bytes,
                    file_mtime_ns, file_inode, project_path,
                    project_from_transcript, event_count, message_count,
                    user_message_count, assistant_message_count,
                    models_json, tokens_input, tokens_output,
                    tokens_cache_creation, tokens_cache_read, has_error,
                    is_sidechain, indexed_at_ms
                 ) VALUES (
                    '/legacy.jsonl', '-legacy', 'OLD', 1, 1, 1, '/x',
                    0, 0, 0, 0, 0, '[]', 0, 0, 0, 0, 0, 0, 0
                 )",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO session_turns (
                    file_path, turn_index, ts_ms, model,
                    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
                    user_prompt_preview
                 ) VALUES ('/legacy.jsonl', 0, 0, 'old-model', 0, 0, 0, 0, NULL)",
                [],
            )
            .unwrap();
        }

        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(
            idx.row_count().unwrap(),
            0,
            "v2→v3 upgrade must clear sessions so refresh repopulates"
        );
        let turn_count: i64 = idx
            .db()
            .query_row("SELECT COUNT(*) FROM session_turns", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            turn_count, 0,
            "v2→v3 upgrade must clear session_turns so per-turn data repopulates from disk"
        );
        assert_eq!(
            idx.schema_version().unwrap().as_deref(),
            Some("3"),
            "schema_version must advance to v3 after upgrade"
        );
    }

    #[test]
    fn fresh_db_open_does_not_clear_anything() {
        // Regression guard for the v1→v2 logic: a brand-new DB has no
        // prior schema_version row, so the upgrade branch must NOT fire.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sessions.db");
        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(idx.row_count().unwrap(), 0);
        // Re-open: still no spurious cleanup.
        drop(idx);
        let idx2 = SessionIndex::open(&path).unwrap();
        assert_eq!(idx2.row_count().unwrap(), 0);
    }

    #[test]
    fn refresh_flips_agent_outcome_to_error_on_failed_tool_result() {
        // End-to-end coverage for the streaming outcome flip the
        // session scanner does (replacing the removed
        // `link_agent_outcomes` two-pass linker). We write one
        // assistant turn that dispatches two Agent calls, then a
        // user line whose tool_result for the second one is_error,
        // and verify that exactly one of the two recorded usage
        // events ends up with outcome=error.
        use crate::artifact_usage::ArtifactKind;
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        // JSONL is strictly one event per line — keep these as
        // single physical lines so the streaming parser sees them.
        let lines = vec![
            r#"{"type":"assistant","timestamp":"2026-04-10T10:00:00Z","sessionId":"S1","message":{"content":[{"type":"tool_use","id":"toolu_OK","name":"Agent","input":{"subagent_type":"Explore"}},{"type":"tool_use","id":"toolu_BAD","name":"Agent","input":{"subagent_type":"Explore"}}]}}"#.to_string(),
            r#"{"type":"user","timestamp":"2026-04-10T10:00:01Z","sessionId":"S1","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_BAD","is_error":true,"content":"boom"}]}}"#.to_string(),
            r#"{"type":"user","timestamp":"2026-04-10T10:00:02Z","sessionId":"S1","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_OK","is_error":false,"content":"ok"}]}}"#.to_string(),
        ];
        write_session(cfg.path(), "-a", "S1", &lines);
        idx.refresh(cfg.path()).unwrap();

        let now_ms = chrono::Utc::now().timestamp_millis();
        let stats = idx
            .usage_for_artifact(ArtifactKind::Agent, "Explore", now_ms)
            .unwrap();
        assert_eq!(
            stats.count_30d, 2,
            "two Agent dispatches should be recorded"
        );
        assert_eq!(
            stats.error_count_30d, 1,
            "exactly one event should be flipped to error by the failed tool_result"
        );
    }

    #[test]
    fn refresh_drops_usage_for_deleted_session_file() {
        let (idx, _tmp) = open_index();
        let cfg = TempDir::new().unwrap();
        let path = write_session(cfg.path(), "-a", "S1", &artifact_session_lines("/a", "S1"));
        idx.refresh(cfg.path()).unwrap();
        assert!(count_usage_rows(&idx) > 0);

        std::fs::remove_file(&path).unwrap();
        idx.refresh(cfg.path()).unwrap();
        assert_eq!(
            count_usage_rows(&idx),
            0,
            "usage rows follow session deletion"
        );
    }
}
