//! Durable metrics store for the Activity Trends view.
//!
//! SQLite file at `<claudepot_data_dir>/activity_metrics.db`, separate
//! from `sessions.db` so the two concerns don't share a schema.
//! Single-writer from the runtime; WAL mode so any readers (Trends
//! view queries) don't block.
//!
//! Schema is deliberately skinny — one row per session per tick,
//! keyed by `(session_id, ts_ms)`. Status + errored + stuck + token
//! counters (M4+) are all we need for the current Trends view; we
//! deliberately don't store current_action / cwd / model snapshots
//! per tick — those belong in the live aggregate, not the long-term
//! series.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use thiserror::Error;

use crate::paths;
use crate::session_live::types::{LiveSessionSummary, Status};

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS metrics_tick (\
    session_id TEXT NOT NULL,\
    ts_ms      INTEGER NOT NULL,\
    status     TEXT NOT NULL,\
    errored    INTEGER NOT NULL DEFAULT 0,\
    stuck      INTEGER NOT NULL DEFAULT 0,\
    PRIMARY KEY (session_id, ts_ms)\
);\
CREATE INDEX IF NOT EXISTS idx_metrics_ts ON metrics_tick(ts_ms);\
";

/// Handle to the metrics SQLite. Cheap to clone — all state lives
/// behind `Arc<Mutex<Connection>>`.
pub struct MetricsStore {
    conn: Mutex<Connection>,
}

impl MetricsStore {
    /// Open the default store at `~/.claudepot/activity_metrics.db`.
    pub fn open_default() -> Result<Self, MetricsError> {
        let path = default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::open(&path)
    }

    /// Open a store at the given path. Used by tests against tempdir.
    pub fn open(path: &Path) -> Result<Self, MetricsError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(md) = std::fs::metadata(path) {
                let mut p = md.permissions();
                p.set_mode(0o600);
                let _ = std::fs::set_permissions(path, p);
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Write a batch of ticks for the current aggregate. Called once
    /// per runtime tick. Idempotent per `(session_id, ts_ms)` —
    /// duplicate rows are replaced (REPLACE semantics).
    pub fn record_tick(
        &self,
        ts_ms: i64,
        sessions: &[LiveSessionSummary],
    ) -> Result<(), MetricsError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| MetricsError::Sqlite(rusqlite::Error::InvalidQuery))?;
        let tx_guard = conn.unchecked_transaction()?;
        for s in sessions {
            let status_s = match s.status {
                Status::Busy => "busy",
                Status::Idle => "idle",
                Status::Waiting => "waiting",
            };
            conn.execute(
                "INSERT OR REPLACE INTO metrics_tick \
                 (session_id, ts_ms, status, errored, stuck) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    s.session_id,
                    ts_ms,
                    status_s,
                    s.errored as i32,
                    s.stuck as i32,
                ],
            )?;
        }
        tx_guard.commit()?;
        Ok(())
    }

    /// Prune rows older than the given cutoff. Called sparingly
    /// (once a day on startup) — keeps the DB bounded so a month of
    /// constant use doesn't balloon the file.
    pub fn prune_before(&self, cutoff_ms: i64) -> Result<usize, MetricsError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| MetricsError::Sqlite(rusqlite::Error::InvalidQuery))?;
        let n = conn.execute(
            "DELETE FROM metrics_tick WHERE ts_ms < ?1",
            rusqlite::params![cutoff_ms],
        )?;
        Ok(n)
    }

    /// Aggregate ticks in `[from_ms, to_ms)` into `bucket_count`
    /// equal-width time buckets. Each bucket's value is the DISTINCT
    /// count of sessions seen live during that bucket — effectively
    /// "how many sessions were alive during this time slice."
    ///
    /// Returns a Vec<u64> of length `bucket_count`. Empty buckets
    /// yield 0.
    pub fn active_series(
        &self,
        from_ms: i64,
        to_ms: i64,
        bucket_count: usize,
    ) -> Result<Vec<u64>, MetricsError> {
        if bucket_count == 0 || to_ms <= from_ms {
            return Ok(Vec::new());
        }
        let total = (to_ms - from_ms).max(1) as f64;
        let bucket_width = total / bucket_count as f64;
        let conn = self
            .conn
            .lock()
            .map_err(|_| MetricsError::Sqlite(rusqlite::Error::InvalidQuery))?;
        let mut stmt = conn.prepare(
            "SELECT ts_ms, session_id FROM metrics_tick \
             WHERE ts_ms >= ?1 AND ts_ms < ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![from_ms, to_ms])?;

        // For each bucket, collect a HashSet of session ids seen in
        // it. Using Vec<HashSet<String>> is O(N) memory-wise; in
        // practice a bucket has tens of sessions at most, so it
        // holds up fine.
        let mut buckets: Vec<std::collections::HashSet<String>> = (0..bucket_count)
            .map(|_| std::collections::HashSet::new())
            .collect();
        while let Some(row) = rows.next()? {
            let ts_ms: i64 = row.get(0)?;
            let sid: String = row.get(1)?;
            let offset = (ts_ms - from_ms) as f64;
            let idx = ((offset / bucket_width) as usize).min(bucket_count - 1);
            buckets[idx].insert(sid);
        }
        Ok(buckets.into_iter().map(|b| b.len() as u64).collect())
    }

    /// Count the number of ticks in `[from_ms, to_ms)` carrying the
    /// `errored` overlay. Used for the error-burst sparkline and the
    /// headline "errors today" stat.
    pub fn error_count(&self, from_ms: i64, to_ms: i64) -> Result<u64, MetricsError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| MetricsError::Sqlite(rusqlite::Error::InvalidQuery))?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM metrics_tick \
             WHERE ts_ms >= ?1 AND ts_ms < ?2 AND errored = 1",
            rusqlite::params![from_ms, to_ms],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }
}

pub fn default_path() -> PathBuf {
    paths::claudepot_data_dir().join("activity_metrics.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn session(sid: &str, status: Status, errored: bool, stuck: bool) -> LiveSessionSummary {
        LiveSessionSummary {
            session_id: sid.to_string(),
            pid: 1,
            cwd: "/tmp/proj".to_string(),
            transcript_path: None,
            status,
            current_action: None,
            model: None,
            waiting_for: None,
            errored,
            stuck,
            idle_ms: 0,
            seq: 0,
        }
    }

    fn open_fresh() -> (TempDir, MetricsStore) {
        let td = TempDir::new().unwrap();
        let p = td.path().join("m.db");
        let store = MetricsStore::open(&p).unwrap();
        (td, store)
    }

    #[test]
    fn record_then_active_series_single_bucket() {
        let (_td, s) = open_fresh();
        s.record_tick(1_000, &[session("a", Status::Busy, false, false)])
            .unwrap();
        let series = s.active_series(0, 2_000, 1).unwrap();
        assert_eq!(series, vec![1]);
    }

    #[test]
    fn active_series_distinct_by_session_id_within_bucket() {
        // Same session, two ticks, same bucket → counted once.
        let (_td, s) = open_fresh();
        s.record_tick(1_000, &[session("a", Status::Busy, false, false)])
            .unwrap();
        s.record_tick(1_500, &[session("a", Status::Idle, false, false)])
            .unwrap();
        assert_eq!(s.active_series(0, 2_000, 1).unwrap(), vec![1]);
    }

    #[test]
    fn active_series_splits_by_bucket() {
        let (_td, s) = open_fresh();
        s.record_tick(100, &[session("a", Status::Busy, false, false)])
            .unwrap();
        s.record_tick(1_100, &[session("b", Status::Busy, false, false)])
            .unwrap();
        s.record_tick(2_100, &[session("c", Status::Busy, false, false)])
            .unwrap();
        let series = s.active_series(0, 3_000, 3).unwrap();
        assert_eq!(series, vec![1, 1, 1]);
    }

    #[test]
    fn active_series_empty_range_yields_empty_vec() {
        let (_td, s) = open_fresh();
        assert!(s.active_series(1_000, 1_000, 5).unwrap().is_empty());
    }

    #[test]
    fn active_series_zero_buckets_yields_empty_vec() {
        let (_td, s) = open_fresh();
        s.record_tick(500, &[session("a", Status::Busy, false, false)])
            .unwrap();
        assert!(s.active_series(0, 1_000, 0).unwrap().is_empty());
    }

    #[test]
    fn error_count_filters_on_flag() {
        let (_td, s) = open_fresh();
        s.record_tick(
            100,
            &[
                session("a", Status::Busy, true, false),
                session("b", Status::Busy, false, false),
            ],
        )
        .unwrap();
        assert_eq!(s.error_count(0, 1_000).unwrap(), 1);
        assert_eq!(s.error_count(0, 0).unwrap(), 0);
    }

    #[test]
    fn prune_removes_old_rows() {
        let (_td, s) = open_fresh();
        s.record_tick(100, &[session("a", Status::Busy, false, false)])
            .unwrap();
        s.record_tick(5_000, &[session("a", Status::Busy, false, false)])
            .unwrap();
        let removed = s.prune_before(1_000).unwrap();
        assert_eq!(removed, 1);
        let series = s.active_series(0, 10_000, 1).unwrap();
        assert_eq!(series, vec![1]);
    }

    #[test]
    fn record_same_ts_is_idempotent() {
        // Re-record the same (session_id, ts_ms) — must not grow.
        let (_td, s) = open_fresh();
        s.record_tick(100, &[session("a", Status::Busy, false, false)])
            .unwrap();
        s.record_tick(100, &[session("a", Status::Busy, false, false)])
            .unwrap();
        assert_eq!(s.active_series(0, 1_000, 1).unwrap(), vec![1]);
    }
}
