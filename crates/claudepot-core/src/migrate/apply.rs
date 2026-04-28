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
    /// For ReplaceFile / WriteJsonFragment: where the prior content
    /// was archived under `~/.claudepot/repair/snapshots/import-<id>/`.
    /// Rollback restores from this path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
    /// Recorded sha256 of `after` at apply time. Rollback uses this to
    /// detect post-apply tampering — if the on-disk sha differs, the
    /// step is skipped with a warning rather than blindly removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sha256: Option<String>,
    /// JSON-pointer-ish key for WriteJsonFragment steps: the path under
    /// `~/.claude.json` whose `projects[<key>]` we wrote. Rollback
    /// removes that key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment_key: Option<String>,
    /// For CreateDir steps: per-file inventory (relative-to-`after`)
    /// of every file the import wrote into the directory. Rollback
    /// removes only these paths so post-import user work survives.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dir_inventory: Vec<String>,
    pub timestamp_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStepKind {
    /// Created a new file at `after` from the bundle. Rollback: delete
    /// `after`.
    CreateFile,
    /// Created a new directory tree at `after` from the bundle.
    /// Rollback: surgical removal — read the journal's
    /// `dir_inventory` (a list of bundle-relative file paths) and
    /// remove only those files; user-added files post-import survive
    /// rollback and are flagged as tampered. The dir itself is
    /// removed only when it's empty after surgical removal.
    CreateDir,
    /// Renamed `before` → `after` (e.g. file-history repath). Rollback:
    /// rename `after` back to `before`.
    RenameFile,
    /// Replaced `after` with bundle content; `before` was archived to
    /// the snapshot path recorded in `snapshot_path`. Rollback: restore
    /// from snapshot.
    ReplaceFile,
    /// Wrote a `~/.claude.json` projects-map fragment update under key
    /// `after` (a JSON pointer string like `projects[/Users/joker/x]`).
    /// Rollback: remove the key (or restore prior content from
    /// `snapshot_path` if non-empty).
    WriteJsonFragment,
    /// Trigger to rebuild the session index for the slugs touched.
    /// Rollback: re-trigger reindex (idempotent, advisory).
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
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| MigrateError::Serialize(e.to_string()))?;
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

/// Per-import snapshot dir: `~/.claudepot/repair/snapshots/import-<id>/`.
/// Used by ReplaceFile and WriteJsonFragment to archive prior content
/// before overwriting.
pub fn snapshot_dir(bundle_id: &str) -> PathBuf {
    let (_, _, snapshots) = paths::claudepot_repair_dirs();
    snapshots.join(format!("import-{bundle_id}"))
}

/// Archive a file's current content to the snapshot dir before
/// overwriting. Returns the snapshot path. Idempotent: if the source
/// doesn't exist, returns `None` (caller records `snapshot_path: None`
/// so rollback knows there was nothing to restore).
pub fn snapshot_file(bundle_id: &str, target: &Path) -> Result<Option<PathBuf>, MigrateError> {
    if !target.exists() {
        return Ok(None);
    }
    let dir = snapshot_dir(bundle_id);
    fs::create_dir_all(&dir).map_err(MigrateError::from)?;
    // Snapshot filename = sha256(target_path) + suffix to preserve
    // extension. Avoids collisions when two steps archive different
    // files with the same basename.
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(target.to_string_lossy().as_bytes());
    let key = hex::encode(&h.finalize()[..8]);
    let suffix = target
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let snap = dir.join(format!("{key}{suffix}"));
    fs::copy(target, &snap).map_err(MigrateError::from)?;
    Ok(Some(snap))
}

/// Compute sha256 of a file. Returns `None` if missing (treated as
/// "post-apply removal" by rollback callers).
pub fn sha256_of_file_optional(path: &Path) -> Option<String> {
    use sha2::Digest;
    let bytes = fs::read(path).ok()?;
    let mut h = sha2::Sha256::new();
    h.update(&bytes);
    Some(hex::encode(h.finalize()))
}

/// Outcome of `rollback`. Carries per-step disposition so callers can
/// distinguish "fully reversed" from "partially reversed; user must
/// reconcile."
#[derive(Debug, Clone, Default)]
pub struct RollbackReport {
    /// Steps successfully reversed.
    pub reversed: usize,
    /// Steps skipped because the on-disk state diverged from the
    /// recorded sha256 (post-apply tamper). Each entry carries a
    /// human-readable message.
    pub skipped_tampered: Vec<String>,
    /// Steps that errored during rollback. Each entry is a
    /// per-step error message; rollback continues past errors so a
    /// single failure doesn't strand the rest of the import.
    pub errors: Vec<String>,
    /// True if the journal was marked uncommitted (rollback ran on a
    /// failed apply rather than user-requested undo).
    pub from_failed_apply: bool,
}

/// Rollback the journal in LIFO order. Each step's `after_sha256`
/// (when present) is verified against the on-disk file; mismatches
/// skip with a `skipped_tampered` entry. Best-effort: errors on one
/// step do not abort the rest.
///
/// After a successful rollback, the journal is marked uncommitted by
/// the caller (this function does not mutate the journal in place).
/// Callers that want a counter-journal (undo of undo) should write
/// one separately — `mod.rs::import_undo` shows the canonical pattern.
pub fn rollback(journal: &ImportJournal) -> Result<RollbackReport, MigrateError> {
    let mut report = RollbackReport {
        from_failed_apply: !journal.committed,
        ..Default::default()
    };

    for step in journal.steps.iter().rev() {
        match rollback_one(step) {
            Ok(StepRollback::Reversed) => report.reversed += 1,
            Ok(StepRollback::SkippedTampered(msg)) => report.skipped_tampered.push(msg),
            Ok(StepRollback::SkippedNoOp) => {}
            Err(e) => report.errors.push(format!(
                "rollback {}: {}",
                step.after.as_deref().unwrap_or("<none>"),
                e
            )),
        }
    }
    Ok(report)
}

enum StepRollback {
    Reversed,
    SkippedTampered(String),
    SkippedNoOp,
}

fn rollback_one(step: &JournalStep) -> Result<StepRollback, MigrateError> {
    match step.kind {
        JournalStepKind::CreateFile => {
            let Some(after) = step.after.as_ref() else {
                return Ok(StepRollback::SkippedNoOp);
            };
            let path = Path::new(after);
            if !path.exists() {
                return Ok(StepRollback::SkippedNoOp);
            }
            // If we recorded an after_sha256, verify before deleting.
            if let Some(expected) = step.after_sha256.as_ref() {
                if let Some(actual) = sha256_of_file_optional(path) {
                    if &actual != expected {
                        return Ok(StepRollback::SkippedTampered(format!(
                            "{after}: sha256 mismatch (was modified after apply)"
                        )));
                    }
                }
            }
            fs::remove_file(path).map_err(MigrateError::from)?;
            Ok(StepRollback::Reversed)
        }
        JournalStepKind::CreateDir => {
            let Some(after) = step.after.as_ref() else {
                return Ok(StepRollback::SkippedNoOp);
            };
            let path = Path::new(after);
            if !path.exists() {
                return Ok(StepRollback::SkippedNoOp);
            }
            // Surgical removal: walk the recorded inventory and remove
            // only those exact files. Anything else under the dir is
            // user work added after the import; we leave it and flag
            // it as tampered so the user knows.
            //
            // If the journal predates the dir_inventory field (legacy
            // entries written by an earlier claudepot), fall back to
            // the conservative recursive-remove behavior — but flag a
            // tamper notice so it's visible in the receipt.
            if step.dir_inventory.is_empty() {
                // Legacy journal — preserve old behavior but warn.
                fs::remove_dir_all(path).map_err(MigrateError::from)?;
                return Ok(StepRollback::SkippedTampered(format!(
                    "{after}: legacy journal had no dir_inventory; \
                     removed entire tree (post-import edits, if any, lost)"
                )));
            }
            let mut tampered: Vec<String> = Vec::new();
            for rel in &step.dir_inventory {
                let p = path.join(rel);
                match fs::remove_file(&p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        tampered.push(format!("{rel}: file disappeared (already removed)"));
                    }
                    Err(e) => return Err(MigrateError::from(e)),
                }
            }
            // Best-effort empty-dir cleanup. Walk in reverse depth
            // order so leaves go first; remove_dir is no-op on
            // non-empty so user-added files prevent removal.
            try_remove_empty_dirs(path);
            // Detect any survivors — files the user added that we
            // didn't journal. They stay on disk; we report them.
            let survivors = collect_survivors(path);
            for s in survivors {
                tampered.push(format!(
                    "{}: user-added file survived undo",
                    s.to_string_lossy()
                ));
            }
            if !tampered.is_empty() {
                return Ok(StepRollback::SkippedTampered(tampered.join("; ")));
            }
            Ok(StepRollback::Reversed)
        }
        JournalStepKind::RenameFile => {
            let (Some(before), Some(after)) = (step.before.as_ref(), step.after.as_ref()) else {
                return Ok(StepRollback::SkippedNoOp);
            };
            let after_p = Path::new(after);
            let before_p = Path::new(before);
            if !after_p.exists() {
                return Ok(StepRollback::SkippedNoOp);
            }
            if before_p.exists() {
                return Ok(StepRollback::SkippedTampered(format!(
                    "rename rollback: {before} reappeared (collision)"
                )));
            }
            fs::rename(after_p, before_p).map_err(MigrateError::from)?;
            Ok(StepRollback::Reversed)
        }
        JournalStepKind::ReplaceFile => {
            let Some(after) = step.after.as_ref() else {
                return Ok(StepRollback::SkippedNoOp);
            };
            let target = Path::new(after);
            if let Some(snap) = step.snapshot_path.as_ref() {
                let snap_p = Path::new(snap);
                if !snap_p.exists() {
                    return Ok(StepRollback::SkippedTampered(format!(
                        "snapshot missing: {snap}"
                    )));
                }
                fs::copy(snap_p, target).map_err(MigrateError::from)?;
                Ok(StepRollback::Reversed)
            } else if target.exists() {
                fs::remove_file(target).map_err(MigrateError::from)?;
                Ok(StepRollback::Reversed)
            } else {
                Ok(StepRollback::SkippedNoOp)
            }
        }
        JournalStepKind::WriteJsonFragment => {
            let Some(snap) = step.snapshot_path.as_ref() else {
                return Ok(StepRollback::SkippedNoOp);
            };
            let Some(target_str) = step.after.as_ref() else {
                return Ok(StepRollback::SkippedNoOp);
            };
            let target = Path::new(target_str);
            let snap_p = Path::new(snap);
            if !snap_p.exists() {
                return Ok(StepRollback::SkippedTampered(format!(
                    "json snapshot missing: {snap}"
                )));
            }
            fs::copy(snap_p, target).map_err(MigrateError::from)?;
            Ok(StepRollback::Reversed)
        }
        JournalStepKind::ReindexSession => {
            // Reindex is idempotent and advisory — we don't reverse it
            // explicitly; the next session_index open will resync.
            Ok(StepRollback::SkippedNoOp)
        }
    }
}

/// Walk a directory and try to remove every empty leaf. Used by
/// surgical CreateDir rollback after the journaled file removals so
/// the leftover empty subtree doesn't linger.
fn try_remove_empty_dirs(root: &Path) {
    fn recurse(p: &Path) {
        if let Ok(rd) = fs::read_dir(p) {
            for entry in rd.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    recurse(&entry.path());
                }
            }
        }
        let _ = fs::remove_dir(p);
    }
    recurse(root);
}

/// Walk a directory and collect every file that survived a surgical
/// removal pass. Used to populate the `skipped_tampered` list so the
/// user sees the post-import work the undo couldn't reverse.
fn collect_survivors(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn recurse(p: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = fs::read_dir(p) {
            for entry in rd.flatten() {
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let path = entry.path();
                if ft.is_dir() {
                    recurse(&path, out);
                } else if ft.is_file() {
                    out.push(path);
                }
            }
        }
    }
    recurse(root, &mut out);
    out
}

/// Walk a target directory tree and produce relative paths for every
/// file inside. Used at apply time to seed `JournalStep::dir_inventory`
/// so surgical rollback knows what the import wrote.
pub fn collect_dir_inventory(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn recurse(p: &Path, base: &Path, out: &mut Vec<String>) {
        if let Ok(rd) = fs::read_dir(p) {
            for entry in rd.flatten() {
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let path = entry.path();
                if ft.is_dir() {
                    recurse(&path, base, out);
                } else if ft.is_file() {
                    if let Ok(rel) = path.strip_prefix(base) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    recurse(root, root, &mut out);
    out
}

/// Discard a journal's snapshot dir. Called after a successful import
/// is committed AND past the 24h undo window, or on `--gc`.
pub fn discard_snapshots(bundle_id: &str) -> Result<(), MigrateError> {
    let dir = snapshot_dir(bundle_id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(MigrateError::from)?;
    }
    Ok(())
}

/// Check whether a journal is within the 24h undo window.
pub fn within_undo_window(journal: &ImportJournal) -> bool {
    const UNDO_WINDOW_SECS: u64 = 24 * 60 * 60;
    let pivot = journal
        .finished_unix_secs
        .unwrap_or(journal.started_unix_secs);
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
            snapshot_path: None,
            after_sha256: None,
            fragment_key: None,
            dir_inventory: Vec::new(),
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

    fn step(kind: JournalStepKind, before: Option<&str>, after: Option<&str>) -> JournalStep {
        JournalStep {
            kind,
            before: before.map(|s| s.to_string()),
            after: after.map(|s| s.to_string()),
            snapshot_path: None,
            after_sha256: None,
            fragment_key: None,
            dir_inventory: Vec::new(),
            timestamp_unix_secs: now_secs(),
        }
    }

    #[test]
    fn rollback_create_file_removes_it() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("created.txt");
        fs::write(&f, b"x").unwrap();

        let mut j = ImportJournal::new("rb1".to_string());
        j.record(step(
            JournalStepKind::CreateFile,
            None,
            Some(f.to_str().unwrap()),
        ));
        j.mark_committed();

        let report = rollback(&j).unwrap();
        assert_eq!(report.reversed, 1);
        assert!(report.errors.is_empty());
        assert!(!f.exists());
    }

    #[test]
    fn rollback_rename_reverses_direction() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let before = tmp.path().join("a.txt");
        let after = tmp.path().join("b.txt");
        fs::write(&after, b"renamed").unwrap();

        let mut j = ImportJournal::new("rb2".to_string());
        j.record(step(
            JournalStepKind::RenameFile,
            Some(before.to_str().unwrap()),
            Some(after.to_str().unwrap()),
        ));
        let report = rollback(&j).unwrap();
        assert_eq!(report.reversed, 1);
        assert!(before.exists());
        assert!(!after.exists());
        assert_eq!(fs::read(&before).unwrap(), b"renamed");
    }

    #[test]
    fn rollback_create_dir_removes_tree() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("d");
        fs::create_dir_all(d.join("nested")).unwrap();
        fs::write(d.join("nested/x"), b"x").unwrap();

        let mut j = ImportJournal::new("rb3".to_string());
        let mut s = step(JournalStepKind::CreateDir, None, Some(d.to_str().unwrap()));
        // New contract: dir_inventory must list every file the
        // import wrote so surgical rollback can target them.
        s.dir_inventory = vec!["nested/x".to_string()];
        j.record(s);
        let report = rollback(&j).unwrap();
        assert_eq!(report.reversed, 1);
        assert!(!d.exists());
    }

    #[test]
    fn rollback_create_dir_preserves_user_added_files() {
        // Audit Robustness fix: when the user adds files into the
        // imported tree post-import, rollback must NOT delete them.
        // It walks `dir_inventory`, removes only those, and surfaces
        // the survivors as `skipped_tampered`.
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("p");
        fs::create_dir_all(d.join("nested")).unwrap();
        // Imported file (in inventory).
        fs::write(d.join("nested/imported.jsonl"), b"x").unwrap();
        // User-added file (NOT in inventory) — must survive.
        fs::write(d.join("user-added.txt"), b"hello").unwrap();

        let mut j = ImportJournal::new("rb-survives".to_string());
        let mut s = step(JournalStepKind::CreateDir, None, Some(d.to_str().unwrap()));
        s.dir_inventory = vec!["nested/imported.jsonl".to_string()];
        j.record(s);

        let report = rollback(&j).unwrap();
        // 0 reversed (we skipped this step due to tamper); 1 tampered.
        assert_eq!(report.reversed, 0);
        assert_eq!(report.skipped_tampered.len(), 1);
        assert!(
            d.exists(),
            "dir must survive when user-added files are present"
        );
        assert!(d.join("user-added.txt").exists());
        assert!(
            !d.join("nested/imported.jsonl").exists(),
            "imported file removed"
        );
    }

    #[test]
    fn rollback_create_dir_with_empty_inventory_is_legacy_path() {
        // Backward compat: a journal written before dir_inventory
        // existed has an empty inventory. We fall back to the
        // recursive-remove behavior but mark the step as tampered so
        // the user knows post-import edits (if any) were lost.
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("legacy");
        fs::create_dir_all(d.join("a")).unwrap();
        fs::write(d.join("a/b"), b"x").unwrap();
        let mut j = ImportJournal::new("rb-legacy".to_string());
        j.record(step(
            JournalStepKind::CreateDir,
            None,
            Some(d.to_str().unwrap()),
        ));
        let report = rollback(&j).unwrap();
        assert_eq!(report.skipped_tampered.len(), 1);
        assert!(!d.exists(), "legacy path still removes the tree");
    }

    #[test]
    fn rollback_replace_restores_snapshot() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("config.json");
        let snap = tmp.path().join("snap-0001.json");
        fs::write(&target, b"new content").unwrap();
        fs::write(&snap, b"old content").unwrap();

        let mut j = ImportJournal::new("rb4".to_string());
        let mut s = step(
            JournalStepKind::ReplaceFile,
            None,
            Some(target.to_str().unwrap()),
        );
        s.snapshot_path = Some(snap.to_str().unwrap().to_string());
        j.record(s);
        let report = rollback(&j).unwrap();
        assert_eq!(report.reversed, 1);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
    }

    #[test]
    fn rollback_skips_tampered_files() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("tampered.txt");
        fs::write(&f, b"original").unwrap();

        let mut j = ImportJournal::new("rb5".to_string());
        let mut s = step(JournalStepKind::CreateFile, None, Some(f.to_str().unwrap()));
        // Pretend the original sha was different — simulating the user
        // having edited it after import.
        s.after_sha256 = Some("0".repeat(64));
        j.record(s);

        let report = rollback(&j).unwrap();
        assert_eq!(report.reversed, 0);
        assert_eq!(report.skipped_tampered.len(), 1);
        // File still present.
        assert!(f.exists());
    }

    #[test]
    fn rollback_lifo_order_preserved() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();

        // Two CreateFile steps. LIFO rollback removes b before a.
        let mut j = ImportJournal::new("rb-lifo".to_string());
        j.record(step(
            JournalStepKind::CreateFile,
            None,
            Some(a.to_str().unwrap()),
        ));
        j.record(step(
            JournalStepKind::CreateFile,
            None,
            Some(b.to_str().unwrap()),
        ));
        let report = rollback(&j).unwrap();
        assert_eq!(report.reversed, 2);
        assert!(!a.exists());
        assert!(!b.exists());
    }

    #[test]
    fn snapshot_file_skips_missing() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("d"));
        let missing = tmp.path().join("never-existed");
        let r = snapshot_file("snap-test", &missing).unwrap();
        assert!(r.is_none());
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn snapshot_file_round_trip() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("d"));
        let target = tmp.path().join("real.json");
        fs::write(&target, b"hello").unwrap();
        let snap = snapshot_file("snap-test", &target).unwrap().unwrap();
        assert!(snap.exists());
        assert_eq!(fs::read(&snap).unwrap(), b"hello");
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }
}
