//! Disable / enable: in-place hide via move to `.disabled/<kind>/`.
//!
//! Same-volume by construction (source and destination both under the
//! same `.claude/` root). Atomicity comes from `std::fs::rename` plus
//! a per-`scope_root` advisory file lock that serializes lifecycle
//! ops within a single Claudepot process. The lock also guards
//! against the validate-then-rename race when two ops on different
//! artifacts target the same destination (collision check + rename
//! happen inside the same critical section).
//!
//! On Linux 5.5+ we additionally use `RENAME_NOREPLACE` via `renameat2`
//! so the kernel itself refuses to overwrite — a defense-in-depth
//! guard layered on top of the application lock.

use crate::artifact_lifecycle::error::{LifecycleError, Result};
use crate::artifact_lifecycle::paths::{
    disabled_target_for, enabled_target_for, ActiveRoots, ArtifactKind, PayloadKind, Scope,
    Trackable,
};
use crate::artifact_lifecycle::scope_lock;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Conflict policy applied to disable/enable destinations and
/// trash-restore destinations alike. UI defaults to `Refuse` and
/// retries with `Suffix` only after explicit user click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    Refuse,
    Suffix,
}

impl Default for OnConflict {
    fn default() -> Self {
        Self::Refuse
    }
}

/// What the caller gets back after a successful disable/enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisabledRecord {
    pub scope: Scope,
    pub scope_root: PathBuf,
    pub kind: ArtifactKind,
    pub name: String,
    pub original_path: PathBuf,
    pub current_path: PathBuf,
    pub payload_kind: PayloadKind,
}

/// Move `path` from `<root>/<kind>/...` to `<root>/.disabled/<kind>/...`.
///
/// Idempotency: if `path` already lives under `.disabled/`, returns
/// the existing record without touching the filesystem.
pub fn disable_at(
    trackable: &Trackable,
    on_conflict: OnConflict,
    _roots: &ActiveRoots,
) -> Result<DisabledRecord> {
    if trackable.already_disabled {
        // Idempotent — caller asked to disable an already-disabled
        // artifact. Surface the current state with no I/O.
        return Ok(record_from_disabled(trackable));
    }

    let source = enabled_target_for(trackable);
    if !source.exists() {
        return Err(LifecycleError::SourceMissing(source));
    }

    let target_initial = disabled_target_for(trackable);
    let _lock = scope_lock::acquire(&trackable.scope_root)?;

    let target = resolve_collision(&target_initial, on_conflict)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(LifecycleError::io("create disabled parent"))?;
    }
    rename_no_replace(&source, &target)?;

    Ok(DisabledRecord {
        scope: trackable.scope,
        scope_root: trackable.scope_root.clone(),
        kind: trackable.kind,
        name: trackable.relative_path.clone(),
        original_path: source,
        current_path: target,
        payload_kind: trackable.payload_kind,
    })
}

/// Move a `<root>/.disabled/<kind>/...` artifact back to
/// `<root>/<kind>/...`. The trackable must satisfy `already_disabled`.
pub fn enable_at(
    trackable: &Trackable,
    on_conflict: OnConflict,
    _roots: &ActiveRoots,
) -> Result<DisabledRecord> {
    if !trackable.already_disabled {
        // The caller passed an active path. No-op — the artifact is
        // already enabled.
        return Ok(record_from_active(trackable));
    }

    let source = disabled_target_for(trackable);
    if !source.exists() {
        return Err(LifecycleError::SourceMissing(source));
    }

    let target_initial = enabled_target_for(trackable);
    let _lock = scope_lock::acquire(&trackable.scope_root)?;

    let target = resolve_collision(&target_initial, on_conflict)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(LifecycleError::io("create enabled parent"))?;
    }
    rename_no_replace(&source, &target)?;

    // After enable, also try to clean up any now-empty parent dirs
    // under .disabled/ so the disabled tree doesn't accumulate
    // skeletons. Best-effort: errors are swallowed.
    cleanup_empty_parents(source.parent(), &trackable.scope_root);

    Ok(DisabledRecord {
        scope: trackable.scope,
        scope_root: trackable.scope_root.clone(),
        kind: trackable.kind,
        name: trackable.relative_path.clone(),
        original_path: target.clone(),
        current_path: target,
        payload_kind: trackable.payload_kind,
    })
}

fn record_from_disabled(t: &Trackable) -> DisabledRecord {
    DisabledRecord {
        scope: t.scope,
        scope_root: t.scope_root.clone(),
        kind: t.kind,
        name: t.relative_path.clone(),
        original_path: enabled_target_for(t),
        current_path: disabled_target_for(t),
        payload_kind: t.payload_kind,
    }
}

fn record_from_active(t: &Trackable) -> DisabledRecord {
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

/// Crate-visible alias used by the trash module's restore path so
/// the same collision policy applies everywhere.
pub(crate) fn resolve_collision_pub(
    target: &Path,
    on_conflict: OnConflict,
) -> Result<PathBuf> {
    resolve_collision(target, on_conflict)
}

/// If `target` exists, either fail (`Refuse`) or compute a unique
/// suffixed sibling (`Suffix`). Suffix iterates `-2`, `-3`, … up to
/// 999 to avoid pathological loops.
///
/// Existence check is case-aware on case-insensitive filesystems
/// (default APFS on macOS, NTFS on Windows). On those platforms a
/// suffixed candidate that case-folds to an existing sibling is
/// also rejected — otherwise `myskill-2` could shadow `MySkill-2`
/// from a tooling perspective even though the filesystem treats
/// them as the same name.
fn resolve_collision(target: &Path, on_conflict: OnConflict) -> Result<PathBuf> {
    if exists_case_aware(target) {
        return match on_conflict {
            OnConflict::Refuse => Err(LifecycleError::Conflict(target.to_path_buf())),
            OnConflict::Suffix => suffix_until_free(target),
        };
    }
    Ok(target.to_path_buf())
}

fn suffix_until_free(target: &Path) -> Result<PathBuf> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "x".into());
    let ext = target
        .extension()
        .map(|s| s.to_string_lossy().into_owned());
    for n in 2..=999 {
        let candidate = if let Some(e) = &ext {
            parent.join(format!("{stem}-{n}.{e}"))
        } else {
            parent.join(format!("{stem}-{n}"))
        };
        if !exists_case_aware(&candidate) {
            return Ok(candidate);
        }
    }
    Err(LifecycleError::Conflict(target.to_path_buf()))
}

/// Existence check that's correct on both case-sensitive (Linux,
/// case-sensitive APFS volumes) and case-insensitive filesystems
/// (macOS default APFS, Windows NTFS).
///
/// `Path::exists` already returns true for case-folded matches on
/// case-insensitive filesystems, so for the same-case target the
/// answer is correct. The extra dirent scan catches the case where
/// our generated suffix differs only in case from a real sibling
/// (`Foo-2.md` exists, we ask about `foo-2.md` on macOS) — the
/// filesystem says "exists" so the bare check is right; we still
/// run the scan to also reject `Foo-2.md` vs `foo-2.md` on the
/// case-sensitive Linux path where `exists()` would say "no".
fn exists_case_aware(target: &Path) -> bool {
    if target.exists() {
        return true;
    }
    // On case-sensitive filesystems we still want to refuse a target
    // whose case-folded name matches an existing sibling — keeps the
    // disabled tree's UX consistent across platforms (a user who
    // disables `Foo.md` on macOS shouldn't be able to disable
    // `foo.md` on Linux without an explicit rename first).
    let parent = match target.parent() {
        Some(p) => p,
        None => return false,
    };
    let want = target
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase());
    let want = match want {
        Some(s) => s,
        None => return false,
    };
    match std::fs::read_dir(parent) {
        Ok(iter) => iter
            .flatten()
            .any(|e| e.file_name().to_string_lossy().to_lowercase() == want),
        Err(_) => false,
    }
}

/// Cross-platform atomic rename that refuses to overwrite an existing
/// destination. On Linux 5.5+ this uses `renameat2(RENAME_NOREPLACE)`;
/// elsewhere it falls back to a final existence check inside the
/// scope lock (caller's responsibility — the lock is already held).
///
/// For non-Linux platforms the lock is the only guard, so this layer
/// is a no-op wrapper. The lock is held by the caller for the duration
/// of validate + rename, so this is acceptable.
///
/// Crate-visible alias used by `trash::restore_at` so the same
/// no-replace semantics apply to restore destinations.
pub(crate) fn rename_no_replace_pub(source: &Path, target: &Path) -> Result<()> {
    rename_no_replace(source, target)
}

fn rename_no_replace(source: &Path, target: &Path) -> Result<()> {
    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        const RENAME_NOREPLACE: libc::c_uint = 1;
        let src_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| LifecycleError::io("rename: bad src")(std::io::Error::other("nul byte")))?;
        let dst_c = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| LifecycleError::io("rename: bad dst")(std::io::Error::other("nul byte")))?;
        // renameat2 is Linux-specific; AT_FDCWD = -100.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                src_c.as_ptr(),
                libc::AT_FDCWD,
                dst_c.as_ptr(),
                RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EEXIST) {
            return Err(LifecycleError::Conflict(target.to_path_buf()));
        }
        // Fall through to portable rename for ENOSYS (older kernels).
        if err.raw_os_error() != Some(libc::ENOSYS) {
            return Err(LifecycleError::io("rename")(err));
        }
    }
    std::fs::rename(source, target).map_err(LifecycleError::io("rename"))
}

/// Best-effort cleanup of empty directories under `.disabled/` after
/// an enable. Walks up from `start` toward `scope_root/.disabled`
/// (the kind subdir + nested rel-path), stopping at the first non-
/// empty parent. The `.disabled/` root itself is intentionally NOT
/// removed — it holds the scope lock file (`.disabled/.lock`) which
/// future ops will need.
fn cleanup_empty_parents(start: Option<&Path>, scope_root: &Path) {
    let mut cur = match start {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let stop = scope_root.join(crate::artifact_lifecycle::paths::DISABLED_DIR);
    while cur.starts_with(&stop) && cur != stop {
        match std::fs::read_dir(&cur) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return;
                }
            }
            Err(_) => return,
        }
        let _ = std::fs::remove_dir(&cur);
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact_lifecycle::paths::classify_path;

    fn user_roots(claude: &Path) -> ActiveRoots {
        ActiveRoots::user(claude.to_path_buf())
    }

    fn write_file(p: &Path, body: &[u8]) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    fn write_skill_dir(root: &Path, name: &str) -> PathBuf {
        let dir = root.join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        dir
    }

    #[test]
    fn disable_agent_file_moves_to_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"hi");
        let t = classify_path(&agent, &user_roots(&claude)).unwrap();
        let rec = disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap();
        assert!(!agent.exists(), "source removed");
        assert!(rec.current_path.exists(), "destination present");
        assert_eq!(
            rec.current_path,
            claude.join(".disabled/agents/foo.md")
        );
    }

    #[test]
    fn disable_skill_dir_moves_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let skill = write_skill_dir(&claude, "myskill");
        let t = classify_path(&skill, &user_roots(&claude)).unwrap();
        disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap();
        assert!(!skill.exists());
        let dest = claude.join(".disabled/skills/myskill/SKILL.md");
        assert!(dest.exists());
    }

    #[test]
    fn disable_nested_command_preserves_relative_path() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let cmd = claude.join("commands/team/lint.md");
        write_file(&cmd, b"x");
        let t = classify_path(&cmd, &user_roots(&claude)).unwrap();
        disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap();
        assert!(claude.join(".disabled/commands/team/lint.md").exists());
        assert!(!cmd.exists());
    }

    #[test]
    fn disable_already_disabled_path_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let disabled_dir = claude.join(".disabled/skills/already");
        std::fs::create_dir_all(&disabled_dir).unwrap();
        std::fs::write(disabled_dir.join("SKILL.md"), b"x").unwrap();
        let t = classify_path(&disabled_dir, &user_roots(&claude)).unwrap();
        assert!(t.already_disabled);
        let rec = disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap();
        // No I/O should have happened — the disabled dir is still there
        assert!(disabled_dir.exists());
        assert_eq!(rec.current_path, disabled_dir);
    }

    #[test]
    fn disable_collision_refuses_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"new");
        // Pre-existing disabled file with the same name.
        write_file(&claude.join(".disabled/agents/foo.md"), b"old");
        let t = classify_path(&agent, &user_roots(&claude)).unwrap();
        let err = disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap_err();
        assert!(matches!(err, LifecycleError::Conflict(_)));
        // The source must remain in place when the disable refuses.
        assert!(agent.exists(), "source must not be moved on conflict refuse");
    }

    #[test]
    fn disable_collision_suffix_appends_n() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"new");
        write_file(&claude.join(".disabled/agents/foo.md"), b"old");
        let t = classify_path(&agent, &user_roots(&claude)).unwrap();
        let rec = disable_at(&t, OnConflict::Suffix, &user_roots(&claude)).unwrap();
        assert!(rec.current_path.ends_with(".disabled/agents/foo-2.md"));
    }

    #[test]
    fn enable_round_trips_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"contents");
        let roots = user_roots(&claude);

        let t = classify_path(&agent, &roots).unwrap();
        let disabled = disable_at(&t, OnConflict::Refuse, &roots).unwrap();
        assert!(!agent.exists());

        let t2 = classify_path(&disabled.current_path, &roots).unwrap();
        let restored = enable_at(&t2, OnConflict::Refuse, &roots).unwrap();
        assert!(agent.exists(), "agent restored");
        assert_eq!(restored.current_path, agent);
        assert_eq!(std::fs::read(&agent).unwrap(), b"contents");
    }

    #[test]
    fn enable_cleans_up_empty_disabled_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let cmd = claude.join("commands/team/lint.md");
        write_file(&cmd, b"x");
        let roots = user_roots(&claude);
        let t = classify_path(&cmd, &roots).unwrap();
        disable_at(&t, OnConflict::Refuse, &roots).unwrap();
        let disabled = claude.join(".disabled/commands/team/lint.md");
        let t2 = classify_path(&disabled, &roots).unwrap();
        enable_at(&t2, OnConflict::Refuse, &roots).unwrap();
        // .disabled/commands/team and .disabled/commands should be
        // cleaned away since they're empty. The .disabled/ root itself
        // remains because it holds the scope lock file (.lock).
        assert!(!claude.join(".disabled/commands").exists());
        assert!(claude.join(".disabled").exists());
        // And only the lock file should be in there.
        let leftovers: Vec<_> = std::fs::read_dir(claude.join(".disabled"))
            .unwrap()
            .filter_map(|e| e.ok().map(|d| d.file_name().to_string_lossy().into_owned()))
            .collect();
        assert_eq!(leftovers, vec![".lock".to_string()]);
    }

    #[test]
    fn disable_missing_source_returns_typed_error() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(claude.join("agents")).unwrap();
        // synthesize a Trackable that points at a non-existent file
        let t = Trackable {
            scope: Scope::User,
            scope_root: claude.clone(),
            kind: ArtifactKind::Agent,
            relative_path: "ghost.md".into(),
            payload_kind: PayloadKind::File,
            already_disabled: false,
        };
        let err = disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap_err();
        assert!(matches!(err, LifecycleError::SourceMissing(_)));
    }

    #[test]
    fn enable_missing_source_returns_typed_error() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(claude.join(".disabled/agents")).unwrap();
        let t = Trackable {
            scope: Scope::User,
            scope_root: claude.clone(),
            kind: ArtifactKind::Agent,
            relative_path: "ghost.md".into(),
            payload_kind: PayloadKind::File,
            already_disabled: true,
        };
        let err = enable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap_err();
        assert!(matches!(err, LifecycleError::SourceMissing(_)));
    }

    #[test]
    fn collision_is_case_aware_on_all_platforms() {
        // The disabled tree treats Foo.md and foo.md as the same
        // artifact, even on case-sensitive filesystems. Without
        // case-fold detection a Linux user could disable Foo.md and
        // then disable foo.md too, producing two disabled entries
        // that look like one to a macOS user pulling the same
        // .claude tree across platforms.
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent_lower = claude.join("agents/foo.md");
        write_file(&agent_lower, b"lower");
        // Pre-existing disabled entry with mixed case.
        write_file(&claude.join(".disabled/agents/Foo.md"), b"upper");
        let t = classify_path(&agent_lower, &user_roots(&claude)).unwrap();
        let err = disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap_err();
        assert!(matches!(err, LifecycleError::Conflict(_)));
        assert!(agent_lower.exists(), "source preserved on conflict");
    }

    #[test]
    fn concurrent_destination_creation_loses_to_lock() {
        // Validate-then-rename race: synthesize the destination
        // between the collision check and the rename, observe that
        // RENAME_NOREPLACE / the final-existence check protects us.
        // Without the lock + the no-replace guard, the rename would
        // silently overwrite on Unix.
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let agent = claude.join("agents/foo.md");
        write_file(&agent, b"new");
        // Pre-create the destination AFTER we'd have classified the
        // source — simulating a racing process.
        let dest = claude.join(".disabled/agents/foo.md");
        write_file(&dest, b"old");
        let t = classify_path(&agent, &user_roots(&claude)).unwrap();
        let err = disable_at(&t, OnConflict::Refuse, &user_roots(&claude)).unwrap_err();
        // We should report the conflict, not silently overwrite.
        assert!(matches!(err, LifecycleError::Conflict(_)));
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"old",
            "pre-existing destination must NOT be overwritten"
        );
        assert!(agent.exists(), "source preserved on conflict");
    }
}
