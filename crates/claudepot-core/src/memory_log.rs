//! Persistent change log for memory file edits.
//!
//! Lives at `~/.claudepot/memory_changes.db`. Every detected write to a
//! memory-shaped file (project or global `CLAUDE.md`, files inside an
//! auto-memory dir) becomes one row carrying:
//!
//! - The role (per [`MemoryFileRole`]) so the UI can label the entry.
//! - A `(size, sha256)` pair before and after, for cheap "did anything
//!   change?" filtering even when the diff is omitted.
//! - The unified diff text, when both sides are valid UTF-8 and the
//!   larger of the two stays under [`MAX_DIFF_FILE_BYTES`].
//!
//! Eviction runs on every insert: rows are dropped past the per-file
//! cap ([`PER_FILE_RING_CAP`]), past the global cap
//! ([`GLOBAL_ROW_CAP`]), and past the age cap ([`MAX_ROW_AGE_NS`]).
//! Whichever fires first.
//!
//! This module is intentionally NOT a generic fs-event store. Callers
//! decide whether a path is in scope (via `memory_view::is_allowed` or
//! a watcher-side filter) before recording. Recording an out-of-scope
//! path still works, but the surface stays minimal so unrelated edits
//! don't pollute the log.

use crate::memory_view::MemoryFileRole;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Maximum file size we'll diff. Above this, the row is recorded with
/// `diff_text = NULL` and a placeholder marker — the UI renders
/// "(file too large for inline diff)".
pub const MAX_DIFF_FILE_BYTES: usize = 256 * 1024;

/// How many rows to keep per file. Once exceeded, oldest rows for that
/// file are dropped.
pub const PER_FILE_RING_CAP: usize = 500;

/// Hard cap on total rows. When exceeded, oldest rows globally are
/// dropped — independent of the per-file cap.
pub const GLOBAL_ROW_CAP: usize = 20_000;

/// Drop rows older than this. 90 days × 86400 s × 1e9 ns.
pub const MAX_ROW_AGE_NS: i64 = 90 * 86_400 * 1_000_000_000;

const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_changes (
    id              INTEGER PRIMARY KEY,
    project_slug    TEXT,
    abs_path        TEXT NOT NULL,
    role            TEXT NOT NULL,
    change_type     TEXT NOT NULL,
    detected_at_ns  INTEGER NOT NULL,
    mtime_ns        INTEGER NOT NULL,
    size_before     INTEGER,
    size_after      INTEGER,
    hash_before     TEXT,
    hash_after      TEXT,
    diff_text       TEXT,
    diff_omitted    INTEGER NOT NULL DEFAULT 0,
    diff_omit_reason TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_changes_project_time
    ON memory_changes(project_slug, detected_at_ns DESC);
CREATE INDEX IF NOT EXISTS idx_memory_changes_path_time
    ON memory_changes(abs_path, detected_at_ns DESC);
"#;

#[derive(Debug, thiserror::Error)]
pub enum MemoryLogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sql: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// Whether the recorded event is a creation, modification, or deletion.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Created,
    Modified,
    Deleted,
}

impl ChangeType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "created" => Some(Self::Created),
            "modified" => Some(Self::Modified),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// Why the diff text is `NULL`. Recorded so the UI can render an
/// honest placeholder instead of a confusing empty diff.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DiffOmitReason {
    /// File size on either side exceeds [`MAX_DIFF_FILE_BYTES`].
    TooLarge,
    /// Either side is invalid UTF-8 — most likely a binary file.
    Binary,
    /// Creation or deletion event; there's no "other side" to diff
    /// against. Content is implied (full file = before, or full file
    /// = after) and the UI renders it as a normal viewer rather than
    /// a +/- gutter.
    Endpoint,
    /// First time we've seen this path. We have no prior content to
    /// diff against, so the row is recorded as a baseline.
    Baseline,
}

impl DiffOmitReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::Binary => "binary",
            Self::Endpoint => "endpoint",
            Self::Baseline => "baseline",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "too_large" => Some(Self::TooLarge),
            "binary" => Some(Self::Binary),
            "endpoint" => Some(Self::Endpoint),
            "baseline" => Some(Self::Baseline),
            _ => None,
        }
    }
}

/// One persisted change-log entry. Returned by query APIs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryChange {
    pub id: i64,
    pub project_slug: Option<String>,
    pub abs_path: PathBuf,
    pub role: MemoryFileRole,
    pub change_type: ChangeType,
    pub detected_at_ns: i64,
    pub mtime_ns: i64,
    pub size_before: Option<i64>,
    pub size_after: Option<i64>,
    pub hash_before: Option<String>,
    pub hash_after: Option<String>,
    pub diff_text: Option<String>,
    pub diff_omitted: bool,
    pub diff_omit_reason: Option<DiffOmitReason>,
}

/// What the caller hands us at record time. `before` and `after` are
/// the raw bytes (not strings — binary files would otherwise be
/// rejected at the type level). `None` means "no content on this
/// side" (creation has no before, deletion has no after).
#[derive(Clone, Debug)]
pub struct RecordInput<'a> {
    pub project_slug: Option<&'a str>,
    pub abs_path: &'a Path,
    pub role: MemoryFileRole,
    pub change_type: ChangeType,
    pub mtime_ns: i64,
    pub before: Option<&'a [u8]>,
    pub after: Option<&'a [u8]>,
}

/// Query parameters for [`MemoryLog::query_for_project`] and
/// [`MemoryLog::query_for_path`]. All fields optional; defaults give a
/// sane "newest 50" answer.
#[derive(Clone, Debug, Default)]
pub struct ChangeQuery {
    /// Lower bound on `detected_at_ns`. Inclusive.
    pub since_ns: Option<i64>,
    /// Upper bound on `detected_at_ns`. Exclusive.
    pub until_ns: Option<i64>,
    /// Maximum rows to return. Defaults to 50.
    pub limit: Option<usize>,
}

impl ChangeQuery {
    fn effective_limit(&self) -> i64 {
        self.limit.unwrap_or(50).min(10_000) as i64
    }
}

/// Persistent log handle. Wrap the connection in a `Mutex` so handles
/// can cross `await` points (mirrors `session_index::SessionIndex`).
pub struct MemoryLog {
    db: Mutex<Connection>,
}

impl MemoryLog {
    /// Open the log at `path`. Creates the parent directory, applies
    /// the schema, and (on Unix) enforces 0600 perms on the main file
    /// + WAL/SHM sidecars.
    ///
    /// Idempotent — re-opening an existing log is safe. A corrupt DB
    /// is renamed aside as `<name>.corrupt-<epoch_ms>` and a fresh one
    /// is created; the change log is a derived cache, never primary
    /// state, so wipe-and-rebuild is always a safe recovery.
    pub fn open(path: &Path) -> Result<Self, MemoryLogError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = match Self::init_connection(path) {
            Ok(c) => c,
            Err(MemoryLogError::Sql(e)) if is_corrupt(&e) => {
                quarantine(path)?;
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

    fn init_connection(path: &Path) -> Result<Connection, MemoryLogError> {
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(SCHEMA)?;
        // Touch meta + force WAL/SHM materialization so the chmod step
        // below narrows real files, not phantoms (mirrors the
        // session_index pattern that fixed the same leak).
        db.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )?;
        Ok(db)
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Insert a row for the given change. Runs eviction in the same
    /// transaction so the log can never grow past the caps even under
    /// crash recovery.
    ///
    /// Returns the new row's id.
    pub fn record(&self, input: &RecordInput<'_>) -> Result<i64, MemoryLogError> {
        let detected_at_ns = unix_ns_now();
        let (size_before, hash_before) = digest_pair(input.before);
        let (size_after, hash_after) = digest_pair(input.after);
        let (diff_text, omit_reason) = compute_diff(input);

        let abs = input.abs_path.to_string_lossy().into_owned();
        let role_str = serde_json::to_value(input.role)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "auto_memory_topic".to_string());

        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO memory_changes (
                project_slug, abs_path, role, change_type, detected_at_ns,
                mtime_ns, size_before, size_after, hash_before, hash_after,
                diff_text, diff_omitted, diff_omit_reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                input.project_slug,
                abs,
                role_str,
                input.change_type.as_str(),
                detected_at_ns,
                input.mtime_ns,
                size_before,
                size_after,
                hash_before,
                hash_after,
                diff_text,
                if diff_text.is_some() { 0i64 } else { 1i64 },
                omit_reason.map(|r| r.as_str()),
            ],
        )?;
        let id = tx.last_insert_rowid();

        // Eviction — same transaction, so the log can't transiently
        // exceed any cap even if we crash between insert and evict.
        evict_for_path(&tx, &abs)?;
        evict_global(&tx)?;
        evict_by_age(&tx, detected_at_ns)?;

        tx.commit()?;
        Ok(id)
    }

    /// Most recent rows for a single file, newest first.
    pub fn query_for_path(
        &self,
        abs_path: &Path,
        q: &ChangeQuery,
    ) -> Result<Vec<MemoryChange>, MemoryLogError> {
        let db = self.db();
        let abs = abs_path.to_string_lossy().into_owned();
        let mut sql = String::from(
            "SELECT id, project_slug, abs_path, role, change_type, detected_at_ns, \
             mtime_ns, size_before, size_after, hash_before, hash_after, \
             diff_text, diff_omitted, diff_omit_reason \
             FROM memory_changes WHERE abs_path = ?1",
        );
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(abs)];
        push_time_filters(&mut sql, &mut bound, q);
        sql.push_str(" ORDER BY detected_at_ns DESC LIMIT ?");
        bound.push(Box::new(q.effective_limit()));

        let mut stmt = db.prepare(&sql)?;
        let bound_refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(bound_refs.as_slice(), row_to_change)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Most recent rows for a project (all files), newest first.
    pub fn query_for_project(
        &self,
        project_slug: &str,
        q: &ChangeQuery,
    ) -> Result<Vec<MemoryChange>, MemoryLogError> {
        let db = self.db();
        let mut sql = String::from(
            "SELECT id, project_slug, abs_path, role, change_type, detected_at_ns, \
             mtime_ns, size_before, size_after, hash_before, hash_after, \
             diff_text, diff_omitted, diff_omit_reason \
             FROM memory_changes WHERE project_slug = ?1",
        );
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_slug.to_string())];
        push_time_filters(&mut sql, &mut bound, q);
        sql.push_str(" ORDER BY detected_at_ns DESC LIMIT ?");
        bound.push(Box::new(q.effective_limit()));

        let mut stmt = db.prepare(&sql)?;
        let bound_refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(bound_refs.as_slice(), row_to_change)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Aggregates per file for the given project: most recent
    /// `detected_at_ns` and the count of rows in the last 30 days.
    /// Used by the file list to render "● modified 3 min ago" badges
    /// without one query per file.
    pub fn project_file_stats(
        &self,
        project_slug: &str,
    ) -> Result<Vec<MemoryFileStats>, MemoryLogError> {
        let cutoff = unix_ns_now() - 30 * 86_400 * 1_000_000_000;
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT abs_path,
                    MAX(detected_at_ns),
                    SUM(CASE WHEN detected_at_ns >= ?2 THEN 1 ELSE 0 END)
             FROM memory_changes
             WHERE project_slug = ?1
             GROUP BY abs_path",
        )?;
        let rows = stmt
            .query_map(params![project_slug, cutoff], |r| {
                let abs: String = r.get(0)?;
                let last: i64 = r.get(1)?;
                let cnt: i64 = r.get(2)?;
                Ok(MemoryFileStats {
                    abs_path: PathBuf::from(abs),
                    last_change_unix_ns: Some(last),
                    change_count_30d: cnt.max(0) as u32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Total row count. Test + diagnostics hook.
    pub fn row_count(&self) -> Result<i64, MemoryLogError> {
        let db = self.db();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM memory_changes", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Latest row for a single path, if any. Used by the watcher to
    /// rebuild its in-memory fingerprint cache after a restart.
    pub fn latest_for_path(&self, abs_path: &Path) -> Result<Option<MemoryChange>, MemoryLogError> {
        let db = self.db();
        let abs = abs_path.to_string_lossy().into_owned();
        let row = db
            .query_row(
                "SELECT id, project_slug, abs_path, role, change_type, detected_at_ns, \
                 mtime_ns, size_before, size_after, hash_before, hash_after, \
                 diff_text, diff_omitted, diff_omit_reason \
                 FROM memory_changes WHERE abs_path = ?1 \
                 ORDER BY detected_at_ns DESC LIMIT 1",
                params![abs],
                row_to_change,
            )
            .optional()?;
        Ok(row)
    }
}

fn push_time_filters(sql: &mut String, bound: &mut Vec<Box<dyn rusqlite::ToSql>>, q: &ChangeQuery) {
    if let Some(since) = q.since_ns {
        sql.push_str(" AND detected_at_ns >= ?");
        bound.push(Box::new(since));
    }
    if let Some(until) = q.until_ns {
        sql.push_str(" AND detected_at_ns < ?");
        bound.push(Box::new(until));
    }
}

fn row_to_change(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryChange> {
    let id: i64 = r.get(0)?;
    let project_slug: Option<String> = r.get(1)?;
    let abs_path: String = r.get(2)?;
    let role_str: String = r.get(3)?;
    let change_type_str: String = r.get(4)?;
    let detected_at_ns: i64 = r.get(5)?;
    let mtime_ns: i64 = r.get(6)?;
    let size_before: Option<i64> = r.get(7)?;
    let size_after: Option<i64> = r.get(8)?;
    let hash_before: Option<String> = r.get(9)?;
    let hash_after: Option<String> = r.get(10)?;
    let diff_text: Option<String> = r.get(11)?;
    let diff_omitted: i64 = r.get(12)?;
    let diff_omit_reason: Option<String> = r.get(13)?;
    let role: MemoryFileRole = serde_json::from_value(serde_json::Value::String(role_str))
        .unwrap_or(MemoryFileRole::AutoMemoryTopic);
    let change_type = ChangeType::from_str(&change_type_str).unwrap_or(ChangeType::Modified);
    let omit_reason = diff_omit_reason
        .as_deref()
        .and_then(DiffOmitReason::from_str);
    Ok(MemoryChange {
        id,
        project_slug,
        abs_path: PathBuf::from(abs_path),
        role,
        change_type,
        detected_at_ns,
        mtime_ns,
        size_before,
        size_after,
        hash_before,
        hash_after,
        diff_text,
        diff_omitted: diff_omitted != 0,
        diff_omit_reason: omit_reason,
    })
}

/// Per-file aggregates returned by [`MemoryLog::project_file_stats`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryFileStats {
    pub abs_path: PathBuf,
    pub last_change_unix_ns: Option<i64>,
    pub change_count_30d: u32,
}

fn evict_for_path(tx: &rusqlite::Transaction<'_>, abs: &str) -> rusqlite::Result<()> {
    // Drop oldest rows for this path beyond the cap.
    tx.execute(
        "DELETE FROM memory_changes WHERE id IN (
            SELECT id FROM memory_changes
            WHERE abs_path = ?1
            ORDER BY detected_at_ns DESC
            LIMIT -1 OFFSET ?2
         )",
        params![abs, PER_FILE_RING_CAP as i64],
    )?;
    Ok(())
}

fn evict_global(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM memory_changes WHERE id IN (
            SELECT id FROM memory_changes
            ORDER BY detected_at_ns DESC
            LIMIT -1 OFFSET ?1
         )",
        params![GLOBAL_ROW_CAP as i64],
    )?;
    Ok(())
}

fn evict_by_age(tx: &rusqlite::Transaction<'_>, now_ns: i64) -> rusqlite::Result<()> {
    let cutoff = now_ns.saturating_sub(MAX_ROW_AGE_NS);
    tx.execute(
        "DELETE FROM memory_changes WHERE detected_at_ns < ?1",
        params![cutoff],
    )?;
    Ok(())
}

fn unix_ns_now() -> i64 {
    let dt = Utc::now();
    let secs = dt.timestamp();
    let nsec = dt.timestamp_subsec_nanos() as i64;
    secs.saturating_mul(1_000_000_000).saturating_add(nsec)
}

fn digest_pair(content: Option<&[u8]>) -> (Option<i64>, Option<String>) {
    match content {
        None => (None, None),
        Some(bytes) => {
            let mut h = Sha256::new();
            h.update(bytes);
            let hash = hex::encode(h.finalize());
            (Some(bytes.len() as i64), Some(hash))
        }
    }
}

fn compute_diff(input: &RecordInput<'_>) -> (Option<String>, Option<DiffOmitReason>) {
    match (input.before, input.after) {
        // Endpoint events: no diff to compute.
        (None, _) | (_, None) => (None, Some(DiffOmitReason::Endpoint)),
        (Some(b), Some(a)) => {
            if b.len() > MAX_DIFF_FILE_BYTES || a.len() > MAX_DIFF_FILE_BYTES {
                return (None, Some(DiffOmitReason::TooLarge));
            }
            let (Ok(bs), Ok(as_)) = (std::str::from_utf8(b), std::str::from_utf8(a)) else {
                return (None, Some(DiffOmitReason::Binary));
            };
            // No-op write (size+mtime change without a real text
            // change): skip the diff but record the row so the user
            // sees the touch. Mark as endpoint so the UI doesn't try
            // to render an empty diff.
            if bs == as_ {
                return (None, Some(DiffOmitReason::Endpoint));
            }
            let diff = similar::TextDiff::from_lines(bs, as_);
            let header_a = "before";
            let header_b = "after";
            let unified = diff
                .unified_diff()
                .header(header_a, header_b)
                .context_radius(3)
                .to_string();
            (Some(unified), None)
        }
    }
}

fn is_corrupt(e: &rusqlite::Error) -> bool {
    use rusqlite::ffi::ErrorCode;
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseCorrupt) | Some(ErrorCode::NotADatabase)
    )
}

fn quarantine(path: &Path) -> Result<(), MemoryLogError> {
    let suffix = chrono::Utc::now().timestamp_millis();
    let mut moved = path.to_path_buf();
    let new_name = format!(
        "{}.corrupt-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("memory_changes.db"),
        suffix
    );
    moved.set_file_name(new_name);
    if path.exists() {
        std::fs::rename(path, &moved)?;
        tracing::warn!("memory_log: quarantined corrupt db to {}", moved.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn open_log() -> (MemoryLog, TempDir) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("memory_changes.db");
        (MemoryLog::open(&path).unwrap(), tmp)
    }

    #[test]
    fn open_creates_file_and_schema() {
        let (log, _t) = open_log();
        assert_eq!(log.row_count().unwrap(), 0);
    }

    #[test]
    fn open_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("memory_changes.db");
        let _a = MemoryLog::open(&path).unwrap();
        let _b = MemoryLog::open(&path).unwrap();
    }

    #[test]
    fn record_modification_with_diff() {
        let (log, _t) = open_log();
        let id = log
            .record(&RecordInput {
                project_slug: Some("test"),
                abs_path: Path::new("/m/MEMORY.md"),
                role: MemoryFileRole::AutoMemoryIndex,
                change_type: ChangeType::Modified,
                mtime_ns: 0,
                before: Some(b"line1\nline2\n"),
                after: Some(b"line1\nline2 modified\n"),
            })
            .unwrap();
        assert!(id > 0);
        let rows = log
            .query_for_path(Path::new("/m/MEMORY.md"), &ChangeQuery::default())
            .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.change_type, ChangeType::Modified);
        assert_eq!(row.role, MemoryFileRole::AutoMemoryIndex);
        let diff = row.diff_text.as_ref().expect("diff text");
        // Unified-diff format: header lines + a hunk that drops "line2"
        // and adds "line2 modified". Don't pin the exact bytes — just
        // assert structural shape.
        assert!(diff.contains("--- before"));
        assert!(diff.contains("+++ after"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+line2 modified"));
        assert!(!row.diff_omitted);
    }

    #[test]
    fn record_creation_omits_diff_with_endpoint_reason() {
        let (log, _t) = open_log();
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/MEMORY.md"),
            role: MemoryFileRole::AutoMemoryIndex,
            change_type: ChangeType::Created,
            mtime_ns: 0,
            before: None,
            after: Some(b"new content\n"),
        })
        .unwrap();
        let rows = log
            .query_for_path(Path::new("/m/MEMORY.md"), &ChangeQuery::default())
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].diff_text.is_none());
        assert_eq!(rows[0].diff_omit_reason, Some(DiffOmitReason::Endpoint));
        assert!(rows[0].size_after.is_some());
        assert!(rows[0].size_before.is_none());
    }

    #[test]
    fn record_deletion_keeps_before_drops_after() {
        let (log, _t) = open_log();
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/topic.md"),
            role: MemoryFileRole::AutoMemoryTopic,
            change_type: ChangeType::Deleted,
            mtime_ns: 0,
            before: Some(b"old content\n"),
            after: None,
        })
        .unwrap();
        let rows = log
            .query_for_path(Path::new("/m/topic.md"), &ChangeQuery::default())
            .unwrap();
        assert_eq!(rows[0].size_before, Some(12));
        assert_eq!(rows[0].size_after, None);
        assert_eq!(rows[0].change_type, ChangeType::Deleted);
    }

    #[test]
    fn record_oversize_omits_diff_with_too_large_reason() {
        let (log, _t) = open_log();
        let big_a = vec![b'a'; MAX_DIFF_FILE_BYTES + 10];
        let big_b = vec![b'b'; MAX_DIFF_FILE_BYTES + 10];
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/big.md"),
            role: MemoryFileRole::AutoMemoryTopic,
            change_type: ChangeType::Modified,
            mtime_ns: 0,
            before: Some(&big_a),
            after: Some(&big_b),
        })
        .unwrap();
        let rows = log
            .query_for_path(Path::new("/m/big.md"), &ChangeQuery::default())
            .unwrap();
        assert!(rows[0].diff_text.is_none());
        assert_eq!(rows[0].diff_omit_reason, Some(DiffOmitReason::TooLarge));
    }

    #[test]
    fn record_binary_omits_diff_with_binary_reason() {
        let (log, _t) = open_log();
        let bin_a = vec![0u8, 0xFF, 0xFE, 0x00];
        let bin_b = vec![0u8, 0xFF, 0xFE, 0x01];
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/b.md"),
            role: MemoryFileRole::AutoMemoryTopic,
            change_type: ChangeType::Modified,
            mtime_ns: 0,
            before: Some(&bin_a),
            after: Some(&bin_b),
        })
        .unwrap();
        let rows = log
            .query_for_path(Path::new("/m/b.md"), &ChangeQuery::default())
            .unwrap();
        assert_eq!(rows[0].diff_omit_reason, Some(DiffOmitReason::Binary));
    }

    #[test]
    fn record_no_op_write_records_endpoint() {
        // Watcher sees an mtime bump but content unchanged (someone
        // ran `touch MEMORY.md`). We still record the row so the
        // user sees the event, but suppress the empty diff.
        let (log, _t) = open_log();
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/MEMORY.md"),
            role: MemoryFileRole::AutoMemoryIndex,
            change_type: ChangeType::Modified,
            mtime_ns: 0,
            before: Some(b"same\n"),
            after: Some(b"same\n"),
        })
        .unwrap();
        let rows = log
            .query_for_path(Path::new("/m/MEMORY.md"), &ChangeQuery::default())
            .unwrap();
        assert_eq!(rows[0].diff_omit_reason, Some(DiffOmitReason::Endpoint));
        assert_eq!(rows[0].hash_before, rows[0].hash_after);
    }

    #[test]
    fn per_file_eviction_keeps_only_cap_rows() {
        let (log, _t) = open_log();
        // Insert PER_FILE_RING_CAP + 5 rows for the same path. Verify
        // the oldest 5 are gone.
        for i in 0..(PER_FILE_RING_CAP + 5) {
            log.record(&RecordInput {
                project_slug: Some("p"),
                abs_path: Path::new("/m/f.md"),
                role: MemoryFileRole::AutoMemoryTopic,
                change_type: ChangeType::Modified,
                mtime_ns: i as i64,
                before: Some(b"x"),
                after: Some(format!("x-{}", i).as_bytes()),
            })
            .unwrap();
        }
        let rows = log
            .query_for_path(
                Path::new("/m/f.md"),
                &ChangeQuery {
                    limit: Some(10_000),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(rows.len(), PER_FILE_RING_CAP);
        // Newest row carries the highest mtime_ns we wrote.
        assert_eq!(rows[0].mtime_ns, (PER_FILE_RING_CAP + 4) as i64);
    }

    #[test]
    fn project_file_stats_aggregates_per_path() {
        let (log, _t) = open_log();
        for _ in 0..3 {
            log.record(&RecordInput {
                project_slug: Some("alpha"),
                abs_path: Path::new("/m/a.md"),
                role: MemoryFileRole::AutoMemoryTopic,
                change_type: ChangeType::Modified,
                mtime_ns: 0,
                before: Some(b"x"),
                after: Some(b"y"),
            })
            .unwrap();
        }
        for _ in 0..2 {
            log.record(&RecordInput {
                project_slug: Some("alpha"),
                abs_path: Path::new("/m/b.md"),
                role: MemoryFileRole::AutoMemoryTopic,
                change_type: ChangeType::Modified,
                mtime_ns: 0,
                before: Some(b"x"),
                after: Some(b"y"),
            })
            .unwrap();
        }
        let stats = log.project_file_stats("alpha").unwrap();
        let by_path: std::collections::HashMap<_, _> =
            stats.iter().map(|s| (s.abs_path.clone(), s)).collect();
        assert_eq!(
            by_path.get(Path::new("/m/a.md")).unwrap().change_count_30d,
            3
        );
        assert_eq!(
            by_path.get(Path::new("/m/b.md")).unwrap().change_count_30d,
            2
        );
    }

    #[test]
    fn latest_for_path_returns_most_recent_row() {
        let (log, _t) = open_log();
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/x.md"),
            role: MemoryFileRole::AutoMemoryTopic,
            change_type: ChangeType::Created,
            mtime_ns: 1,
            before: None,
            after: Some(b"first"),
        })
        .unwrap();
        log.record(&RecordInput {
            project_slug: None,
            abs_path: Path::new("/m/x.md"),
            role: MemoryFileRole::AutoMemoryTopic,
            change_type: ChangeType::Modified,
            mtime_ns: 2,
            before: Some(b"first"),
            after: Some(b"second"),
        })
        .unwrap();
        let latest = log
            .latest_for_path(Path::new("/m/x.md"))
            .unwrap()
            .expect("row");
        assert_eq!(latest.change_type, ChangeType::Modified);
        assert_eq!(latest.mtime_ns, 2);
    }

    #[test]
    fn open_quarantines_corrupt_db_and_creates_fresh() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("memory_changes.db");
        // Write garbage to the path so SQLite reports SQLITE_NOTADB.
        fs::write(&path, b"not a sqlite database, just some bytes").unwrap();
        let log = MemoryLog::open(&path).unwrap();
        assert_eq!(log.row_count().unwrap(), 0);
        // The corrupt file should have been moved aside.
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.iter().any(|n| n.contains(".corrupt-")),
            "expected a quarantined file in {:?}",
            entries
        );
    }
}
