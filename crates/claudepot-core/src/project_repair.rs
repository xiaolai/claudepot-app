//! Repair orchestration for project rename journals.
//!
//! The CLI and GUI both call into these functions; the only things
//! that stay CLI-side are user-confirmation prompts, pretty printing,
//! and JSON output shaping. No I/O formatting happens here.
//!
//! Spec references:
//! - §5.1 lock break + audit
//! - §6 journal states (running / pending / stale / abandoned)
//! - §6 resume/rollback/abandon semantics
//! - §7 GC retention

use crate::error::ProjectError;
use crate::project::{self, MoveArgs, MoveResult};
use crate::project_journal::{self, Journal, JournalStatus};
use crate::project_lock::{self, Lock};
use crate::project_progress::ProgressSink;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A journal file plus its classified status. The CLI and GUI both
/// iterate over these rather than the raw `(PathBuf, Journal)` tuples.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    /// Stable identifier = the journal file stem (e.g. `move-1744800000-12345`).
    pub id: String,
    pub path: PathBuf,
    pub journal: Journal,
    pub status: JournalStatus,
}

impl JournalEntry {
    fn stem(path: &Path) -> String {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}

/// Returned by [`gc`]. In `dry_run=true` mode, `entries_removed` is
/// the set of paths that *would* be removed and the counters reflect
/// projected deletes; in `dry_run=false` mode the counters reflect
/// actually-removed files and `entries_removed` is empty.
#[derive(Debug, Clone, Default)]
pub struct GcResult {
    pub removed_journals: usize,
    pub removed_snapshots: usize,
    pub bytes_freed: u64,
    /// Only populated in dry-run mode.
    pub would_remove: Vec<PathBuf>,
}

/// Audit record written by [`break_lock_with_audit`].
#[derive(Debug, Clone)]
pub struct BrokenLock {
    pub prior: Lock,
    pub audit_path: PathBuf,
}

/// List every journal on disk with its current status. This includes
/// entries with an `.abandoned.json` sidecar — callers that only care
/// about actionable work should use [`list_actionable`].
pub fn list_pending_with_status(
    journals_dir: &Path,
    locks_dir: &Path,
    nag_threshold_secs: u64,
) -> Result<Vec<JournalEntry>, ProjectError> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let raw = project_journal::list_pending(journals_dir)?;
    let mut out = Vec::with_capacity(raw.len());
    for (path, journal) in raw {
        let lock_path = locks_dir.join(format!("{}.lock", journal.old_san));
        let lock_live = match project_lock::read_lock(&lock_path) {
            Ok(l) => project_lock::is_live(&l),
            Err(_) => false,
        };
        let status =
            project_journal::classify(&path, &journal, lock_live, now_unix, nag_threshold_secs);
        let id = JournalEntry::stem(&path);
        out.push(JournalEntry {
            id,
            path,
            journal,
            status,
        });
    }
    Ok(out)
}

/// Only journals the user should act on — excludes those already
/// marked `Abandoned` via a sidecar.
pub fn list_actionable(
    journals_dir: &Path,
    locks_dir: &Path,
    nag_threshold_secs: u64,
) -> Result<Vec<JournalEntry>, ProjectError> {
    Ok(list_pending_with_status(journals_dir, locks_dir, nag_threshold_secs)?
        .into_iter()
        .filter(|e| e.status != JournalStatus::Abandoned)
        .collect())
}

/// Re-run the original move. The original journal is marked abandoned
/// first so the pending-journal gate doesn't block the re-run. Phases
/// are idempotent (spec §6).
///
/// The returned `MoveResult` carries a fresh journal path (the
/// successor), not the old one.
pub fn resume(
    entry: &JournalEntry,
    config_dir: PathBuf,
    claude_json_path: Option<PathBuf>,
    snapshots_dir: Option<PathBuf>,
    sink: &dyn ProgressSink,
) -> Result<MoveResult, ProjectError> {
    // Supersede the prior journal (audit trail preserved; gate ignores it).
    let _ = project_journal::mark_abandoned(&entry.path);

    let args = MoveArgs {
        old_path: entry.journal.old_path.clone().into(),
        new_path: entry.journal.new_path.clone().into(),
        config_dir,
        claude_json_path,
        snapshots_dir,
        no_move: entry.journal.flags.no_move,
        merge: entry.journal.flags.merge,
        overwrite: entry.journal.flags.overwrite,
        force: entry.journal.flags.force,
        dry_run: false,
        ignore_pending_journals: true,
        claudepot_state_dir: None,
    };
    project::move_project(&args, sink)
}

/// Run the reverse move (new → old). Snapshots from destructive phases
/// are NOT auto-restored — callers surface `entry.journal.snapshot_paths`
/// for manual inspection.
pub fn rollback(
    entry: &JournalEntry,
    config_dir: PathBuf,
    claude_json_path: Option<PathBuf>,
    snapshots_dir: Option<PathBuf>,
    sink: &dyn ProgressSink,
) -> Result<MoveResult, ProjectError> {
    let _ = project_journal::mark_abandoned(&entry.path);

    let args = MoveArgs {
        old_path: entry.journal.new_path.clone().into(),
        new_path: entry.journal.old_path.clone().into(),
        config_dir,
        claude_json_path,
        snapshots_dir,
        no_move: entry.journal.flags.no_move,
        merge: entry.journal.flags.merge,
        overwrite: entry.journal.flags.overwrite,
        force: entry.journal.flags.force,
        dry_run: false,
        ignore_pending_journals: true,
        claudepot_state_dir: None,
    };
    project::move_project(&args, sink)
}

/// Write the `.abandoned.json` sidecar. The journal itself is kept
/// for audit.
pub fn abandon(entry: &JournalEntry) -> Result<PathBuf, ProjectError> {
    project_journal::mark_abandoned(&entry.path)
}

/// Force-break a lock file and write an audit record to the journals
/// directory. Callers handle user confirmation before calling.
pub fn break_lock_with_audit(
    lock_path: &Path,
    journals_dir: &Path,
) -> Result<BrokenLock, ProjectError> {
    let prior = project_lock::break_lock(lock_path)?;

    fs::create_dir_all(journals_dir).map_err(ProjectError::Io)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let audit_path = journals_dir.join(format!("broken-lock-{ts}.json"));
    let body = serde_json::json!({
        "broken_at": chrono::Utc::now().to_rfc3339(),
        "reason": "manual --break-lock",
        "prior_lock": &prior,
        "broken_by_pid": std::process::id(),
        "lock_path": lock_path,
    });
    fs::write(
        &audit_path,
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(ProjectError::Io)?;

    Ok(BrokenLock { prior, audit_path })
}

/// Per-entry detail in an [`AbandonedCleanupReport`]. One row per
/// `.abandoned.json` sidecar encountered — lets the UI show the
/// user exactly which artifacts will go / went away.
#[derive(Debug, Clone)]
pub struct AbandonedCleanupEntry {
    /// Journal stem (e.g. `move-1744800000-12345`).
    pub id: String,
    /// Absolute path of the `.json` journal file.
    pub journal_path: PathBuf,
    /// Absolute path of the `.abandoned.json` sidecar.
    pub sidecar_path: PathBuf,
    /// Every snapshot file referenced by the journal's
    /// `snapshot_paths` that currently exists on disk.
    pub referenced_snapshots: Vec<PathBuf>,
    /// Aggregate size of journal + sidecar + referenced snapshots,
    /// in bytes. Measured even in dry-run mode so the UI can preview
    /// the cost.
    pub bytes: u64,
}

/// Returned by [`preview_abandoned`] / [`cleanup_abandoned`].
///
/// In preview mode, `entries` lists every artifact that *would* be
/// removed and `removed_*` are zero. In cleanup mode, `entries` is
/// the set of artifacts that were actually removed and the counters
/// reflect true deletions (M12 honesty rule).
#[derive(Debug, Clone, Default)]
pub struct AbandonedCleanupReport {
    pub entries: Vec<AbandonedCleanupEntry>,
    pub removed_journals: usize,
    pub removed_snapshots: usize,
    pub bytes_freed: u64,
}

/// List every abandoned journal on disk with its referenced snapshot
/// paths. Does not modify anything. Used to populate the
/// "Clean recovery artifacts" preview.
pub fn preview_abandoned(
    journals_dir: &Path,
) -> Result<AbandonedCleanupReport, ProjectError> {
    let mut out = AbandonedCleanupReport::default();
    for entry in scan_abandoned_entries(journals_dir)? {
        out.bytes_freed += entry.bytes;
        out.entries.push(entry);
    }
    Ok(out)
}

/// Remove every abandoned journal + its sidecar + the snapshots it
/// references. Unlike [`gc`], this function does NOT sweep
/// unreferenced or age-old snapshots — they may belong to running
/// ops or to successful ops whose operator still wants the audit
/// artifact. The cascade is strictly journal → its own
/// `snapshot_paths`.
///
/// M12: counters increment only for files actually removed. A
/// filesystem race or permission denied leaves the counter at its
/// prior value and the paths stay out of the report.
pub fn cleanup_abandoned(
    journals_dir: &Path,
) -> Result<AbandonedCleanupReport, ProjectError> {
    let mut out = AbandonedCleanupReport::default();
    for entry in scan_abandoned_entries(journals_dir)? {
        // Remove referenced snapshots first so an orphaned snapshot
        // never outlives its journal. Non-existent paths are a no-op.
        let mut actually_removed_snaps = Vec::with_capacity(entry.referenced_snapshots.len());
        for snap in &entry.referenced_snapshots {
            let size = fs::metadata(snap).map(|m| m.len()).unwrap_or(0);
            match fs::remove_file(snap) {
                Ok(_) => {
                    out.bytes_freed += size;
                    out.removed_snapshots += 1;
                    actually_removed_snaps.push(snap.clone());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Already gone — record nothing.
                }
                Err(e) => {
                    tracing::warn!(
                        snap = ?snap,
                        error = %e,
                        "cleanup_abandoned: snapshot removal failed"
                    );
                }
            }
        }

        // Then the sidecar, then the journal. If sidecar removal
        // fails we don't remove the journal — leaving the sidecar in
        // place keeps list_actionable excluding the entry, so the
        // invariant "no sidecar → must be actionable" holds.
        let sidecar_size = fs::metadata(&entry.sidecar_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if fs::remove_file(&entry.sidecar_path).is_ok() {
            out.bytes_freed += sidecar_size;
            let journal_size = fs::metadata(&entry.journal_path)
                .map(|m| m.len())
                .unwrap_or(0);
            if fs::remove_file(&entry.journal_path).is_ok() {
                out.bytes_freed += journal_size;
                out.removed_journals += 1;
                out.entries.push(AbandonedCleanupEntry {
                    referenced_snapshots: actually_removed_snaps,
                    ..entry
                });
            }
        }
    }
    Ok(out)
}

/// Internal: enumerate every `.abandoned.json` sidecar + its twin
/// journal + the snapshot paths the journal references. Shared by
/// [`preview_abandoned`] and [`cleanup_abandoned`] so the two stay
/// byte-for-byte consistent — preview says exactly what cleanup
/// would do.
fn scan_abandoned_entries(
    journals_dir: &Path,
) -> Result<Vec<AbandonedCleanupEntry>, ProjectError> {
    if !journals_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(journals_dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".abandoned.json") {
            continue;
        }
        let base = name.trim_end_matches(".abandoned.json");
        let sidecar_path = entry.path();
        let journal_path = journals_dir.join(format!("{base}.json"));

        let mut bytes: u64 = 0;
        bytes += fs::metadata(&sidecar_path).map(|m| m.len()).unwrap_or(0);
        bytes += fs::metadata(&journal_path).map(|m| m.len()).unwrap_or(0);

        // The sidecar file stores an abandonment marker; the
        // snapshot_paths live in the original journal JSON.
        let mut referenced_snapshots: Vec<PathBuf> = Vec::new();
        if let Ok(body) = fs::read_to_string(&journal_path) {
            if let Ok(j) = serde_json::from_str::<Journal>(&body) {
                for snap in &j.snapshot_paths {
                    if snap.exists() {
                        bytes += fs::metadata(snap).map(|m| m.len()).unwrap_or(0);
                        referenced_snapshots.push(snap.clone());
                    }
                }
            }
        }

        out.push(AbandonedCleanupEntry {
            id: base.to_string(),
            journal_path,
            sidecar_path,
            referenced_snapshots,
            bytes,
        });
    }
    Ok(out)
}

/// Garbage-collect abandoned journals and snapshots older than
/// `older_than_days`. With `dry_run=true` nothing is deleted and
/// `would_remove` is populated with the candidate paths.
pub fn gc(
    journals_dir: &Path,
    snapshots_dir: &Path,
    older_than_days: u64,
    dry_run: bool,
) -> Result<GcResult, ProjectError> {
    let cutoff_secs = older_than_days.saturating_mul(86_400);
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut result = GcResult::default();

    // Abandoned journals (identified by `.abandoned.json` sidecar).
    if journals_dir.exists() {
        for entry in fs::read_dir(journals_dir).map_err(ProjectError::Io)? {
            let entry = entry.map_err(ProjectError::Io)?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".abandoned.json") {
                continue;
            }
            let base = name.trim_end_matches(".abandoned.json");
            let journal_path = journals_dir.join(format!("{base}.json"));
            let meta = entry.metadata().map_err(ProjectError::Io)?;
            let age = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| now_unix.saturating_sub(d.as_secs()))
                .unwrap_or(0);
            if age < cutoff_secs {
                continue;
            }
            if dry_run {
                result.would_remove.push(entry.path());
                if journal_path.exists() {
                    result.would_remove.push(journal_path);
                }
            } else {
                // Audit M12: only count files we actually removed. The
                // previous implementation incremented bytes_freed and
                // removed_journals unconditionally even when fs::remove_file
                // errored (permission denied, race). The CLI + GUI both
                // reported deletions that didn't happen.
                let sidecar_size = meta.len();
                if fs::remove_file(entry.path()).is_ok() {
                    result.bytes_freed += sidecar_size;
                    if journal_path.exists() {
                        let journal_size = fs::metadata(&journal_path)
                            .map(|m| m.len())
                            .unwrap_or(0);
                        if fs::remove_file(&journal_path).is_ok() {
                            result.bytes_freed += journal_size;
                        }
                    }
                    result.removed_journals += 1;
                }
            }
        }
    }

    // Snapshots older than cutoff.
    if snapshots_dir.exists() {
        for entry in fs::read_dir(snapshots_dir).map_err(ProjectError::Io)? {
            let entry = entry.map_err(ProjectError::Io)?;
            let meta = entry.metadata().map_err(ProjectError::Io)?;
            let age = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| now_unix.saturating_sub(d.as_secs()))
                .unwrap_or(0);
            if age < cutoff_secs {
                continue;
            }
            if dry_run {
                result.would_remove.push(entry.path());
            } else {
                // M12: same honesty requirement as above.
                let size = meta.len();
                if fs::remove_file(entry.path()).is_ok() {
                    result.bytes_freed += size;
                    result.removed_snapshots += 1;
                }
            }
        }
    }

    Ok(result)
}

/// Resolve a user-supplied project hint (sanitized or raw path) to a
/// lock file. Returns `None` if no lock exists under either form.
pub fn resolve_lock_file(locks_dir: &Path, project_hint: &str) -> Option<PathBuf> {
    let san = project::sanitize_path(project_hint);
    let primary = locks_dir.join(format!("{san}.lock"));
    if primary.exists() {
        return Some(primary);
    }
    let alt = locks_dir.join(format!("{project_hint}.lock"));
    if alt.exists() {
        return Some(alt);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_journal::JournalFlags;
    use std::time::Duration;
    use tempfile::TempDir;

    fn write_journal(dir: &Path, id: &str, started_unix: u64) -> PathBuf {
        let j = Journal {
            version: 1,
            started_at: "2026-04-15T00:00:00Z".to_string(),
            started_unix_secs: started_unix,
            pid: 12345,
            hostname: "test-host".to_string(),
            claudepot_version: "0.1.0".to_string(),
            old_path: "/tmp/old".to_string(),
            new_path: "/tmp/new".to_string(),
            old_san: "-tmp-old".to_string(),
            new_san: "-tmp-new".to_string(),
            old_git_root: None,
            new_git_root: None,
            flags: JournalFlags::default(),
            phases_completed: vec!["P3".to_string()],
            snapshot_paths: vec![],
            last_error: None,
        };
        let path = dir.join(format!("{id}.json"));
        fs::write(&path, serde_json::to_string_pretty(&j).unwrap()).unwrap();
        path
    }

    #[test]
    fn test_list_pending_with_status_classifies_pending_vs_stale() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&locks).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        write_journal(&journals, "move-fresh", now - 60);
        write_journal(&journals, "move-old", now - 3 * 86_400);

        let entries = list_pending_with_status(&journals, &locks, 86_400).unwrap();
        assert_eq!(entries.len(), 2);
        let fresh = entries.iter().find(|e| e.id == "move-fresh").unwrap();
        let old = entries.iter().find(|e| e.id == "move-old").unwrap();
        assert_eq!(fresh.status, JournalStatus::Pending);
        assert_eq!(old.status, JournalStatus::Stale);
    }

    #[test]
    fn test_list_actionable_excludes_abandoned() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&locks).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let a = write_journal(&journals, "move-a", now - 60);
        let _b = write_journal(&journals, "move-b", now - 60);
        project_journal::mark_abandoned(&a).unwrap();

        let actionable = list_actionable(&journals, &locks, 86_400).unwrap();
        assert_eq!(actionable.len(), 1);
        assert_eq!(actionable[0].id, "move-b");
    }

    #[test]
    fn test_abandon_writes_sidecar_and_preserves_journal() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&locks).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        write_journal(&journals, "move-x", now - 60);

        let entries = list_actionable(&journals, &locks, 86_400).unwrap();
        let sidecar = abandon(&entries[0]).unwrap();
        assert!(sidecar.exists());
        assert!(entries[0].path.exists(), "journal is preserved for audit");

        // Second pass sees no actionable entries.
        let after = list_actionable(&journals, &locks, 86_400).unwrap();
        assert!(after.is_empty());
    }

    #[test]
    fn test_break_lock_with_audit_writes_audit_record() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&locks).unwrap();

        // Fabricate a lock file directly.
        let lock_path = locks.join("-tmp-foo.lock");
        let lock = Lock {
            version: 1,
            pid: 99999,
            hostname: "ghost-host".to_string(),
            start_iso8601: "2026-04-15T00:00:00Z".to_string(),
            start_unix_secs: 0,
            claudepot_version: "0.1.0".to_string(),
        };
        fs::write(&lock_path, serde_json::to_string(&lock).unwrap()).unwrap();

        let broken = break_lock_with_audit(&lock_path, &journals).unwrap();
        assert!(!lock_path.exists(), "lock removed");
        assert!(broken.audit_path.exists(), "audit written");
        assert_eq!(broken.prior.pid, 99999);

        let audit = fs::read_to_string(&broken.audit_path).unwrap();
        assert!(audit.contains("\"reason\""));
        assert!(audit.contains("manual --break-lock"));
    }

    #[test]
    fn test_gc_dry_run_does_not_delete() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let snaps = tmp.path().join("snapshots");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&snaps).unwrap();

        // Seed an abandoned journal with an old mtime.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let journal_path = write_journal(&journals, "move-old", now - 100 * 86_400);
        project_journal::mark_abandoned(&journal_path).unwrap();
        // Backdate sidecar mtime by 100 days.
        let sidecar = journals.join("move-old.abandoned.json");
        let old_time =
            SystemTime::now().checked_sub(Duration::from_secs(100 * 86_400)).unwrap();
        filetime::set_file_mtime(&sidecar, old_time.into()).ok();
        filetime::set_file_mtime(&journal_path, old_time.into()).ok();

        let result = gc(&journals, &snaps, 30, /* dry_run */ true).unwrap();
        assert_eq!(result.removed_journals, 0);
        assert!(!result.would_remove.is_empty());
        assert!(sidecar.exists(), "dry-run preserves files");
        assert!(journal_path.exists());
    }

    #[test]
    fn test_gc_removes_abandoned_journals_older_than_cutoff() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let snaps = tmp.path().join("snapshots");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&snaps).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let journal_path = write_journal(&journals, "move-old", now - 100 * 86_400);
        project_journal::mark_abandoned(&journal_path).unwrap();
        let sidecar = journals.join("move-old.abandoned.json");
        let old_time =
            SystemTime::now().checked_sub(Duration::from_secs(100 * 86_400)).unwrap();
        filetime::set_file_mtime(&sidecar, old_time.into()).ok();
        filetime::set_file_mtime(&journal_path, old_time.into()).ok();

        let result = gc(&journals, &snaps, 30, /* dry_run */ false).unwrap();
        assert_eq!(result.removed_journals, 1);
        assert!(!sidecar.exists());
        assert!(!journal_path.exists());
        assert!(result.bytes_freed > 0);
    }

    #[test]
    fn test_gc_leaves_recent_journals_alone() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let snaps = tmp.path().join("snapshots");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&snaps).unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let journal_path = write_journal(&journals, "move-new", now - 60);
        project_journal::mark_abandoned(&journal_path).unwrap();

        let result = gc(&journals, &snaps, 30, /* dry_run */ false).unwrap();
        assert_eq!(result.removed_journals, 0);
        assert!(journal_path.exists());
    }

    #[test]
    fn test_resolve_lock_file_by_sanitized_path() {
        let tmp = TempDir::new().unwrap();
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&locks).unwrap();
        let san = project::sanitize_path("/tmp/my-project");
        let lock_path = locks.join(format!("{san}.lock"));
        fs::write(&lock_path, "{}").unwrap();

        let resolved = resolve_lock_file(&locks, "/tmp/my-project").unwrap();
        assert_eq!(resolved, lock_path);
    }

    #[test]
    fn test_resolve_lock_file_by_bare_sanitized_name() {
        let tmp = TempDir::new().unwrap();
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&locks).unwrap();
        let lock_path = locks.join("-tmp-bare.lock");
        fs::write(&lock_path, "{}").unwrap();

        let resolved = resolve_lock_file(&locks, "-tmp-bare").unwrap();
        assert_eq!(resolved, lock_path);
    }

    #[test]
    fn test_resolve_lock_file_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let locks = tmp.path().join("locks");
        fs::create_dir_all(&locks).unwrap();
        assert!(resolve_lock_file(&locks, "/nowhere").is_none());
    }

    // -----------------------------------------------------------------
    // cleanup_abandoned / preview_abandoned
    // -----------------------------------------------------------------

    fn write_journal_with_snapshots(
        journals_dir: &Path,
        id: &str,
        snapshots: &[PathBuf],
    ) -> PathBuf {
        let j = Journal {
            version: 1,
            started_at: "2026-04-15T00:00:00Z".to_string(),
            started_unix_secs: 1_700_000_000,
            pid: 12345,
            hostname: "test-host".to_string(),
            claudepot_version: "0.1.0".to_string(),
            old_path: "/tmp/old".to_string(),
            new_path: "/tmp/new".to_string(),
            old_san: "-tmp-old".to_string(),
            new_san: "-tmp-new".to_string(),
            old_git_root: None,
            new_git_root: None,
            flags: JournalFlags::default(),
            phases_completed: vec!["P3".to_string()],
            snapshot_paths: snapshots.to_vec(),
            last_error: None,
        };
        let path = journals_dir.join(format!("{id}.json"));
        fs::write(&path, serde_json::to_string_pretty(&j).unwrap()).unwrap();
        path
    }

    fn write_sidecar(journals_dir: &Path, id: &str) {
        fs::write(journals_dir.join(format!("{id}.abandoned.json")), "{}").unwrap();
    }

    #[test]
    fn preview_abandoned_reports_journal_sidecar_and_referenced_snapshots() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let snapshots = tmp.path().join("snapshots");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&snapshots).unwrap();

        let snap_a = snapshots.join("ts-1-P7.json");
        let snap_b = snapshots.join("ts-1-P8.json");
        fs::write(&snap_a, "a").unwrap(); // 1 byte
        fs::write(&snap_b, "bb").unwrap(); // 2 bytes

        let journal_path = write_journal_with_snapshots(
            &journals,
            "move-abandoned",
            &[snap_a.clone(), snap_b.clone()],
        );
        write_sidecar(&journals, "move-abandoned");

        let report = preview_abandoned(&journals).expect("preview");
        assert_eq!(report.entries.len(), 1);
        let entry = &report.entries[0];
        assert_eq!(entry.id, "move-abandoned");
        assert_eq!(entry.journal_path, journal_path);
        assert_eq!(entry.referenced_snapshots, vec![snap_a.clone(), snap_b.clone()]);
        assert!(entry.bytes >= 3, "bytes must account for snapshots at minimum");
        // Preview must NOT delete anything.
        assert!(snap_a.exists());
        assert!(snap_b.exists());
        assert!(journal_path.exists());
    }

    #[test]
    fn cleanup_abandoned_removes_journal_sidecar_and_referenced_snapshots() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let snapshots = tmp.path().join("snapshots");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&snapshots).unwrap();

        let snap = snapshots.join("ts-1-P7.json");
        fs::write(&snap, "bytes").unwrap();
        let journal_path =
            write_journal_with_snapshots(&journals, "move-abandoned", &[snap.clone()]);
        let sidecar_path = journals.join("move-abandoned.abandoned.json");
        write_sidecar(&journals, "move-abandoned");

        let report = cleanup_abandoned(&journals).expect("cleanup");
        assert_eq!(report.removed_journals, 1);
        assert_eq!(report.removed_snapshots, 1);
        assert_eq!(report.entries.len(), 1);
        assert!(report.bytes_freed >= 5);
        assert!(!journal_path.exists(), "journal must be removed");
        assert!(!sidecar_path.exists(), "sidecar must be removed");
        assert!(!snap.exists(), "referenced snapshot must be removed");
    }

    #[test]
    fn cleanup_abandoned_leaves_unreferenced_and_non_abandoned_artifacts_alone() {
        // This is the load-bearing safety test: gc(0, ...) would sweep
        // these too. cleanup_abandoned MUST NOT.
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        let snapshots = tmp.path().join("snapshots");
        fs::create_dir_all(&journals).unwrap();
        fs::create_dir_all(&snapshots).unwrap();

        // A live journal (no sidecar).
        let live_journal = write_journal_with_snapshots(&journals, "move-live", &[]);
        // A snapshot that isn't referenced by any journal — e.g. from
        // a successful rename. Must survive cleanup.
        let orphan_snap = snapshots.join("ts-99-P7.json");
        fs::write(&orphan_snap, "orphan").unwrap();
        // An abandoned journal with its own referenced snapshot.
        let referenced = snapshots.join("ts-1-P7.json");
        fs::write(&referenced, "x").unwrap();
        let abandoned_journal = write_journal_with_snapshots(
            &journals,
            "move-abandoned",
            &[referenced.clone()],
        );
        write_sidecar(&journals, "move-abandoned");

        let report = cleanup_abandoned(&journals).expect("cleanup");
        assert_eq!(report.removed_journals, 1);
        assert_eq!(report.removed_snapshots, 1);

        // Live journal untouched.
        assert!(live_journal.exists(), "live journal must survive");
        // Orphan snapshot untouched — this is the difference from gc.
        assert!(
            orphan_snap.exists(),
            "unreferenced snapshot must survive cleanup_abandoned"
        );
        // Abandoned artifacts gone.
        assert!(!abandoned_journal.exists());
        assert!(!referenced.exists());
    }

    #[test]
    fn cleanup_abandoned_returns_empty_when_no_sidecars_exist() {
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        fs::create_dir_all(&journals).unwrap();
        let _ = write_journal_with_snapshots(&journals, "move-live", &[]);

        let report = cleanup_abandoned(&journals).expect("cleanup");
        assert!(report.entries.is_empty());
        assert_eq!(report.removed_journals, 0);
        assert_eq!(report.removed_snapshots, 0);
    }

    #[test]
    fn cleanup_abandoned_tolerates_missing_snapshot_paths() {
        // If a snapshot listed in snapshot_paths was already removed
        // manually (user ran `rm`), cleanup_abandoned should still
        // succeed and remove the journal + sidecar without counting
        // the missing snapshot.
        let tmp = TempDir::new().unwrap();
        let journals = tmp.path().join("journals");
        fs::create_dir_all(&journals).unwrap();

        let phantom = tmp.path().join("this-was-deleted-out-of-band.json");
        let journal_path =
            write_journal_with_snapshots(&journals, "move-abandoned", &[phantom]);
        write_sidecar(&journals, "move-abandoned");

        let report = cleanup_abandoned(&journals).expect("cleanup");
        assert_eq!(report.removed_journals, 1);
        assert_eq!(report.removed_snapshots, 0);
        assert!(!journal_path.exists());
    }
}
