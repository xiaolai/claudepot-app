//! Merge-mode apply — union a staged slug tree into an *existing*
//! target slug tree under a uniform prefer policy.
//!
//! See `dev-docs/archive/project-migrate-spec.md` §6.
//!
//! ## Why this is a file-level union and not a content merge
//!
//! Spec §6 is explicit that conflict resolution is "project-level
//! only. There is no per-session UI; the engine handles session-id
//! collisions inside `merge` mode under a uniform policy." So a
//! session that exists on both sides is resolved *whole*: one copy
//! wins, the other is preserved out-of-band (snapshot, for undo).
//! Nothing interleaves two transcripts.
//!
//! That is the same call CC makes when one sessionId resolves to
//! several files — newest mtime wins, no merge
//! (`listSessionsImpl.ts:246-261`) — and the same call the analysis
//! corpus makes (`corpus.rs:572`, most-complete copy wins). Merging
//! transcript *content* would need a common ancestor, which a bundle
//! does not carry; §1 scopes that to a separate continuous-sync
//! feature.
//!
//! ## Ordering contract
//!
//! Every replaced file is snapshotted *before* it is overwritten, so
//! the journal's `snapshot_path` always points at recoverable bytes.
//! A failure between snapshot and place leaves the target untouched;
//! a failure after leaves a journal step whose rollback restores the
//! snapshot.

use crate::migrate::apply;
use crate::migrate::conflicts::MergePreference;
use crate::migrate::error::MigrateError;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// What happened to one file during a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeApplyKind {
    /// Target had no file at this relative path; the imported copy
    /// landed. Rollback deletes it.
    Added,
    /// Both sides had the file and `prefer = Imported`: the target's
    /// copy was snapshotted, then overwritten. Rollback restores the
    /// snapshot.
    Replaced,
    /// Both sides had the file and `prefer = Target`: the target's
    /// copy stands and the imported one is dropped. Nothing to roll
    /// back.
    KeptTarget,
}

/// One file's disposition. Mirrors `worktree::WorktreeApplyStep`'s
/// shape so `mod.rs` maps both into `apply::JournalStep` the same way.
#[derive(Debug, Clone)]
pub struct MergeStep {
    pub kind: MergeApplyKind,
    /// Absolute path of the file in the target tree.
    pub after: String,
    /// Where the target's prior content was archived. `Some` only for
    /// `Replaced`.
    pub snapshot_path: Option<String>,
}

/// Top-level session ids present in a CC slug directory.
///
/// CC names a transcript `<sessionId>.jsonl` at the top level of the
/// slug dir; nested dirs (`subagents/`, `memory/`) are not sessions.
/// The stem *is* the identity — CC derives it from the filename and
/// validates it as a UUID (`listSessionsImpl.ts:183-185`), and so does
/// Claudepot (`session/core.rs:549`). Reading the id out of the JSONL
/// body would disagree with both.
///
/// Returns an empty set when `dir` is absent or unreadable — callers
/// treat that as "no overlap", which is correct for a target slug that
/// does not exist yet.
pub fn session_ids_in(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            if let Some(stem) = path.file_stem() {
                out.insert(stem.to_string_lossy().to_string());
            }
        }
    }
    out
}

/// Session ids in a bundle's slug tree, read from the manifest's file
/// inventory instead of from disk.
///
/// The dry run runs before extraction, so it has no staging tree to
/// walk. This applies the *same* rule as [`session_ids_in`] —
/// top-level `*.jsonl` stems only — so a dry run predicts the
/// resolution apply will reach. Two different notions of "which
/// sessions does this bundle carry" would let the plan disagree with
/// the outcome, which is the one thing a dry run must never do.
///
/// `slug_prefix` is the bundle-relative directory prefix, trailing
/// separator included.
pub fn session_ids_in_inventory<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    slug_prefix: &str,
) -> BTreeSet<String> {
    paths
        .into_iter()
        .filter_map(|p| p.strip_prefix(slug_prefix))
        // Bundle paths are always `/`-joined; nested means not a session.
        .filter(|rest| !rest.contains('/'))
        .filter_map(|rest| rest.strip_suffix(".jsonl"))
        .map(|s| s.to_string())
        .collect()
}

/// Union `staged_slug_root` into `target_slug_dir`, which must already
/// exist (the caller only reaches merge when the target slug is
/// present). Returns one step per staged file.
///
/// Files are placed individually rather than by directory rename: the
/// target tree holds sessions the bundle knows nothing about, and a
/// directory rename would erase them.
/// `steps` is an **out-parameter, not a return value**, on purpose: a
/// failure partway through has already moved files onto the target, and
/// the caller must journal those before propagating. Returning
/// `Result<Vec<MergeStep>>` discarded them on the error path, leaving
/// placed files unjournaled — invisible to rollback, with their
/// snapshots orphaned.
pub fn merge_slug_tree(
    staged_slug_root: &Path,
    target_slug_dir: &Path,
    prefer: MergePreference,
    bundle_id: &str,
    steps: &mut Vec<MergeStep>,
) -> Result<(), MigrateError> {
    // `collect_dir_inventory` walks recursively and yields `/`-joined
    // paths relative to the root, on every platform.
    for rel in apply::collect_dir_inventory(staged_slug_root) {
        let from = join_rel(staged_slug_root, &rel);
        let to = join_rel(target_slug_dir, &rel);

        if to.exists() {
            match prefer {
                MergePreference::Target => {
                    steps.push(MergeStep {
                        kind: MergeApplyKind::KeptTarget,
                        after: to.to_string_lossy().to_string(),
                        snapshot_path: None,
                    });
                }
                MergePreference::Imported => {
                    // Snapshot BEFORE the overwrite — see the ordering
                    // contract in the module docs.
                    let snap = apply::snapshot_file(bundle_id, &to)?;
                    place_file(&from, &to)?;
                    steps.push(MergeStep {
                        kind: MergeApplyKind::Replaced,
                        after: to.to_string_lossy().to_string(),
                        snapshot_path: snap.map(|p| p.to_string_lossy().to_string()),
                    });
                }
            }
        } else {
            place_file(&from, &to)?;
            steps.push(MergeStep {
                kind: MergeApplyKind::Added,
                after: to.to_string_lossy().to_string(),
                snapshot_path: None,
            });
        }
    }
    Ok(())
}

/// Join a `/`-separated relative path onto `base` using native
/// separators. `collect_dir_inventory` normalizes `\` to `/` on
/// Windows, so this reverses that for the filesystem call.
fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(seg);
    }
    p
}

/// Move `from` onto `to`, creating parents. Falls back to copy when the
/// two sit on different volumes — staging lives under `~/.claudepot`
/// and the target under `~/.claude`, which are usually but not always
/// the same filesystem. Mirrors the EXDEV handling in `mod.rs`'s
/// whole-tree rename.
fn place_file(from: &Path, to: &Path) -> Result<(), MigrateError> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(MigrateError::from)?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e)
            if e.kind() == std::io::ErrorKind::CrossesDevices
                || e.raw_os_error() == Some(libc::EXDEV) =>
        {
            fs::copy(from, to).map_err(MigrateError::from)?;
            let _ = fs::remove_file(from);
            Ok(())
        }
        Err(e) => Err(MigrateError::from(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, body: &str) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    #[test]
    fn session_ids_reads_top_level_jsonl_stems_only() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path();
        write(&root.join("aaa.jsonl"), "{}");
        write(&root.join("bbb.jsonl"), "{}");
        // Not sessions: wrong extension, and a nested transcript.
        write(&root.join("notes.meta.json"), "{}");
        write(&root.join("aaa").join("subagents").join("ccc.jsonl"), "{}");

        let ids = session_ids_in(root);
        assert_eq!(
            ids.into_iter().collect::<Vec<_>>(),
            vec!["aaa".to_string(), "bbb".to_string()]
        );
    }

    #[test]
    fn session_ids_of_missing_dir_is_empty() {
        let td = tempfile::tempdir().unwrap();
        assert!(session_ids_in(&td.path().join("nope")).is_empty());
    }

    #[test]
    fn merge_adds_files_the_target_lacks() {
        let td = tempfile::tempdir().unwrap();
        let staged = td.path().join("staged");
        let target = td.path().join("target");
        write(&staged.join("new.jsonl"), "imported");
        fs::create_dir_all(&target).unwrap();

        let mut steps = Vec::new();
        merge_slug_tree(
            &staged,
            &target,
            MergePreference::Imported,
            "b1",
            &mut steps,
        )
        .unwrap();

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kind, MergeApplyKind::Added);
        assert_eq!(
            fs::read_to_string(target.join("new.jsonl")).unwrap(),
            "imported"
        );
    }

    /// The union property: sessions the bundle never heard of must
    /// survive. A whole-directory rename would have erased this file.
    #[test]
    fn merge_preserves_target_only_sessions() {
        let td = tempfile::tempdir().unwrap();
        let staged = td.path().join("staged");
        let target = td.path().join("target");
        write(&staged.join("from-peer.jsonl"), "imported");
        write(&target.join("local-only.jsonl"), "mine");

        let mut steps = Vec::new();
        merge_slug_tree(
            &staged,
            &target,
            MergePreference::Imported,
            "b1",
            &mut steps,
        )
        .unwrap();
        let _ = &steps;

        assert_eq!(
            fs::read_to_string(target.join("local-only.jsonl")).unwrap(),
            "mine",
            "a session absent from the bundle must survive the merge"
        );
        assert_eq!(
            fs::read_to_string(target.join("from-peer.jsonl")).unwrap(),
            "imported"
        );
    }

    #[test]
    fn prefer_imported_overwrites_and_snapshots() {
        let td = tempfile::tempdir().unwrap();
        let staged = td.path().join("staged");
        let target = td.path().join("target");
        write(&staged.join("dup.jsonl"), "imported");
        write(&target.join("dup.jsonl"), "target");

        let mut steps = Vec::new();
        merge_slug_tree(
            &staged,
            &target,
            MergePreference::Imported,
            "b-imp",
            &mut steps,
        )
        .unwrap();

        assert_eq!(steps[0].kind, MergeApplyKind::Replaced);
        assert_eq!(
            fs::read_to_string(target.join("dup.jsonl")).unwrap(),
            "imported"
        );
        let snap = steps[0].snapshot_path.as_ref().expect("snapshot recorded");
        assert_eq!(
            fs::read_to_string(snap).unwrap(),
            "target",
            "the overwritten copy must be recoverable for undo"
        );
    }

    #[test]
    fn prefer_target_keeps_target_and_takes_no_snapshot() {
        let td = tempfile::tempdir().unwrap();
        let staged = td.path().join("staged");
        let target = td.path().join("target");
        write(&staged.join("dup.jsonl"), "imported");
        write(&target.join("dup.jsonl"), "target");

        let mut steps = Vec::new();
        merge_slug_tree(
            &staged,
            &target,
            MergePreference::Target,
            "b-tgt",
            &mut steps,
        )
        .unwrap();

        assert_eq!(steps[0].kind, MergeApplyKind::KeptTarget);
        assert!(steps[0].snapshot_path.is_none());
        assert_eq!(
            fs::read_to_string(target.join("dup.jsonl")).unwrap(),
            "target"
        );
    }

    #[test]
    fn merge_walks_nested_dirs() {
        let td = tempfile::tempdir().unwrap();
        let staged = td.path().join("staged");
        let target = td.path().join("target");
        write(&staged.join("sid").join("subagents").join("a.jsonl"), "sub");
        fs::create_dir_all(&target).unwrap();

        let mut steps = Vec::new();
        merge_slug_tree(
            &staged,
            &target,
            MergePreference::Imported,
            "b1",
            &mut steps,
        )
        .unwrap();

        assert_eq!(steps.len(), 1);
        assert_eq!(
            fs::read_to_string(target.join("sid").join("subagents").join("a.jsonl")).unwrap(),
            "sub"
        );
    }
}
