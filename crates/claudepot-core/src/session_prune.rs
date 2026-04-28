//! Bulk delete session transcripts, reversible via trash.
//!
//! Two-phase API: `plan_prune()` is a pure scan that produces a
//! `PrunePlan`; `execute_prune()` consumes the plan and actually moves
//! each file into the trash. The split lets the CLI preview (`--dry-run`,
//! the default) and the GUI present a confirmation before anything
//! touches disk.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::project_progress::{PhaseStatus, ProgressSink};
use crate::session::{SessionError, SessionRow};
use crate::trash::{self, TrashError, TrashKind, TrashPut};

#[derive(Debug, Error)]
pub enum PruneError {
    #[error("empty filter — at least one criterion must be set")]
    EmptyFilter,
    #[error("listing sessions failed: {0}")]
    List(#[from] SessionError),
    #[error("trash op failed: {0}")]
    Trash(#[from] TrashError),
}

/// Filter clauses AND-composed. Every non-empty field narrows.
#[derive(Debug, Clone, Default)]
pub struct PruneFilter {
    /// Match sessions whose `last_ts` is older than `now - older_than`.
    pub older_than: Option<Duration>,
    /// Match sessions whose `file_size_bytes` is `>= threshold`.
    pub larger_than: Option<u64>,
    /// If non-empty, session's `project_path` must equal one of these.
    pub project: Vec<PathBuf>,
    /// `Some(true)` → only errored; `Some(false)` → only clean; `None` → any.
    pub has_error: Option<bool>,
    /// `Some(true)` → only sidechains; `Some(false)` → only main; `None` → any.
    pub is_sidechain: Option<bool>,
}

impl PruneFilter {
    /// Reject an entirely empty filter — the user almost certainly
    /// didn't mean "prune everything". Zero-valued `older_than`
    /// (0 seconds) and `larger_than` (0 bytes) count as "not set",
    /// because they would match every session and silently defeat the
    /// guard.
    pub fn validate(&self) -> Result<(), PruneError> {
        let has_older = matches!(self.older_than, Some(d) if !d.is_zero());
        let has_larger = matches!(self.larger_than, Some(n) if n > 0);
        let any = has_older
            || has_larger
            || !self.project.is_empty()
            || self.has_error.is_some()
            || self.is_sidechain.is_some();
        if any {
            Ok(())
        } else {
            Err(PruneError::EmptyFilter)
        }
    }

    /// Does the row match?  All non-empty clauses must pass.
    pub fn matches(&self, row: &SessionRow, now_ms: i64) -> bool {
        if let Some(max_age) = self.older_than {
            let cut = now_ms.saturating_sub(max_age.as_millis() as i64);
            // Prefer the in-transcript `last_ts` (event timestamp),
            // but fall back to the file's mtime when no parseable
            // timestamp survived. Without the fallback, `last_ts: None`
            // collapses to `0` (the Unix epoch) and any "older than X"
            // filter sweeps the row up — even when the file was
            // written seconds ago. If neither timestamp is available,
            // refuse to classify as old (skip the row).
            let last = match row.last_ts.map(|t| t.timestamp_millis()).or_else(|| {
                row.last_modified
                    .and_then(|st| st.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| i64::try_from(d.as_millis()).ok())
            }) {
                Some(ms) => ms,
                None => return false,
            };
            if last >= cut {
                return false;
            }
        }
        if let Some(thresh) = self.larger_than {
            if row.file_size_bytes < thresh {
                return false;
            }
        }
        if !self.project.is_empty() {
            let hit = self
                .project
                .iter()
                .any(|p| p.to_string_lossy() == row.project_path);
            if !hit {
                return false;
            }
        }
        if let Some(want) = self.has_error {
            if row.has_error != want {
                return false;
            }
        }
        if let Some(want) = self.is_sidechain {
            if row.is_sidechain != want {
                return false;
            }
        }
        true
    }
}

/// One row from the plan.
#[derive(Debug, Clone, Serialize)]
pub struct PruneEntry {
    pub session_id: String,
    pub file_path: PathBuf,
    pub project_path: String,
    pub size_bytes: u64,
    pub last_ts_ms: Option<i64>,
    pub has_error: bool,
    pub is_sidechain: bool,
}

impl PruneEntry {
    fn from_row(row: &SessionRow) -> Self {
        Self {
            session_id: row.session_id.clone(),
            file_path: row.file_path.clone(),
            project_path: row.project_path.clone(),
            size_bytes: row.file_size_bytes,
            last_ts_ms: row.last_ts.map(|t| t.timestamp_millis()),
            has_error: row.has_error,
            is_sidechain: row.is_sidechain,
        }
    }
}

/// Result of `plan_prune`.
#[derive(Debug, Clone, Serialize)]
pub struct PrunePlan {
    pub entries: Vec<PruneEntry>,
    pub total_bytes: u64,
}

/// Summary emitted by `execute_prune`.
#[derive(Debug, Clone, Serialize)]
pub struct PruneReport {
    pub moved: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub freed_bytes: u64,
}

/// Pure plan: filter + sort + sum, given an already-scanned row list
/// and a fixed `now_ms`. Test-friendly entry point that doesn't touch
/// the persistent index.
pub fn plan_from_rows(
    rows: &[SessionRow],
    filter: &PruneFilter,
    now_ms: i64,
) -> Result<PrunePlan, PruneError> {
    filter.validate()?;
    let mut entries: Vec<PruneEntry> = rows
        .iter()
        .filter(|r| filter.matches(r, now_ms))
        .map(PruneEntry::from_row)
        .collect();
    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    let total_bytes = entries.iter().map(|e| e.size_bytes).sum();
    Ok(PrunePlan {
        entries,
        total_bytes,
    })
}

/// Build a plan without touching disk state (beyond reading the index).
/// Rows are sorted **size-desc** so the user sees the biggest reclaim
/// targets first.
pub fn plan_prune(config_dir: &Path, filter: &PruneFilter) -> Result<PrunePlan, PruneError> {
    let rows = crate::session::list_all_sessions(config_dir)?;
    plan_from_rows(&rows, filter, chrono::Utc::now().timestamp_millis())
}

/// Execute a plan. Moves each file into the trash; records per-file
/// failures so one locked file doesn't block the rest. Emits
/// `plan-validated → moving → complete` phase events.
pub fn execute_prune(
    data_dir: &Path,
    plan: &PrunePlan,
    sink: &dyn ProgressSink,
) -> Result<PruneReport, PruneError> {
    sink.phase("plan-validated", PhaseStatus::Complete);
    let total = plan.entries.len();
    let mut moved: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();
    let mut freed: u64 = 0;
    for (i, e) in plan.entries.iter().enumerate() {
        sink.sub_progress("moving", i, total);
        let put = TrashPut {
            orig_path: &e.file_path,
            restore_path: None,
            kind: TrashKind::Prune,
            cwd: Some(Path::new(&e.project_path)),
            reason: Some(format!("prune session {}", e.session_id)),
        };
        match trash::write(data_dir, put) {
            Ok(entry) => {
                freed = freed.saturating_add(entry.size);
                moved.push(e.file_path.clone());
            }
            Err(err) => {
                failed.push((e.file_path.clone(), err.to_string()));
            }
        }
    }
    sink.sub_progress("moving", total, total);
    sink.phase("complete", PhaseStatus::Complete);
    Ok(PruneReport {
        moved,
        failed,
        freed_bytes: freed,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_progress::NoopSink;
    use crate::session::TokenUsage;
    use chrono::{Duration as ChronoDuration, TimeZone, Utc};
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn mk_row_on_disk(
        tmp: &Path,
        id: &str,
        size: usize,
        last_ts_offset_sec: i64,
        has_error: bool,
        is_sidechain: bool,
    ) -> SessionRow {
        let slug = format!("-p{id}");
        let dir = tmp.join("projects").join(&slug);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}-uuid.jsonl"));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(&vec![b'a'; size]).unwrap();
        let now = Utc::now();
        SessionRow {
            session_id: format!("{id}-uuid"),
            slug,
            file_path: path,
            file_size_bytes: size as u64,
            last_modified: Some(std::time::SystemTime::now()),
            project_path: format!("/repo/p{id}"),
            project_from_transcript: true,
            first_ts: None,
            last_ts: Some(now - ChronoDuration::seconds(last_ts_offset_sec)),
            event_count: 1,
            message_count: 1,
            user_message_count: 1,
            assistant_message_count: 0,
            first_user_prompt: None,
            models: vec![],
            tokens: TokenUsage::default(),
            git_branch: None,
            cc_version: None,
            display_slug: None,
            has_error,
            is_sidechain,
        }
    }

    fn now_ms() -> i64 {
        Utc::now().timestamp_millis()
    }

    #[test]
    fn empty_filter_rejected() {
        let f = PruneFilter::default();
        assert!(matches!(f.validate(), Err(PruneError::EmptyFilter)));
    }

    #[test]
    fn zero_valued_older_than_and_larger_than_count_as_empty() {
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(0)),
            larger_than: Some(0),
            ..PruneFilter::default()
        };
        assert!(
            matches!(f.validate(), Err(PruneError::EmptyFilter)),
            "zero-valued numeric filters must be treated as empty"
        );
    }

    #[test]
    fn older_than_matches_only_old_rows() {
        let tmp = TempDir::new().unwrap();
        let old = mk_row_on_disk(tmp.path(), "a", 10, 7200, false, false); // 2h ago
        let new = mk_row_on_disk(tmp.path(), "b", 10, 60, false, false); // 1 min ago
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(3600)), // 1h
            ..PruneFilter::default()
        };
        assert!(f.matches(&old, now_ms()));
        assert!(!f.matches(&new, now_ms()));
    }

    #[test]
    fn larger_than_matches_only_big_rows() {
        let tmp = TempDir::new().unwrap();
        let small = mk_row_on_disk(tmp.path(), "s", 100, 60, false, false);
        let big = mk_row_on_disk(tmp.path(), "b", 1000, 60, false, false);
        let f = PruneFilter {
            larger_than: Some(500),
            ..PruneFilter::default()
        };
        assert!(!f.matches(&small, now_ms()));
        assert!(f.matches(&big, now_ms()));
    }

    #[test]
    fn project_list_ands_with_other_filters() {
        let tmp = TempDir::new().unwrap();
        let a = mk_row_on_disk(tmp.path(), "a", 1000, 7200, false, false);
        let b = mk_row_on_disk(tmp.path(), "b", 1000, 7200, false, false);
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(3600)),
            project: vec![PathBuf::from("/repo/pa")],
            ..PruneFilter::default()
        };
        assert!(f.matches(&a, now_ms()));
        assert!(!f.matches(&b, now_ms()));
    }

    #[test]
    fn has_error_and_is_sidechain_clauses_each_work() {
        let tmp = TempDir::new().unwrap();
        let clean_main = mk_row_on_disk(tmp.path(), "a", 10, 60, false, false);
        let err_side = mk_row_on_disk(tmp.path(), "b", 10, 60, true, true);
        let err_only = PruneFilter {
            has_error: Some(true),
            ..PruneFilter::default()
        };
        assert!(!err_only.matches(&clean_main, now_ms()));
        assert!(err_only.matches(&err_side, now_ms()));
        let side_only = PruneFilter {
            is_sidechain: Some(true),
            ..PruneFilter::default()
        };
        assert!(!side_only.matches(&clean_main, now_ms()));
        assert!(side_only.matches(&err_side, now_ms()));
    }

    #[test]
    fn older_than_falls_back_to_file_mtime_when_last_ts_missing() {
        // Regression: a row with no parseable `last_ts` must NOT be
        // treated as ancient. When the file mtime is recent, the
        // older-than filter should leave it alone.
        let tmp = TempDir::new().unwrap();
        let mut row = mk_row_on_disk(tmp.path(), "fresh", 10, 60, false, false);
        row.last_ts = None;
        // mtime is "just now" via mk_row_on_disk's default. A 1-hour
        // older-than filter must miss this row.
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(3600)),
            ..PruneFilter::default()
        };
        assert!(
            !f.matches(&row, now_ms()),
            "untimestamped session with recent mtime must not be pruned"
        );
    }

    #[test]
    fn older_than_uses_mtime_for_old_untimestamped_rows() {
        // Counterpart to the regression test: a row with no `last_ts`
        // but an old mtime should still be reachable by older-than.
        let tmp = TempDir::new().unwrap();
        let mut row = mk_row_on_disk(tmp.path(), "stale", 10, 60, false, false);
        row.last_ts = None;
        row.last_modified =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(7200));
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(3600)),
            ..PruneFilter::default()
        };
        assert!(
            f.matches(&row, now_ms()),
            "untimestamped session with old mtime should be eligible"
        );
    }

    #[test]
    fn older_than_skips_rows_with_no_timestamps_at_all() {
        // Belt-and-suspenders: when neither `last_ts` nor
        // `last_modified` is available, refuse to classify the row
        // as old. Anything else risks deleting live data.
        let tmp = TempDir::new().unwrap();
        let mut row = mk_row_on_disk(tmp.path(), "blind", 10, 60, false, false);
        row.last_ts = None;
        row.last_modified = None;
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(3600)),
            ..PruneFilter::default()
        };
        assert!(
            !f.matches(&row, now_ms()),
            "row with no timestamps must not be eligible for older-than prune"
        );
    }

    #[test]
    fn older_than_tz_safe_against_utc() {
        // Build a row last-touched at an explicit UTC moment and show
        // the cutoff math treats everything in UTC ms.
        let tmp = TempDir::new().unwrap();
        let mut r = mk_row_on_disk(tmp.path(), "t", 10, 60, false, false);
        r.last_ts = Some(Utc.with_ymd_and_hms(2026, 4, 22, 10, 0, 0).unwrap());
        let cutoff = Utc.with_ymd_and_hms(2026, 4, 22, 12, 0, 0).unwrap();
        let f = PruneFilter {
            older_than: Some(Duration::from_secs(3600)),
            ..PruneFilter::default()
        };
        assert!(f.matches(&r, cutoff.timestamp_millis()));
    }

    #[test]
    fn plan_from_rows_sorts_by_size_desc_and_sums_bytes() {
        let tmp = TempDir::new().unwrap();
        let r_a = mk_row_on_disk(tmp.path(), "a", 500, 7200, false, false);
        let r_b = mk_row_on_disk(tmp.path(), "b", 1500, 7200, false, false);
        let r_c = mk_row_on_disk(tmp.path(), "c", 100, 60, false, false);
        let rows = vec![r_a, r_b, r_c];
        let plan = plan_from_rows(
            &rows,
            &PruneFilter {
                older_than: Some(Duration::from_secs(3600)),
                ..PruneFilter::default()
            },
            now_ms(),
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.entries[0].size_bytes, 1500);
        assert_eq!(plan.entries[1].size_bytes, 500);
        assert_eq!(plan.total_bytes, 2000);
    }

    #[test]
    fn plan_from_rows_has_no_side_effects() {
        let tmp = TempDir::new().unwrap();
        let r = mk_row_on_disk(tmp.path(), "a", 500, 7200, false, false);
        let path = r.file_path.clone();
        let _ = plan_from_rows(
            std::slice::from_ref(&r),
            &PruneFilter {
                older_than: Some(Duration::from_secs(3600)),
                ..PruneFilter::default()
            },
            now_ms(),
        )
        .unwrap();
        // The file must still be where we left it.
        assert!(path.exists());
    }

    #[test]
    fn execute_prune_moves_files_to_trash_and_reports_bytes_freed() {
        let tmp = TempDir::new().unwrap();
        let r = mk_row_on_disk(tmp.path(), "a", 123, 7200, false, false);
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let plan = PrunePlan {
            total_bytes: r.file_size_bytes,
            entries: vec![PruneEntry::from_row(&r)],
        };
        let report = execute_prune(&data_dir, &plan, &NoopSink).unwrap();
        assert_eq!(report.moved.len(), 1);
        assert!(report.failed.is_empty());
        assert_eq!(report.freed_bytes, 123);
        assert!(!r.file_path.exists(), "file should be moved out");
        let listing = trash::list(&data_dir, Default::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
    }

    #[test]
    fn execute_prune_partial_failure_is_per_file() {
        let tmp = TempDir::new().unwrap();
        let r = mk_row_on_disk(tmp.path(), "a", 10, 7200, false, false);
        let missing = PruneEntry {
            session_id: "ghost".into(),
            file_path: tmp.path().join("not-there.jsonl"),
            project_path: "/repo/ghost".into(),
            size_bytes: 0,
            last_ts_ms: None,
            has_error: false,
            is_sidechain: false,
        };
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let plan = PrunePlan {
            total_bytes: r.file_size_bytes,
            entries: vec![missing.clone(), PruneEntry::from_row(&r)],
        };
        let report = execute_prune(&data_dir, &plan, &NoopSink).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, missing.file_path);
        assert_eq!(report.moved.len(), 1);
    }
}
