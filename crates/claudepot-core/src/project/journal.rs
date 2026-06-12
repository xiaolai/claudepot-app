//! Project-rename journal for failure recovery (spec §6).
//!
//! A journal records in-flight rename state: which phases completed,
//! snapshot paths for destructive operations, and the originating PID
//! so `project repair` can decide finish-forward vs. rollback.
//!
//! Layout: `~/.claudepot/repair/journals/move-<secs>-<pid>-<suffix>.json`
//! (legacy location `~/.claude/claudepot/journals/…` is migrated in
//! place on first boot — see `migrations::migrate_repair_tree`).
//! The 6-char suffix disambiguates concurrent moves started by the
//! same process in the same wall-clock second (Tauri runs multiple
//! moves concurrently; locks are per-project, not global).
//! On successful completion the journal is deleted. Failed runs leave
//! the journal for user inspection.

use crate::error::ProjectError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Serialized on-disk form of a rename journal. Versioned so future
/// additions can migrate cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub started_at: String,
    pub started_unix_secs: u64,
    pub pid: u32,
    pub hostname: String,
    pub claudepot_version: String,
    pub old_path: String,
    pub new_path: String,
    pub old_san: String,
    pub new_san: String,
    pub old_git_root: Option<String>,
    pub new_git_root: Option<String>,
    pub flags: JournalFlags,
    pub phases_completed: Vec<String>,
    pub snapshot_paths: Vec<PathBuf>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JournalFlags {
    pub merge: bool,
    pub overwrite: bool,
    pub no_move: bool,
    pub force: bool,
    pub ignore_pending_journals: bool,
}

/// Handle returned by `open_journal`. Mutations go through this; Drop
/// does not implicitly delete the file — the caller must call `finish()`
/// to mark success.
pub struct JournalHandle {
    pub path: PathBuf,
    journal: Journal,
}

impl JournalHandle {
    /// Append a completed phase tag and fsync. Phase tags are short
    /// strings like "P3", "P4", "P6".
    pub fn mark_phase(&mut self, phase: &str) -> Result<(), ProjectError> {
        if !self.journal.phases_completed.iter().any(|p| p == phase) {
            self.journal.phases_completed.push(phase.to_string());
        }
        self.flush()
    }

    /// Record a snapshot path produced by a destructive phase.
    pub fn record_snapshot(&mut self, path: PathBuf) -> Result<(), ProjectError> {
        self.journal.snapshot_paths.push(path);
        self.flush()
    }

    /// Record a fatal error. Does not delete the journal.
    pub fn mark_error(&mut self, err: &str) -> Result<(), ProjectError> {
        self.journal.last_error = Some(err.to_string());
        self.flush()
    }

    /// Audit-log a stale lock we broke during acquire. Writes a
    /// `broken-lock-<ts>.json` next to the journal (spec §5.1) and
    /// keeps a pointer so `repair` can surface the trail.
    pub fn note_broken_lock(
        &mut self,
        record: &crate::project_lock::BrokenLockRecord,
    ) -> Result<(), ProjectError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let out = parent.join(format!("broken-lock-{ts}.json"));
        let body = serde_json::json!({
            "broken_at": chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "reason": record.reason,
            "prior_lock": record.prior,
            "broken_by_pid": std::process::id(),
            "related_journal": self.path,
        });
        let body_str = serde_json::to_string_pretty(&body)
            .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
        fs::write(&out, body_str).map_err(ProjectError::Io)?;
        tracing::info!(
            broken_at = ?out,
            reason = %record.reason,
            "broken-lock audit record written"
        );
        Ok(())
    }

    /// Delete the journal — signal of successful completion.
    pub fn finish(self) -> Result<(), ProjectError> {
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(ProjectError::Io)?;
        }
        Ok(())
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    fn flush(&self) -> Result<(), ProjectError> {
        write_atomic(&self.path, &self.journal)
    }
}

/// Open a fresh journal. Writes the initial file atomically before
/// returning — if the caller later crashes, the journal is already on
/// disk for `project repair` to find.
///
/// Filename carries seconds + PID + 6-char random suffix. The suffix
/// exists because Tauri can start multiple project moves concurrently
/// in the same process (locks are per-project, not global), and
/// `started_unix_secs` is second-granularity — two moves started by
/// the same process within the same wall-clock second would otherwise
/// collide and the second `write_atomic` would overwrite the first,
/// erasing its phases / snapshot paths / error state.
pub fn open_journal(journals_dir: &Path, initial: Journal) -> Result<JournalHandle, ProjectError> {
    fs::create_dir_all(journals_dir).map_err(ProjectError::Io)?;
    // Build the uniquified filename. If the caller happens to collide
    // on the first random draw (cosmically unlikely at 6 lowercase+digit
    // = ~2 billion space, but possible during tests that share the
    // journals_dir), retry up to 5 times before giving up.
    let mut attempts = 0;
    let path = loop {
        let suffix = random_suffix(6);
        let candidate = journals_dir.join(format!(
            "move-{}-{}-{}.json",
            initial.started_unix_secs, initial.pid, suffix
        ));
        if !candidate.exists() {
            break candidate;
        }
        attempts += 1;
        if attempts >= 5 {
            return Err(ProjectError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("failed to generate unique journal filename after {attempts} attempts"),
            )));
        }
    };
    write_atomic(&path, &initial)?;
    Ok(JournalHandle {
        path,
        journal: initial,
    })
}

/// Tiny ASCII-only random suffix for journal filenames. Uses
/// `SystemTime::now().subsec_nanos()` as a cheap entropy source plus
/// a per-call counter — avoids pulling in the `rand` crate just for
/// filename uniqueness. 6 chars over [0-9a-z] = ~36^6 ≈ 2.2B; far
/// more than enough to disambiguate concurrent moves in the same
/// second from the same process.
fn random_suffix(len: usize) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let bump = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let mut mix = nanos
        .wrapping_mul(6364136223846793005)
        .wrapping_add(bump.wrapping_mul(1442695040888963407));
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        s.push(alphabet[(mix as usize) % alphabet.len()] as char);
        mix = mix.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
    s
}

/// Build an initial journal from a MoveArgs-like set of values.
pub fn new_initial_journal(
    old_path: &str,
    new_path: &str,
    old_san: &str,
    new_san: &str,
    old_git_root: Option<String>,
    new_git_root: Option<String>,
    flags: JournalFlags,
) -> Journal {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let started_at = chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Journal {
        version: 1,
        started_at,
        started_unix_secs: now_unix,
        pid: std::process::id(),
        hostname: whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()),
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        old_path: old_path.to_string(),
        new_path: new_path.to_string(),
        old_san: old_san.to_string(),
        new_san: new_san.to_string(),
        old_git_root,
        new_git_root,
        flags,
        phases_completed: Vec::new(),
        snapshot_paths: Vec::new(),
        last_error: None,
    }
}

/// List all pending journals (excludes `.abandoned.json` sidecars).
/// Returns (path, parsed) tuples; files that fail to parse are
/// surfaced as synthetic "unparseable" journals so gate/repair notice
/// them rather than silently skipping.
pub fn list_pending(journals_dir: &Path) -> Result<Vec<(PathBuf, Journal)>, ProjectError> {
    if !journals_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(journals_dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("move-") || !name.ends_with(".json") {
            continue;
        }
        if name.ends_with(".abandoned.json") {
            continue;
        }
        let path = entry.path();
        match fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Journal>(&s).ok())
        {
            Some(j) => out.push((path, j)),
            None => {
                tracing::warn!(file = ?path, "unparseable journal — surfacing");
                // Synthetic placeholder: mark as present so gate/repair
                // see it and the user must resolve via --abandon or
                // manual deletion after inspection.
                let placeholder = Journal {
                    version: 0,
                    started_at: "?".to_string(),
                    started_unix_secs: 0,
                    pid: 0,
                    hostname: "?".to_string(),
                    claudepot_version: "?".to_string(),
                    old_path: "?".to_string(),
                    new_path: "?".to_string(),
                    old_san: "?".to_string(),
                    new_san: "?".to_string(),
                    old_git_root: None,
                    new_git_root: None,
                    flags: JournalFlags::default(),
                    phases_completed: vec![],
                    snapshot_paths: vec![],
                    last_error: Some("unparseable journal on disk".to_string()),
                };
                out.push((path, placeholder));
            }
        }
    }
    // Oldest first.
    out.sort_by_key(|(_, j)| j.started_unix_secs);
    Ok(out)
}

/// List pending journals with explicitly-abandoned ones filtered out.
///
/// `list_pending` returns every parseable `move-*.json` it finds, even
/// ones the user marked `.abandoned.json`. Callers that gate mutating
/// commands or print user-facing pending banners want the
/// "actionable" subset — pending journals the user has not yet
/// dismissed. This helper packages that filter so it can't drift
/// across CLI / Tauri call sites.
pub fn list_active_pending(journals_dir: &Path) -> Result<Vec<(PathBuf, Journal)>, ProjectError> {
    let all = list_pending(journals_dir)?;
    Ok(all
        .into_iter()
        .filter(|(path, _)| !abandoned_path(path).exists())
        .collect())
}

/// Status classification per spec §6 (informed by Q6 lockfile and Q7
/// nag thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalStatus {
    /// Lock file present and live (same-host PID exists).
    Running,
    /// Dead/absent lock, age < 24h.
    Pending,
    /// Dead/absent lock, age ≥ 24h.
    Stale,
    /// `.abandoned.json` sidecar found.
    Abandoned,
}

impl JournalStatus {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Pending => "pending",
            Self::Stale => "stale",
            Self::Abandoned => "abandoned",
        }
    }
}

/// Classify a journal. Pass `now_unix_secs` for testability; in
/// production callers pass `SystemTime::now()` via the helper.
pub fn classify(
    journal_path: &Path,
    journal: &Journal,
    lock_is_live: bool,
    now_unix_secs: u64,
    nag_threshold_secs: u64,
) -> JournalStatus {
    let abandoned_sidecar = abandoned_path(journal_path);
    if abandoned_sidecar.exists() {
        return JournalStatus::Abandoned;
    }
    if lock_is_live {
        return JournalStatus::Running;
    }
    let age = now_unix_secs.saturating_sub(journal.started_unix_secs);
    if age >= nag_threshold_secs {
        JournalStatus::Stale
    } else {
        JournalStatus::Pending
    }
}

/// Write a `.abandoned.json` sidecar next to the journal. The journal
/// itself is preserved for audit (spec §6).
pub fn mark_abandoned(journal_path: &Path) -> Result<PathBuf, ProjectError> {
    let sidecar = abandoned_path(journal_path);
    let stamp = chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let body = serde_json::json!({
        "abandoned_at": stamp,
        "abandoned_by_pid": std::process::id(),
    });
    let body_str = serde_json::to_string_pretty(&body)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    fs::write(&sidecar, body_str).map_err(ProjectError::Io)?;
    Ok(sidecar)
}

fn abandoned_path(journal_path: &Path) -> PathBuf {
    let mut s = journal_path.as_os_str().to_os_string();
    s.push(".abandoned.json");
    // Replace .json.abandoned.json → .abandoned.json by switching extension.
    // Simpler: build directly on the stem.
    if let (Some(stem), Some(parent)) = (journal_path.file_stem(), journal_path.parent()) {
        return parent.join(format!("{}.abandoned.json", stem.to_string_lossy()));
    }
    PathBuf::from(s)
}

fn write_atomic(path: &Path, journal: &Journal) -> Result<(), ProjectError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(ProjectError::Io)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(ProjectError::Io)?;
    let json = serde_json::to_string_pretty(journal)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    tmp.write_all(json.as_bytes()).map_err(ProjectError::Io)?;
    tmp.write_all(b"\n").map_err(ProjectError::Io)?;
    tmp.as_file().sync_all().map_err(ProjectError::Io)?;
    tmp.persist(path).map_err(|e| ProjectError::Io(e.error))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn mkjournal(old: &str, new: &str, started: u64) -> Journal {
        Journal {
            version: 1,
            started_at: "2026-04-15T00:00:00Z".to_string(),
            started_unix_secs: started,
            pid: 42,
            hostname: "test-host".to_string(),
            claudepot_version: "0.1.0".to_string(),
            old_path: old.to_string(),
            new_path: new.to_string(),
            old_san: "-old".to_string(),
            new_san: "-new".to_string(),
            old_git_root: None,
            new_git_root: None,
            flags: JournalFlags::default(),
            phases_completed: vec![],
            snapshot_paths: vec![],
            last_error: None,
        }
    }

    #[test]
    fn test_open_and_finish_journal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let h = open_journal(&dir, mkjournal("/a", "/b", 1000)).unwrap();
        assert!(h.path.exists());
        h.finish().unwrap();
        // Dir stays, file gone.
        assert!(dir.exists());
    }

    #[test]
    fn test_open_journal_unique_filename_same_second_same_pid() {
        // Regression guard for audit H5. Two moves started in the
        // same wall-clock second by the same process must get
        // distinct journal filenames — otherwise the second open
        // would silently overwrite the first.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let j = mkjournal("/a", "/b", 1000);
        let h1 = open_journal(&dir, j.clone()).unwrap();
        let h2 = open_journal(&dir, j).unwrap();
        assert_ne!(h1.path, h2.path, "journal filenames must be unique");
        assert!(h1.path.exists());
        assert!(h2.path.exists());
    }

    #[test]
    fn test_mark_phase_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let mut h = open_journal(&dir, mkjournal("/a", "/b", 1000)).unwrap();
        h.mark_phase("P3").unwrap();
        h.mark_phase("P4").unwrap();
        let read: Journal = serde_json::from_str(&fs::read_to_string(&h.path).unwrap()).unwrap();
        assert_eq!(read.phases_completed, vec!["P3", "P4"]);

        // Idempotent.
        h.mark_phase("P3").unwrap();
        let read2: Journal = serde_json::from_str(&fs::read_to_string(&h.path).unwrap()).unwrap();
        assert_eq!(read2.phases_completed, vec!["P3", "P4"]);
    }

    #[test]
    fn test_record_snapshot_and_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let mut h = open_journal(&dir, mkjournal("/a", "/b", 1000)).unwrap();
        let snap = PathBuf::from("/tmp/snap.json");
        h.record_snapshot(snap.clone()).unwrap();
        h.mark_error("uh oh").unwrap();
        let read: Journal = serde_json::from_str(&fs::read_to_string(&h.path).unwrap()).unwrap();
        assert_eq!(read.snapshot_paths, vec![snap]);
        assert_eq!(read.last_error.as_deref(), Some("uh oh"));
    }

    #[test]
    fn test_list_pending_sorted_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let _ = open_journal(&dir, mkjournal("/a", "/b", 2000)).unwrap();
        let _ = open_journal(&dir, mkjournal("/c", "/d", 1500)).unwrap();
        let _ = open_journal(&dir, mkjournal("/e", "/f", 1800)).unwrap();

        let pending = list_pending(&dir).unwrap();
        let timestamps: Vec<u64> = pending.iter().map(|(_, j)| j.started_unix_secs).collect();
        assert_eq!(timestamps, vec![1500, 1800, 2000]);
    }

    #[test]
    fn test_list_pending_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        assert!(list_pending(&dir).unwrap().is_empty());
    }

    #[test]
    fn test_list_active_pending_excludes_abandoned() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let active = open_journal(&dir, mkjournal("/a", "/b", 1000)).unwrap();
        let dropped = open_journal(&dir, mkjournal("/c", "/d", 1500)).unwrap();
        // Mark the second journal abandoned. `list_pending` still
        // returns it (it's the parent journal, not the sidecar);
        // `list_active_pending` must filter it out.
        mark_abandoned(&dropped.path).unwrap();

        let raw_paths: Vec<PathBuf> = list_pending(&dir)
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert!(raw_paths.contains(&active.path));
        assert!(raw_paths.contains(&dropped.path));

        let active_paths: Vec<PathBuf> = list_active_pending(&dir)
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert!(active_paths.contains(&active.path));
        assert!(!active_paths.contains(&dropped.path));
    }

    #[test]
    fn test_mark_abandoned_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let h = open_journal(&dir, mkjournal("/a", "/b", 1000)).unwrap();
        let sidecar = mark_abandoned(&h.path).unwrap();
        assert!(sidecar.exists());
        assert!(sidecar
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".abandoned.json"));

        // List should now skip this journal as abandoned.
        let status = classify(&h.path, h.journal(), false, 1000, 86400);
        assert_eq!(status, JournalStatus::Abandoned);
    }

    #[test]
    fn test_classify_running_pending_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journals");
        let h = open_journal(&dir, mkjournal("/a", "/b", 1000)).unwrap();

        // Lock live → running (regardless of age).
        assert_eq!(
            classify(&h.path, h.journal(), true, 9_999_999, 86400),
            JournalStatus::Running
        );
        // Lock dead, young → pending.
        assert_eq!(
            classify(&h.path, h.journal(), false, 1_500, 86400),
            JournalStatus::Pending
        );
        // Lock dead, old → stale.
        assert_eq!(
            classify(&h.path, h.journal(), false, 90_000, 86400),
            JournalStatus::Stale
        );
    }
}
