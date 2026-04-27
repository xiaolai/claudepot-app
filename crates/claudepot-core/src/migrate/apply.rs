//! Apply pipeline — staging, locks, journal, atomic apply, rollback.
//!
//! See `dev-docs/project-migrate-spec.md` §8.
//!
//! Phase contract (P0..P8):
//!   P0 inspect — verify integrity, parse manifest.
//!   P1 stage — extract under `~/.claudepot/imports/<id>/staging/`,
//!              normalize file modes, reject symlinks/dotdot
//!              (handled inside `bundle::BundleReader::extract_all`).
//!   P2 plan — substitution table + conflict detection (caller).
//!   P3 rewrite — slugs, JSONLs, fragment, history fragment, file-history
//!              repath. **In staging.**
//!   P4 lock — global import lock + per-project locks.
//!   P5 apply — atomic rename staged → final per artifact.
//!   P6 verify — sanity-check the rewrites landed.
//!   P7 reindex — trigger session_index rebuild.
//!   P8 release — discard staging, keep journal 24h.
//!
//! v0 scope: this module ships P1, the journal helpers, and the
//! rollback primitive. The full apply pipeline is the next layer of
//! work (see `mod.rs` for the orchestrator stub).

use crate::migrate::error::MigrateError;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Root directory for in-flight import staging.
pub fn imports_root() -> PathBuf {
    paths::claudepot_data_dir().join("imports")
}

/// Per-import staging dir: `~/.claudepot/imports/<bundle_id>/`.
pub fn staging_dir(bundle_id: &str) -> PathBuf {
    imports_root().join(bundle_id).join("staging")
}

/// Per-import journal path: `~/.claudepot/repair/journals/import-<id>.json`.
pub fn journal_path(bundle_id: &str) -> PathBuf {
    let (journals, _, _) = paths::claudepot_repair_dirs();
    journals.join(format!("import-{bundle_id}.json"))
}

/// One step in the import journal. Each apply phase that touches the
/// target tree records BEFORE / AFTER paths so rollback can reverse
/// LIFO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalStep {
    pub kind: JournalStepKind,
    pub before: Option<String>,
    pub after: Option<String>,
    /// Recorded sha256 of `before` so rollback can refuse to overwrite
    /// a file that's been touched by something else since the apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_sha256: Option<String>,
    pub timestamp_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStepKind {
    /// Created a new file at `after` from the bundle.
    CreateFile,
    /// Renamed `before` → `after` (e.g. file-history repath).
    RenameFile,
    /// Replaced `after` with bundle content; `before` was archived to
    /// `before` (a snapshot path under `~/.claudepot/repair/snapshots/`).
    ReplaceFile,
    /// Wrote a `~/.claude.json` projects-map fragment update.
    WriteJsonFragment,
    /// Trigger to rebuild the session index for the slugs touched.
    ReindexSession,
}

/// Top-level import journal. Persisted at `journal_path(bundle_id)`
/// for 24 hours after apply (§8.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportJournal {
    pub bundle_id: String,
    pub started_unix_secs: u64,
    pub finished_unix_secs: Option<u64>,
    pub claudepot_version: String,
    pub steps: Vec<JournalStep>,
    /// True once the apply pipeline finished P5..P8. Rollback target
    /// when undo runs within the 24h window.
    pub committed: bool,
}

impl ImportJournal {
    pub fn new(bundle_id: String) -> Self {
        Self {
            bundle_id,
            started_unix_secs: now_secs(),
            finished_unix_secs: None,
            claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
            steps: Vec::new(),
            committed: false,
        }
    }

    pub fn record(&mut self, step: JournalStep) {
        self.steps.push(step);
    }

    /// Persist to disk atomically (tempfile + rename).
    pub fn persist(&self, path: &Path) -> Result<(), MigrateError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(MigrateError::from)?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| MigrateError::Serialize(e.to_string()))?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(MigrateError::from)?;
        std::io::Write::write_all(&mut tmp, &bytes).map_err(MigrateError::from)?;
        tmp.persist(path).map_err(|e| MigrateError::from(e.error))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, MigrateError> {
        let bytes = fs::read(path).map_err(MigrateError::from)?;
        serde_json::from_slice(&bytes).map_err(|e| MigrateError::Serialize(e.to_string()))
    }

    pub fn mark_committed(&mut self) {
        self.committed = true;
        self.finished_unix_secs = Some(now_secs());
    }
}

/// Drop the staging tree for an import. Idempotent: missing → ok.
pub fn discard_staging(bundle_id: &str) -> Result<(), MigrateError> {
    let dir = staging_dir(bundle_id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(MigrateError::from)?;
    }
    // Also clean up the parent `<bundle_id>/` if empty.
    let parent = dir.parent();
    if let Some(p) = parent {
        if p.exists() {
            // Best-effort; ignore non-empty dirs (might still hold
            // diagnostics).
            let _ = fs::remove_dir(p);
        }
    }
    Ok(())
}

/// Check whether a journal is within the 24h undo window.
pub fn within_undo_window(journal: &ImportJournal) -> bool {
    const UNDO_WINDOW_SECS: u64 = 24 * 60 * 60;
    let pivot = journal.finished_unix_secs.unwrap_or(journal.started_unix_secs);
    let age = now_secs().saturating_sub(pivot);
    age <= UNDO_WINDOW_SECS
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;

    #[test]
    fn journal_round_trip() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("import-x.json");

        let mut j = ImportJournal::new("abc".to_string());
        j.record(JournalStep {
            kind: JournalStepKind::CreateFile,
            before: None,
            after: Some("/tmp/x".to_string()),
            before_sha256: None,
            timestamp_unix_secs: 1,
        });
        j.persist(&p).unwrap();

        let back = ImportJournal::load(&p).unwrap();
        assert_eq!(back.bundle_id, "abc");
        assert_eq!(back.steps.len(), 1);
    }

    #[test]
    fn discard_staging_is_idempotent() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path());
        let id = "abc-test";
        let dir = staging_dir(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a"), "x").unwrap();
        discard_staging(id).unwrap();
        assert!(!dir.exists());
        // Second call: no error.
        discard_staging(id).unwrap();
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn within_undo_window_fresh_journal() {
        let j = ImportJournal::new("x".to_string());
        assert!(within_undo_window(&j));
    }

    #[test]
    fn within_undo_window_old_journal() {
        let mut j = ImportJournal::new("x".to_string());
        j.started_unix_secs = 0;
        j.finished_unix_secs = Some(0);
        assert!(!within_undo_window(&j));
    }
}
