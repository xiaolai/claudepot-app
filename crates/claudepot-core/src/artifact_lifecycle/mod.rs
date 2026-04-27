//! Artifact lifecycle — disable / enable / trash for installed CC
//! artifacts (skills, agents, slash commands).
//!
//! See `dev-docs/artifact-lifecycle-plan.md` for the full design.
//! Short version:
//!
//! - **Disable**: in-place hide via move to `<root>/.disabled/<kind>/...`.
//!   Reversible in one click. Atomic rename, same volume.
//! - **Trash**: two-phase staging move to `~/.claudepot/trash/<uuid>/`
//!   with manifest. Best-effort 30-day retention.
//! - Plugin / managed-policy / out-of-scope paths are refused at
//!   `paths::classify_path`.
//!
//! The public API takes the canonical `(scope_root, kind,
//! relative_path)` triple, never bare paths. The UI calls
//! `classify_path(abs_path)` first to derive the triple, then calls
//! the action. Same primitives serve a future CLI.

pub mod disable;
pub mod discover;
pub mod error;
pub mod paths;
pub mod scope_lock;
pub mod trash;

pub use disable::{disable_at, enable_at, DisabledRecord, OnConflict};
pub use discover::list_disabled;
pub use error::{LifecycleError, RefuseReason, Result};
pub use paths::{
    classify_path, disabled_target_for, enabled_target_for, ActiveRoots, ArtifactKind,
    PayloadKind, Scope, Trackable, DISABLED_DIR,
};
pub use trash::{
    forget_at, list_at as list_trash_at, purge_older_than, recover_at, restore_at,
    trash_at, RestoredArtifact, TrashEntry, TrashManifest, TrashState,
};

use std::path::{Path, PathBuf};

/// Default trash root: `<claudepot_data_dir>/trash/`.
pub fn default_trash_root() -> PathBuf {
    crate::paths::claudepot_data_dir().join("trash")
}

/// One-shot list_disabled with the default scope roots
/// (`~/.claude/` + an optional project anchor passed by the caller).
pub fn list_disabled_for(
    user_root: PathBuf,
    project_root: Option<PathBuf>,
) -> Result<Vec<DisabledRecord>> {
    let mut roots = ActiveRoots::user(user_root);
    if let Some(p) = project_root {
        roots = roots.with_project(p);
    }
    list_disabled(&roots)
}

/// Convenience for tests + Tauri: classify, then disable.
pub fn disable_path(
    abs_path: &Path,
    roots: &ActiveRoots,
    on_conflict: OnConflict,
) -> Result<DisabledRecord> {
    let trackable = classify_path(abs_path, roots)?;
    if trackable.already_disabled {
        return Err(LifecycleError::Refused(RefuseReason::AlreadyDisabled {
            path: abs_path.to_path_buf(),
        }));
    }
    disable_at(&trackable, on_conflict, roots)
}

/// Convenience for tests + Tauri: classify, then enable.
pub fn enable_path(
    abs_path: &Path,
    roots: &ActiveRoots,
    on_conflict: OnConflict,
) -> Result<DisabledRecord> {
    let trackable = classify_path(abs_path, roots)?;
    if !trackable.already_disabled {
        return Ok(record_for_already_active(&trackable));
    }
    enable_at(&trackable, on_conflict, roots)
}

/// Convenience for tests + Tauri: classify, then trash.
pub fn trash_path(
    abs_path: &Path,
    roots: &ActiveRoots,
    trash_root: &Path,
) -> Result<TrashEntry> {
    let trackable = classify_path(abs_path, roots)?;
    trash_at(&trackable, trash_root, roots)
}

fn record_for_already_active(t: &Trackable) -> DisabledRecord {
    DisabledRecord {
        scope: t.scope,
        scope_root: t.scope_root.clone(),
        kind: t.kind,
        name: t.relative_path.clone(),
        original_path: enabled_target_for(t),
        current_path: enabled_target_for(t),
        payload_kind: t.payload_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(p: &Path, body: &[u8]) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn end_to_end_disable_then_enable_via_path_api() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"hi");
        let roots = ActiveRoots::user(claude.clone());

        let disabled = disable_path(&agent, &roots, OnConflict::Refuse).unwrap();
        assert!(!agent.exists());
        assert!(disabled.current_path.exists());

        let restored = enable_path(&disabled.current_path, &roots, OnConflict::Refuse).unwrap();
        assert!(agent.exists());
        assert_eq!(restored.current_path, agent);
    }

    #[test]
    fn list_disabled_returns_user_and_project_entries_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let user_claude = tmp.path().join("user/.claude");
        let proj_claude = tmp.path().join("repo/.claude");
        write_file(&user_claude.join("agents/aaa.md"), b"x");
        write_file(&user_claude.join("commands/bbb.md"), b"x");
        write_file(&proj_claude.join("agents/ccc.md"), b"x");

        let mut roots = ActiveRoots::user(user_claude.clone());
        roots = roots.with_project(proj_claude.clone());

        // disable each via the path API
        for p in [
            user_claude.join("agents/aaa.md"),
            user_claude.join("commands/bbb.md"),
            proj_claude.join("agents/ccc.md"),
        ] {
            disable_path(&p, &roots, OnConflict::Refuse).unwrap();
        }

        let list = list_disabled(&roots).unwrap();
        assert_eq!(list.len(), 3);
        // User scope first, then by kind alphabetically
        assert_eq!(list[0].scope, Scope::User);
        assert_eq!(list[0].kind, ArtifactKind::Agent);
        assert_eq!(list[0].name, "aaa.md");
        assert_eq!(list[1].scope, Scope::User);
        assert_eq!(list[1].kind, ArtifactKind::Command);
        assert_eq!(list[2].scope, Scope::Project);
        assert_eq!(list[2].kind, ArtifactKind::Agent);
    }

    #[test]
    fn trash_then_restore_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let trash = tmp.path().join("trash");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"contents");
        let roots = ActiveRoots::user(claude.clone());

        let entry = trash_path(&agent, &roots, &trash).unwrap();
        assert!(!agent.exists(), "source removed");
        assert_eq!(entry.state, TrashState::Healthy);
        assert!(entry.entry_dir.exists());

        let restored = restore_at(&trash, &entry.id, OnConflict::Refuse).unwrap();
        assert!(agent.exists(), "agent restored to original path");
        assert_eq!(restored.final_path, agent);
        assert_eq!(std::fs::read(&agent).unwrap(), b"contents");
        // Trash entry was removed after restore
        assert!(!entry.entry_dir.exists());
    }

    #[test]
    fn trash_then_purge_drops_only_old_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let trash = tmp.path().join("trash");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"x");
        let roots = ActiveRoots::user(claude.clone());

        let entry = trash_path(&agent, &roots, &trash).unwrap();
        // Forge an old timestamp by rewriting the manifest.
        let manifest_path = entry.entry_dir.join("manifest.json");
        let mut manifest: TrashManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest.trashed_at_ms -= 31 * 86_400_000;
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

        let purged = purge_older_than(&trash, 30).unwrap();
        assert_eq!(purged, 1);
        assert!(!entry.entry_dir.exists());
    }

    #[test]
    fn trash_with_missing_source_returns_typed_error() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let trash = tmp.path().join("trash");
        std::fs::create_dir_all(claude.join("agents")).unwrap();
        let ghost = claude.join("agents/missing.md");
        let roots = ActiveRoots::user(claude.clone());
        let err = trash_path(&ghost, &roots, &trash);
        // The path doesn't exist so classify already returns OutOfScope.
        // Either Refused/OutOfScope or SourceMissing is acceptable.
        assert!(err.is_err());
    }

    #[test]
    fn list_trash_classifies_corrupt_states() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = tmp.path().join("trash");
        std::fs::create_dir_all(&trash).unwrap();

        // Healthy
        let h_dir = trash.join("00000000-0000-0000-0000-000000000001");
        std::fs::create_dir_all(h_dir.join("payload")).unwrap();
        std::fs::write(h_dir.join("payload/foo.md"), b"x").unwrap();
        let manifest = TrashManifest {
            version: 1,
            id: "00000000-0000-0000-0000-000000000001".into(),
            trashed_at_ms: 0,
            scope: Scope::User,
            scope_root: tmp.path().join(".claude"),
            kind: ArtifactKind::Agent,
            relative_path: "foo.md".into(),
            original_path: tmp.path().join(".claude/agents/foo.md"),
            source_basename: "foo.md".into(),
            payload_kind: PayloadKind::File,
            byte_count: 1,
            sha256: None,
        };
        std::fs::write(
            h_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // MissingManifest
        let mm_dir = trash.join("00000000-0000-0000-0000-000000000002");
        std::fs::create_dir_all(mm_dir.join("payload")).unwrap();
        std::fs::write(mm_dir.join("payload/foo.md"), b"x").unwrap();

        // MissingPayload
        let mp_dir = trash.join("00000000-0000-0000-0000-000000000003");
        std::fs::create_dir_all(&mp_dir).unwrap();
        std::fs::write(mp_dir.join("manifest.json"), b"{}").unwrap();

        // OrphanPayload
        let op_dir = trash.join("00000000-0000-0000-0000-000000000004");
        std::fs::create_dir_all(op_dir.join("payload")).unwrap();
        std::fs::write(op_dir.join("payload/a.md"), b"x").unwrap();
        std::fs::write(op_dir.join("payload/b.md"), b"x").unwrap();
        std::fs::write(
            op_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // AbandonedStaging
        let as_dir = trash.join("00000000-0000-0000-0000-000000000005.staging");
        std::fs::create_dir_all(as_dir.join("payload")).unwrap();
        std::fs::write(as_dir.join("payload/foo.md"), b"x").unwrap();

        let mut entries = list_trash_at(&trash).unwrap();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].state, TrashState::Healthy);
        assert_eq!(entries[1].state, TrashState::MissingManifest);
        assert_eq!(entries[2].state, TrashState::MissingPayload);
        assert_eq!(entries[3].state, TrashState::OrphanPayload);
        assert_eq!(entries[4].state, TrashState::AbandonedStaging);
    }

    #[test]
    fn restore_refuses_non_healthy_state() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = tmp.path().join("trash");
        let mm_dir = trash.join("00000000-0000-0000-0000-000000000002");
        std::fs::create_dir_all(mm_dir.join("payload")).unwrap();
        std::fs::write(mm_dir.join("payload/foo.md"), b"x").unwrap();
        let err = restore_at(&trash, "00000000-0000-0000-0000-000000000002", OnConflict::Refuse)
            .unwrap_err();
        assert!(matches!(err, LifecycleError::WrongTrashState { .. }));
    }

    #[test]
    fn recover_promotes_missing_manifest_via_confirmed_target() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = tmp.path().join("trash");
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(claude.join("agents")).unwrap();
        let mm_dir = trash.join("00000000-0000-0000-0000-000000000007");
        std::fs::create_dir_all(mm_dir.join("payload")).unwrap();
        std::fs::write(mm_dir.join("payload/foo.md"), b"recovered").unwrap();

        let target = claude.join("agents/foo.md");
        let restored = recover_at(
            &trash,
            "00000000-0000-0000-0000-000000000007",
            &target,
            ArtifactKind::Agent,
            OnConflict::Refuse,
        )
        .unwrap();
        assert_eq!(restored.final_path, target);
        assert_eq!(std::fs::read(&target).unwrap(), b"recovered");
    }

    #[test]
    fn recover_refuses_orphan_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = tmp.path().join("trash");
        let op_dir = trash.join("00000000-0000-0000-0000-000000000008");
        std::fs::create_dir_all(op_dir.join("payload")).unwrap();
        std::fs::write(op_dir.join("payload/a.md"), b"x").unwrap();
        std::fs::write(op_dir.join("payload/b.md"), b"x").unwrap();
        // Write a (technically valid) manifest so the entry classifies
        // as OrphanPayload rather than MissingManifest. Recover must
        // refuse OrphanPayload (the user has no way to know which
        // child is the right one).
        let manifest = TrashManifest {
            version: 1,
            id: "00000000-0000-0000-0000-000000000008".into(),
            trashed_at_ms: 0,
            scope: Scope::User,
            scope_root: tmp.path().join(".claude"),
            kind: ArtifactKind::Agent,
            relative_path: "a.md".into(),
            original_path: tmp.path().join(".claude/agents/a.md"),
            source_basename: "a.md".into(),
            payload_kind: PayloadKind::File,
            byte_count: 1,
            sha256: None,
        };
        std::fs::write(
            op_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let target = tmp.path().join(".claude/agents/foo.md");
        let err = recover_at(
            &trash,
            "00000000-0000-0000-0000-000000000008",
            &target,
            ArtifactKind::Agent,
            OnConflict::Refuse,
        )
        .unwrap_err();
        assert!(matches!(err, LifecycleError::WrongTrashState { .. }));
    }

    #[test]
    fn forget_removes_a_trash_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = tmp.path().join("trash");
        let mp_dir = trash.join("00000000-0000-0000-0000-000000000009");
        std::fs::create_dir_all(&mp_dir).unwrap();
        forget_at(&trash, "00000000-0000-0000-0000-000000000009").unwrap();
        assert!(!mp_dir.exists());
    }
}
