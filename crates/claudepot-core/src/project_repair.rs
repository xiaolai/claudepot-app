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
    Ok(
        list_pending_with_status(journals_dir, locks_dir, nag_threshold_secs)?
            .into_iter()
            .filter(|e| e.status != JournalStatus::Abandoned)
            .collect(),
    )
}

/// Look up a single pending journal by its stable id (the file stem,
/// e.g. `move-1744800000-12345`). Returns `Ok(None)` if no journal in
/// `journals_dir` carries that id. Same I/O as
/// [`list_pending_with_status`] — narrower contract.
pub fn find_pending_by_id(
    journals_dir: &Path,
    locks_dir: &Path,
    nag_threshold_secs: u64,
    id: &str,
) -> Result<Option<JournalEntry>, ProjectError> {
    Ok(
        list_pending_with_status(journals_dir, locks_dir, nag_threshold_secs)?
            .into_iter()
            .find(|e| e.id == id),
    )
}

/// Find the most recent pending journal whose `old_path` matches the
/// given value, picked by `started_unix_secs` (max wins). Used by the
/// IPC layer's failure-finalizer to deep-link the user back to the
/// exact journal that just failed. Returns `Ok(None)` when no journal
/// matches. Same I/O as [`list_pending_with_status`].
pub fn newest_pending_for_old_path(
    journals_dir: &Path,
    locks_dir: &Path,
    nag_threshold_secs: u64,
    old_path: &str,
) -> Result<Option<JournalEntry>, ProjectError> {
    Ok(
        list_pending_with_status(journals_dir, locks_dir, nag_threshold_secs)?
            .into_iter()
            .filter(|e| e.journal.old_path == old_path)
            .max_by_key(|e| e.journal.started_unix_secs),
    )
}

/// Re-run the original move. The original journal is marked abandoned
/// first so the pending-journal gate doesn't block the re-run. Phases
/// are idempotent (spec §6).
///
/// The returned `MoveResult` carries a fresh journal path (the
/// successor), not the old one.
///
/// `claudepot_state_dir` MUST be the same repair-tree root that owns
/// `entry.path` — otherwise the new journal/lock/snapshot tree splits
/// from the original and the post-failure audit trail goes stale.
/// Production callers pass `Some(paths::claudepot_repair_dir())`; the
/// resolver below derives the root from the journal's location as a
/// safety net for tests that pass `None`.
pub fn resume(
    entry: &JournalEntry,
    config_dir: PathBuf,
    claude_json_path: Option<PathBuf>,
    snapshots_dir: Option<PathBuf>,
    claudepot_state_dir: Option<PathBuf>,
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
        // Audit B3 fix: thread the explicit override OR fall back to
        // the journal's own repair tree (parent of `journals/`). The
        // legacy code hard-coded `None`, which forced new
        // locks/journals/snapshots into `<config_dir>/claudepot/`
        // instead of the real `~/.claudepot/repair/` tree, splitting
        // the audit trail.
        claudepot_state_dir: claudepot_state_dir.or_else(|| state_root_from_entry(entry)),
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
    claudepot_state_dir: Option<PathBuf>,
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
        // Audit B3 fix: see `resume` above — preserve the original
        // repair-tree root so the rollback's audit artifacts stay
        // alongside the failed forward move's.
        claudepot_state_dir: claudepot_state_dir.or_else(|| state_root_from_entry(entry)),
    };
    project::move_project(&args, sink)
}

/// Derive the repair-tree root from a journal entry's path. The journal
/// lives at `<state_root>/journals/<id>.json`, so the root is two
/// `parent()`s up. Returns `None` if the path is unexpected (e.g. a
/// shallow tmp path in a test); callers fall back to the legacy default.
fn state_root_from_entry(entry: &JournalEntry) -> Option<PathBuf> {
    entry
        .path
        .parent() // <state_root>/journals
        .and_then(|p| p.parent()) // <state_root>
        .map(|p| p.to_path_buf())
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
pub fn preview_abandoned(journals_dir: &Path) -> Result<AbandonedCleanupReport, ProjectError> {
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
pub fn cleanup_abandoned(journals_dir: &Path) -> Result<AbandonedCleanupReport, ProjectError> {
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
fn scan_abandoned_entries(journals_dir: &Path) -> Result<Vec<AbandonedCleanupEntry>, ProjectError> {
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
                        let journal_size =
                            fs::metadata(&journal_path).map(|m| m.len()).unwrap_or(0);
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
#[path = "project_repair_tests.rs"]
mod tests;
