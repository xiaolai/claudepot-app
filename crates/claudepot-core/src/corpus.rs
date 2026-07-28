//! The analysis corpus — every transcript this user has, from every
//! machine, in one queryable place.
//!
//! # Why this is its own database file
//!
//! `sessions.db` is a **cache of one machine's live `~/.claude`**.
//! `SessionIndex::refresh` diffs every row in it against whatever is
//! found under one `config_dir` and deletes the remainder
//! (`diff_fs_vs_db` → `codec::delete_row`, which cascades turns and,
//! via the v4 FK, exchanges / tool_calls / FTS). That is correct for a
//! cache and fatal for an archive: point `refresh` at an imported
//! corpus and it deletes the live rows; run it again on the live
//! directory and it deletes the imported ones.
//!
//! So the corpus lives in `~/.claudepot/corpus.db`, outside that loop.
//! The consequences are all good ones:
//!
//! - **No migration.** Nothing lands on a 574 MB production database.
//! - **`host_id` is a column, not a schema change.** The thing that
//!   blocked the original multi-host phase costs one `TEXT NOT NULL`
//!   here.
//! - **Rebuildable by definition.** It is derived from files on disk,
//!   so none of the durable-archive semantics (quota, export, real
//!   deletion) are required to make it safe to throw away.
//!
//! Outputs do *not* live here. Distilled claims go to `memories` in
//! `sessions.db`, which carries no foreign key to `sessions` and is
//! never cascaded — verified, and the reason this split works at all.
//!
//! # Deduplication
//!
//! The same session is frequently present on several machines: four
//! hosts here hold 14,059 transcript files between them, with real
//! overlap. Counting those as distinct sessions would inflate every
//! aggregate the detectors compute.
//!
//! Sessions are therefore keyed by **`session_id`** (CC's per-session
//! UUID), with `corpus_files` recording every physical copy. When two
//! hosts disagree — one synced mid-session, so the same UUID has more
//! events on one machine — the **more complete** copy wins
//! (`event_count`), because a truncated copy is a strictly worse
//! answer to every question the corpus is asked.
//!
//! This replaces the plan's "content hash": a hash would answer
//! "are these bytes identical", but the question that matters is "are
//! these the same conversation, and which copy is best". A UUID plus a
//! completeness comparison answers that directly, and does not require
//! a second full read of 7.9 GB to decide whether to read it.

pub mod detect;
pub mod normalize;

use crate::session::core::{scan_session, SessionRow};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Host id used for the machine Claudepot is running on. Archived
/// corpora carry their source host's name instead.
pub const LOCAL_HOST: &str = "local";

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS corpus_meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);

-- One row per logical conversation, deduped across hosts by CC's
-- session UUID. Holds the most complete copy seen.
CREATE TABLE IF NOT EXISTS corpus_sessions (
    session_id       TEXT PRIMARY KEY,
    project_path     TEXT    NOT NULL,
    slug             TEXT    NOT NULL,
    first_ts_ms      INTEGER,
    last_ts_ms       INTEGER,
    event_count      INTEGER NOT NULL,
    message_count    INTEGER NOT NULL,
    user_message_count      INTEGER NOT NULL,
    assistant_message_count INTEGER NOT NULL,
    first_user_prompt TEXT,
    models_json      TEXT    NOT NULL,
    git_branch       TEXT,
    cc_version       TEXT,
    has_error        INTEGER NOT NULL,
    is_sidechain     INTEGER NOT NULL,
    -- Which physical copy the row above was folded from.
    best_host_id     TEXT    NOT NULL,
    best_file_path   TEXT    NOT NULL,
    indexed_at_ms    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_corpus_sessions_project ON corpus_sessions(project_path);
CREATE INDEX IF NOT EXISTS idx_corpus_sessions_last_ts ON corpus_sessions(last_ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_corpus_sessions_error   ON corpus_sessions(has_error);

-- Every physical copy, on every host. `(size_bytes, mtime_ns)` is the
-- re-scan guard, matching the idiom `sessions` already uses.
CREATE TABLE IF NOT EXISTS corpus_files (
    host_id       TEXT    NOT NULL,
    file_path     TEXT    NOT NULL,
    session_id    TEXT    NOT NULL,
    size_bytes    INTEGER NOT NULL,
    mtime_ns      INTEGER NOT NULL,
    event_count   INTEGER NOT NULL,
    indexed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (host_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_corpus_files_session ON corpus_files(session_id);
CREATE INDEX IF NOT EXISTS idx_corpus_files_host    ON corpus_files(host_id);

-- Turn-level content. Keyed by `file_path`, not `session_id`: CC leaves
-- the same session uuid in two project dirs after a move/adopt, so the
-- uuid is unique only within a project. `corpus_sessions` dedups at the
-- logical level; this table stays faithful to the physical file the
-- turns were read from.
CREATE TABLE IF NOT EXISTS corpus_exchanges (
    id             TEXT    PRIMARY KEY,        -- <file_path>\x1f<turn_index>
    file_path      TEXT    NOT NULL,
    session_id     TEXT    NOT NULL,
    turn_index     INTEGER NOT NULL,
    timestamp_ms   INTEGER,
    user_text      TEXT    NOT NULL,
    assistant_text TEXT    NOT NULL,
    UNIQUE (file_path, turn_index)
);

CREATE INDEX IF NOT EXISTS idx_corpus_exch_file    ON corpus_exchanges(file_path);
CREATE INDEX IF NOT EXISTS idx_corpus_exch_session ON corpus_exchanges(session_id);
CREATE INDEX IF NOT EXISTS idx_corpus_exch_ts      ON corpus_exchanges(timestamp_ms);

-- The Tier-3 substrate: `is_error` + `tool_result_text` are what a
-- failure→recovery detector reads. Cascades with its exchange so a
-- re-indexed file replaces its turns cleanly.
CREATE TABLE IF NOT EXISTS corpus_tool_calls (
    id               TEXT    PRIMARY KEY,      -- <exchange_id>\x1f<ordinal>
    exchange_id      TEXT    NOT NULL REFERENCES corpus_exchanges(id) ON DELETE CASCADE,
    file_path        TEXT    NOT NULL,
    ordinal          INTEGER NOT NULL,
    tool_name        TEXT    NOT NULL,
    tool_input_json  TEXT,
    tool_result_text TEXT,
    is_error         INTEGER NOT NULL DEFAULT 0,
    timestamp_ms     INTEGER
);

CREATE INDEX IF NOT EXISTS idx_corpus_tc_exchange ON corpus_tool_calls(exchange_id);
CREATE INDEX IF NOT EXISTS idx_corpus_tc_error    ON corpus_tool_calls(is_error);
CREATE INDEX IF NOT EXISTS idx_corpus_tc_name     ON corpus_tool_calls(tool_name);
"#;

#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    #[error("corpus db: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// What one `index_root` pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStats {
    /// Transcript files walked.
    pub seen: u64,
    /// Files parsed and folded in (new or changed).
    pub indexed: u64,
    /// Files whose `(size, mtime)` matched the last index — not re-read.
    pub unchanged: u64,
    /// Files whose session was already represented by a copy at least
    /// as complete. Counted, not hidden: this is the multi-host overlap.
    pub duplicate: u64,
    /// Files that could not be parsed. Never fatal to the pass.
    pub failed: u64,
}

/// Per-host coverage, for the staleness surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCoverage {
    pub host_id: String,
    pub files: u64,
    pub sessions: u64,
    pub newest_ts_ms: Option<i64>,
}

pub struct CorpusIndex {
    db: Mutex<Connection>,
}

impl CorpusIndex {
    /// Open (creating if absent) at `path`. 0600 on Unix, matching every
    /// other Claudepot database.
    pub fn open(path: &Path) -> Result<Self, CorpusError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Pre-create at 0600 BEFORE rusqlite opens it, mirroring
        // `SessionIndex::open`. Creating first and chmod-ing after
        // leaves a window where the file exists at the process umask
        // (typically 0644) and another local user can open it. The
        // corpus holds project paths and prompt previews from every
        // machine, so that window is not acceptable here either.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path);
        }
        let conn = Connection::open(path)?;
        // `busy_timeout` matters: the GUI and the CLI both open this
        // file, and `corpus index` holds write transactions. Without it
        // a concurrent `corpus status` fails instantly with SQLITE_BUSY.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; \
             PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        conn.execute_batch(SCHEMA)?;
        // WAL sidecars are created by SQLite at the umask, so tighten
        // them too — they hold the same content as the main file.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in [
                path.to_path_buf(),
                path.with_extension("db-wal"),
                path.with_extension("db-shm"),
            ] {
                if let Ok(md) = std::fs::metadata(&p) {
                    let mut perm = md.permissions();
                    perm.set_mode(0o600);
                    let _ = std::fs::set_permissions(&p, perm);
                }
            }
        }
        Ok(Self {
            db: Mutex::new(conn),
        })
    }

    /// Test/None-path helper: an in-memory corpus.
    pub fn in_memory() -> Result<Self, CorpusError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            db: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock means an earlier panic mid-write. The corpus
        // is derived data — recovering and continuing beats propagating
        // a panic into a background index pass.
        self.db.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Walk `projects_root` (a `.../projects` directory) and fold every
    /// transcript into the corpus under `host_id`.
    ///
    /// **Incremental and re-runnable.** A file whose `(size, mtime_ns)`
    /// match the recorded values is skipped without being read. Nothing
    /// is ever deleted here: an archive of a live host is a snapshot,
    /// so "present in the index but not on disk right now" is normal,
    /// not stale. The question this asks is *what is missing from the
    /// index*, never *what does the index have that disk does not* —
    /// which is precisely the mistake that makes `SessionIndex::refresh`
    /// unusable for archives.
    pub fn index_root(
        &self,
        host_id: &str,
        projects_root: &Path,
        now_ms: i64,
    ) -> Result<IndexStats, CorpusError> {
        let mut stats = IndexStats::default();
        let Ok(project_dirs) = std::fs::read_dir(projects_root) else {
            return Ok(stats);
        };

        for project in project_dirs.flatten() {
            if !project.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let slug = project.file_name().to_string_lossy().into_owned();
            let Ok(entries) = std::fs::read_dir(project.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                stats.seen += 1;
                match self.index_file(host_id, &slug, &path, now_ms) {
                    Ok(FileOutcome::Unchanged) => stats.unchanged += 1,
                    Ok(FileOutcome::Indexed) => stats.indexed += 1,
                    Ok(FileOutcome::Duplicate) => {
                        stats.indexed += 1;
                        stats.duplicate += 1;
                    }
                    Err(_) => stats.failed += 1,
                }
            }
        }
        Ok(stats)
    }

    fn index_file(
        &self,
        host_id: &str,
        slug: &str,
        path: &Path,
        now_ms: i64,
    ) -> Result<FileOutcome, CorpusError> {
        let md = std::fs::metadata(path)?;
        let size = md.len() as i64;
        let mtime_ns = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        let path_str = path.to_string_lossy().into_owned();

        {
            let db = self.conn();
            // `.optional()`, not `.ok()`: the latter would turn a
            // corrupt database or a locked connection into "cache
            // miss" and quietly re-index, hiding a real failure.
            let fresh: Option<(i64, i64)> = db
                .query_row(
                    "SELECT size_bytes, mtime_ns FROM corpus_files \
                     WHERE host_id = ?1 AND file_path = ?2",
                    params![host_id, path_str],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if fresh == Some((size, mtime_ns)) {
                return Ok(FileOutcome::Unchanged);
            }
        }

        let scan = scan_session(slug, path).map_err(|e| {
            CorpusError::Io(std::io::Error::other(format!(
                "scan {}: {e}",
                path.display()
            )))
        })?;
        let row = scan.row;

        // Second pass for turn-level content. Reuses the exact pairing
        // `shared_memory::claude_exchanges` uses to fill the
        // `sessions.db` equivalents, so `corpus_tool_calls.is_error`
        // means the same thing as `tool_calls.is_error` — the property
        // that made D3's dead `has_error` diagnosable in the first place.
        //
        // This is the expensive half: the session pass folds one row per
        // file; this one materializes every turn and every tool call.
        let exchanges = match crate::session::parse_events_public(path) {
            Ok(events) => crate::shared_memory::claude_exchanges::pair_events_into_exchanges(
                &row.session_id,
                &events,
            ),
            // A file whose events will not re-parse still has a usable
            // session row; losing its turns is better than losing both.
            Err(_) => Vec::new(),
        };

        let mut db = self.conn();
        // One transaction for both writes. Separately, the
        // `corpus_files` row (which carries the freshness guard) could
        // commit while the `corpus_sessions` write failed — and the
        // guard would then make every future pass skip the file, so a
        // missing or stale logical session would persist forever.
        let tx = db.transaction()?;

        tx.execute(
            "INSERT INTO corpus_files \
               (host_id, file_path, session_id, size_bytes, mtime_ns, event_count, indexed_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(host_id, file_path) DO UPDATE SET \
               session_id = excluded.session_id, size_bytes = excluded.size_bytes, \
               mtime_ns = excluded.mtime_ns, event_count = excluded.event_count, \
               indexed_at_ms = excluded.indexed_at_ms",
            params![
                host_id,
                path_str,
                row.session_id,
                size,
                mtime_ns,
                row.event_count as i64,
                now_ms
            ],
        )?;

        // Turn-level content, in the same transaction as the file row.
        // Replace-then-insert rather than upsert: a re-indexed file may
        // have *fewer* turns than before (a truncated copy on another
        // host), and leaving orphaned turns behind would let a detector
        // read turns that are not in the file it is quoting.
        tx.execute(
            "DELETE FROM corpus_tool_calls WHERE file_path = ?1",
            [&path_str],
        )?;
        tx.execute(
            "DELETE FROM corpus_exchanges WHERE file_path = ?1",
            [&path_str],
        )?;
        for ex in &exchanges {
            let ex_id = format!("{}\u{001f}{}", path_str, ex.turn_index);
            tx.execute(
                "INSERT INTO corpus_exchanges \
                   (id, file_path, session_id, turn_index, timestamp_ms, user_text, assistant_text) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    ex_id,
                    path_str,
                    row.session_id,
                    ex.turn_index as i64,
                    ex.timestamp_ms,
                    ex.user_text,
                    ex.assistant_text,
                ],
            )?;
            for (ordinal, tc) in ex.tool_calls.iter().enumerate() {
                tx.execute(
                    "INSERT INTO corpus_tool_calls \
                       (id, exchange_id, file_path, ordinal, tool_name, tool_input_json, \
                        tool_result_text, is_error, timestamp_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        format!("{ex_id}\u{001f}{ordinal}"),
                        ex_id,
                        path_str,
                        ordinal as i64,
                        tc.tool_name,
                        tc.tool_input_json,
                        tc.tool_result_text,
                        tc.is_error as i64,
                        tc.timestamp_ms,
                    ],
                )?;
            }
        }

        let existed: bool = tx
            .query_row(
                "SELECT 1 FROM corpus_sessions WHERE session_id = ?1",
                [&row.session_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();

        // "Most complete copy wins", decided by the database rather than
        // by a read-then-write in Rust. The comparison lives in the
        // conflict clause so two indexers racing on the same session
        // cannot both read the old count and let the shorter copy land
        // last. A session synced mid-flight to a second machine has the
        // same UUID and fewer events there; taking it would silently
        // truncate the record.
        let updated = upsert_session_if_more_complete(&tx, &row, host_id, &path_str, now_ms)?;
        tx.commit()?;

        Ok(if existed || updated == 0 {
            FileOutcome::Duplicate
        } else {
            FileOutcome::Indexed
        })
    }

    pub fn session_count(&self) -> Result<u64, CorpusError> {
        let db = self.conn();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM corpus_sessions", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    }

    pub fn file_count(&self) -> Result<u64, CorpusError> {
        let db = self.conn();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM corpus_files", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    }

    /// Per-host coverage. Powers "<host> last indexed 2026-05-28"
    /// without the caller re-deriving it.
    pub fn host_coverage(&self) -> Result<Vec<HostCoverage>, CorpusError> {
        let db = self.conn();
        let mut stmt = db.prepare(
            "SELECT f.host_id, COUNT(*), COUNT(DISTINCT f.session_id), \
                    MAX(s.last_ts_ms) \
               FROM corpus_files f \
               LEFT JOIN corpus_sessions s ON s.session_id = f.session_id \
              GROUP BY f.host_id ORDER BY f.host_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(HostCoverage {
                host_id: r.get(0)?,
                files: r.get::<_, i64>(1)?.max(0) as u64,
                sessions: r.get::<_, i64>(2)?.max(0) as u64,
                newest_ts_ms: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

enum FileOutcome {
    Indexed,
    Unchanged,
    Duplicate,
}

/// Insert the session, or replace the stored copy **only if this one is
/// more complete**. Returns rows affected — `0` means an existing copy
/// won, i.e. this file is a duplicate.
///
/// The `WHERE excluded.event_count > corpus_sessions.event_count` guard
/// on the conflict clause is what makes the outcome independent of
/// index order and safe against two concurrent indexers.
fn upsert_session_if_more_complete(
    db: &Connection,
    row: &SessionRow,
    host_id: &str,
    file_path: &str,
    now_ms: i64,
) -> Result<usize, CorpusError> {
    let n = db.execute(
        "INSERT INTO corpus_sessions \
           (session_id, project_path, slug, first_ts_ms, last_ts_ms, event_count, \
            message_count, user_message_count, assistant_message_count, first_user_prompt, \
            models_json, git_branch, cc_version, has_error, is_sidechain, \
            best_host_id, best_file_path, indexed_at_ms) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18) \
         ON CONFLICT(session_id) DO UPDATE SET \
           project_path = excluded.project_path, slug = excluded.slug, \
           first_ts_ms = excluded.first_ts_ms, last_ts_ms = excluded.last_ts_ms, \
           event_count = excluded.event_count, message_count = excluded.message_count, \
           user_message_count = excluded.user_message_count, \
           assistant_message_count = excluded.assistant_message_count, \
           first_user_prompt = excluded.first_user_prompt, models_json = excluded.models_json, \
           git_branch = excluded.git_branch, cc_version = excluded.cc_version, \
           has_error = excluded.has_error, is_sidechain = excluded.is_sidechain, \
           best_host_id = excluded.best_host_id, best_file_path = excluded.best_file_path, \
           indexed_at_ms = excluded.indexed_at_ms \
         WHERE excluded.event_count > corpus_sessions.event_count",
        params![
            row.session_id,
            row.project_path,
            row.slug,
            row.first_ts.map(|t| t.timestamp_millis()),
            row.last_ts.map(|t| t.timestamp_millis()),
            row.event_count as i64,
            row.message_count as i64,
            row.user_message_count as i64,
            row.assistant_message_count as i64,
            row.first_user_prompt,
            serde_json::to_string(&row.models).unwrap_or_else(|_| "[]".into()),
            row.git_branch,
            row.cc_version,
            row.has_error as i64,
            row.is_sidechain as i64,
            host_id,
            file_path,
            now_ms,
        ],
    )?;
    Ok(n)
}

/// Default corpus database location.
pub fn default_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join("corpus.db")
}

/// Where the rescue writes imported hosts. One subdirectory per host,
/// each containing a `projects/` tree mirroring `~/.claude/projects`.
pub fn default_archive_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("claude-corpus-archive"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_transcript(root: &Path, slug: &str, session_id: &str, lines: &[&str]) -> PathBuf {
        let dir = root.join(slug);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{session_id}.jsonl"));
        fs::write(&p, format!("{}\n", lines.join("\n"))).unwrap();
        p
    }

    fn user_line(text: &str, ts: &str, cwd: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{text}"}},"timestamp":"{ts}","cwd":"{cwd}"}}"#
        )
    }

    fn corpus() -> CorpusIndex {
        CorpusIndex::in_memory().unwrap()
    }

    #[test]
    fn indexes_a_transcript_and_counts_it_once() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        write_transcript(
            &root,
            "-repo-foo",
            "S1",
            &[&user_line("hi", "2026-04-10T10:00:00Z", "/repo/foo")],
        );

        let c = corpus();
        let s = c.index_root(LOCAL_HOST, &root, 1).unwrap();
        assert_eq!(s.seen, 1);
        assert_eq!(s.indexed, 1);
        assert_eq!(c.session_count().unwrap(), 1);
        assert_eq!(c.file_count().unwrap(), 1);
    }

    /// Re-running must not re-read unchanged files — this is what makes
    /// indexing 14,059 transcripts repeatable rather than a one-shot.
    #[test]
    fn a_second_pass_skips_unchanged_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        write_transcript(
            &root,
            "-repo-foo",
            "S1",
            &[&user_line("hi", "2026-04-10T10:00:00Z", "/repo/foo")],
        );
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();
        let s2 = c.index_root(LOCAL_HOST, &root, 2).unwrap();
        assert_eq!(s2.unchanged, 1);
        assert_eq!(s2.indexed, 0);
    }

    /// The multi-host case: the same conversation on two machines is one
    /// session, two files. Counting it twice would inflate every
    /// aggregate the detectors compute.
    #[test]
    fn the_same_session_on_two_hosts_is_one_session_two_files() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("host-a/projects");
        let b = tmp.path().join("host-b/projects");
        let line = user_line("hi", "2026-04-10T10:00:00Z", "/repo/foo");
        write_transcript(&a, "-repo-foo", "SHARED", &[&line]);
        write_transcript(&b, "-repo-foo", "SHARED", &[&line]);

        let c = corpus();
        c.index_root("host-a", &a, 1).unwrap();
        let s = c.index_root("host-b", &b, 1).unwrap();

        assert_eq!(s.duplicate, 1, "second host's copy is a duplicate");
        assert_eq!(c.session_count().unwrap(), 1);
        assert_eq!(c.file_count().unwrap(), 2, "both physical copies recorded");
    }

    /// A host that synced mid-session holds a truncated copy. Taking it
    /// would silently shorten the record.
    #[test]
    fn the_more_complete_copy_wins() {
        let tmp = TempDir::new().unwrap();
        let short_root = tmp.path().join("short/projects");
        let long_root = tmp.path().join("long/projects");
        let l1 = user_line("one", "2026-04-10T10:00:00Z", "/repo/foo");
        let l2 = user_line("two", "2026-04-10T10:01:00Z", "/repo/foo");
        write_transcript(&short_root, "-repo-foo", "S1", &[&l1]);
        write_transcript(&long_root, "-repo-foo", "S1", &[&l1, &l2]);

        let c = corpus();
        c.index_root("short-host", &short_root, 1).unwrap();
        c.index_root("long-host", &long_root, 1).unwrap();
        let db = c.conn();
        let (events, host): (i64, String) = db
            .query_row(
                "SELECT event_count, best_host_id FROM corpus_sessions WHERE session_id='S1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(events, 2);
        assert_eq!(host, "long-host");
    }

    /// ...and the reverse order must reach the same answer.
    #[test]
    fn a_truncated_copy_indexed_second_does_not_overwrite() {
        let tmp = TempDir::new().unwrap();
        let short_root = tmp.path().join("short/projects");
        let long_root = tmp.path().join("long/projects");
        let l1 = user_line("one", "2026-04-10T10:00:00Z", "/repo/foo");
        let l2 = user_line("two", "2026-04-10T10:01:00Z", "/repo/foo");
        write_transcript(&short_root, "-repo-foo", "S1", &[&l1]);
        write_transcript(&long_root, "-repo-foo", "S1", &[&l1, &l2]);

        let c = corpus();
        c.index_root("long-host", &long_root, 1).unwrap();
        c.index_root("short-host", &short_root, 1).unwrap();
        let db = c.conn();
        let (events, host): (i64, String) = db
            .query_row(
                "SELECT event_count, best_host_id FROM corpus_sessions WHERE session_id='S1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(events, 2, "the shorter copy must not win by arriving later");
        assert_eq!(host, "long-host");
    }

    /// Nothing is ever deleted: an archive of a live host is a snapshot,
    /// so a file that is gone now is not evidence the record is stale.
    /// This is the exact property `SessionIndex::refresh` lacks.
    #[test]
    fn indexing_never_deletes_rows_for_vanished_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let p = write_transcript(
            &root,
            "-repo-foo",
            "S1",
            &[&user_line("hi", "2026-04-10T10:00:00Z", "/repo/foo")],
        );
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();
        fs::remove_file(&p).unwrap();

        c.index_root(LOCAL_HOST, &root, 2).unwrap();
        assert_eq!(c.session_count().unwrap(), 1, "the record must survive");
        assert_eq!(c.file_count().unwrap(), 1);
    }

    #[test]
    fn a_changed_file_is_reindexed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let l1 = user_line("one", "2026-04-10T10:00:00Z", "/repo/foo");
        let p = write_transcript(&root, "-repo-foo", "S1", &[&l1]);
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();

        let l2 = user_line("two", "2026-04-10T10:01:00Z", "/repo/foo");
        fs::write(&p, format!("{l1}\n{l2}\n")).unwrap();
        let s = c.index_root(LOCAL_HOST, &root, 2).unwrap();
        assert_eq!(s.indexed, 1);
        assert_eq!(s.unchanged, 0);
    }

    #[test]
    fn an_unparseable_file_is_counted_not_fatal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        write_transcript(
            &root,
            "-repo-foo",
            "GOOD",
            &[&user_line("hi", "2026-04-10T10:00:00Z", "/repo/foo")],
        );
        let dir = root.join("-repo-foo");
        fs::write(dir.join("BAD.jsonl"), b"not json at all\n").unwrap();

        let c = corpus();
        let s = c.index_root(LOCAL_HOST, &root, 1).unwrap();
        assert_eq!(s.seen, 2);
        // A malformed transcript still yields a row (CC's own tolerance
        // is line-level), so what matters is that the pass completed.
        assert!(s.indexed >= 1);
        assert!(c.session_count().unwrap() >= 1);
    }

    // ─── turn-level content (the C2 substrate) ──────────────────────

    fn assistant_line(text: &str, ts: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"{text}"}}]}},"timestamp":"{ts}","cwd":"/repo/foo"}}"#
        )
    }

    fn tool_use_line(id: &str, name: &str, ts: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{{"command":"x"}}}}]}},"timestamp":"{ts}","cwd":"/repo/foo"}}"#
        )
    }

    fn tool_result_line(id: &str, text: &str, is_error: bool, ts: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{id}","content":"{text}","is_error":{is_error}}}]}},"timestamp":"{ts}","cwd":"/repo/foo"}}"#
        )
    }

    #[test]
    fn exchanges_and_tool_calls_are_indexed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        write_transcript(
            &root,
            "-repo-foo",
            "S1",
            &[
                &user_line("build it", "2026-04-10T10:00:00Z", "/repo/foo"),
                &tool_use_line("t1", "Bash", "2026-04-10T10:00:01Z"),
                &tool_result_line("t1", "Exit code 1", true, "2026-04-10T10:00:02Z"),
                &assistant_line("fixed", "2026-04-10T10:00:03Z"),
            ],
        );
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();

        let db = c.conn();
        let ex: i64 = db
            .query_row("SELECT COUNT(*) FROM corpus_exchanges", [], |r| r.get(0))
            .unwrap();
        assert!(ex > 0, "turns must be materialized");
        let tc: i64 = db
            .query_row("SELECT COUNT(*) FROM corpus_tool_calls", [], |r| r.get(0))
            .unwrap();
        assert!(tc > 0, "tool calls must be materialized");
    }

    /// The contract's acceptance condition: Tier 3 is unbuildable
    /// without a truthy `is_error`, and this is the column that made
    /// `sessions.has_error` diagnosable (D3).
    #[test]
    fn a_failed_tool_call_is_recorded_with_is_error_and_its_output() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        write_transcript(
            &root,
            "-repo-foo",
            "S1",
            &[
                &user_line("build it", "2026-04-10T10:00:00Z", "/repo/foo"),
                &tool_use_line("t1", "Bash", "2026-04-10T10:00:01Z"),
                &tool_result_line("t1", "Exit code 1", true, "2026-04-10T10:00:02Z"),
                &assistant_line("ok", "2026-04-10T10:00:03Z"),
            ],
        );
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();

        let db = c.conn();
        let (name, text, err): (String, Option<String>, i64) = db
            .query_row(
                "SELECT tool_name, tool_result_text, is_error FROM corpus_tool_calls \
                 WHERE is_error = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "Bash");
        assert_eq!(err, 1);
        assert!(
            text.unwrap_or_default().contains("Exit code 1"),
            "the failure output is the evidence a detector quotes"
        );
    }

    /// A re-index must not leave turns from the previous read behind —
    /// a shorter copy would otherwise let a detector quote turns that
    /// are not in the file it names.
    #[test]
    fn reindexing_replaces_turns_rather_than_accumulating() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let l1 = user_line("one", "2026-04-10T10:00:00Z", "/repo/foo");
        let a1 = assistant_line("first", "2026-04-10T10:00:01Z");
        let l2 = user_line("two", "2026-04-10T10:01:00Z", "/repo/foo");
        let a2 = assistant_line("second", "2026-04-10T10:01:01Z");
        let p = write_transcript(&root, "-repo-foo", "S1", &[&l1, &a1, &l2, &a2]);
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();
        let before: i64 = c
            .conn()
            .query_row("SELECT COUNT(*) FROM corpus_exchanges", [], |r| r.get(0))
            .unwrap();
        assert!(before >= 2);

        // Rewrite shorter.
        fs::write(&p, format!("{l1}\n{a1}\n")).unwrap();
        c.index_root(LOCAL_HOST, &root, 2).unwrap();
        let after: i64 = c
            .conn()
            .query_row("SELECT COUNT(*) FROM corpus_exchanges", [], |r| r.get(0))
            .unwrap();
        assert!(
            after < before,
            "stale turns must be removed, not accumulated ({before} -> {after})"
        );
    }

    #[test]
    fn tool_calls_cascade_when_their_exchange_is_replaced() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let lines = [
            user_line("build", "2026-04-10T10:00:00Z", "/repo/foo"),
            tool_use_line("t1", "Bash", "2026-04-10T10:00:01Z"),
            tool_result_line("t1", "boom", true, "2026-04-10T10:00:02Z"),
            assistant_line("ok", "2026-04-10T10:00:03Z"),
        ];
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(&root, "-repo-foo", "S1", &refs);
        let c = corpus();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();

        // Replace with a transcript containing no tool calls at all.
        fs::write(
            &p,
            format!(
                "{}\n{}\n",
                user_line("hi", "2026-04-10T10:00:00Z", "/repo/foo"),
                assistant_line("hello", "2026-04-10T10:00:01Z")
            ),
        )
        .unwrap();
        c.index_root(LOCAL_HOST, &root, 2).unwrap();

        let orphans: i64 = c
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM corpus_tool_calls tc \
                 LEFT JOIN corpus_exchanges e ON e.id = tc.exchange_id \
                 WHERE e.id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0, "no tool call may outlive its exchange");
    }

    #[test]
    fn host_coverage_reports_each_host() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a/projects");
        let b = tmp.path().join("b/projects");
        write_transcript(
            &a,
            "-p",
            "S1",
            &[&user_line("x", "2026-04-10T10:00:00Z", "/p")],
        );
        write_transcript(
            &b,
            "-p",
            "S2",
            &[&user_line("y", "2026-04-11T10:00:00Z", "/p")],
        );
        let c = corpus();
        c.index_root("host-a", &a, 1).unwrap();
        c.index_root("host-b", &b, 1).unwrap();

        let cov = c.host_coverage().unwrap();
        assert_eq!(cov.len(), 2);
        assert_eq!(cov[0].host_id, "host-a");
        assert_eq!(cov[0].files, 1);
        assert_eq!(cov[1].host_id, "host-b");
    }

    #[test]
    fn a_missing_root_is_empty_not_an_error() {
        let tmp = TempDir::new().unwrap();
        let c = corpus();
        let s = c
            .index_root(LOCAL_HOST, &tmp.path().join("nope"), 1)
            .unwrap();
        assert_eq!(s, IndexStats::default());
    }

    #[test]
    fn non_jsonl_files_are_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let dir = root.join("-repo-foo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("notes.md"), b"hello").unwrap();
        let c = corpus();
        assert_eq!(c.index_root(LOCAL_HOST, &root, 1).unwrap().seen, 0);
    }
}
