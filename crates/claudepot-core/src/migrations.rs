//! One-time data-layout migrations.
//!
//! Every function here must be idempotent — safe to call on every
//! startup, safe to interleave with partial prior runs. The invariant:
//! after the migration returns Ok, the canonical location holds the
//! data; the legacy location is either absent or empty.

use crate::paths;
use std::fs;
use std::io;
use std::path::Path;

/// Move the repair tree from its legacy home at
/// `<claude_config_dir>/claudepot/` to the consolidated
/// `<claudepot_data_dir>/repair/`.
///
/// Cases handled:
/// * legacy absent → no-op
/// * target absent → atomic rename of the whole tree (same filesystem)
/// * target absent, rename fails with EXDEV → copy + remove (cross-FS)
/// * target present → merge children from legacy into target, preserving
///   target's copy on conflict (partial prior migration is resolved
///   without data loss)
///
/// Does NOT delete a non-empty legacy root — anything the merge left
/// behind stays on disk for the user to inspect rather than being
/// silently discarded.
pub fn migrate_repair_tree() -> io::Result<()> {
    let legacy = paths::claude_config_dir().join("claudepot");
    let target = paths::claudepot_repair_dir();

    if !legacy.exists() {
        return Ok(());
    }

    if !target.exists() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::rename(&legacy, &target) {
            Ok(()) => {
                tracing::info!(
                    src = %legacy.display(),
                    dst = %target.display(),
                    "migrated repair tree"
                );
                return Ok(());
            }
            // EXDEV (18 on Linux/macOS) → cross-filesystem rename not
            // supported; fall through to copy + delete.
            Err(e) if e.raw_os_error() == Some(18) => {
                tracing::info!(
                    "cross-filesystem migration, copying then removing"
                );
                copy_dir_all(&legacy, &target)?;
                fs::remove_dir_all(&legacy)?;
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }

    // Target exists — resolve against partial prior runs by merging
    // children. Files already present in target win.
    tracing::warn!(
        src = %legacy.display(),
        dst = %target.display(),
        "partial prior migration detected, merging"
    );
    merge_dir_into(&legacy, &target)?;

    // Tidy up: if the legacy root is now empty, remove it. Otherwise
    // leave it alone — something collided and the user should look.
    if let Ok(mut it) = fs::read_dir(&legacy) {
        if it.next().is_none() {
            let _ = fs::remove_dir(&legacy);
        }
    }

    Ok(())
}

/// Recursively copy `src` into `dst`. Used only as the cross-filesystem
/// fallback path for the repair-tree migration.
fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Move every child of `src` into `dst` via `rename`. Children already
/// present in `dst` are skipped — target's existing copy wins, the
/// legacy copy is left behind for manual inspection.
fn merge_dir_into(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if to.exists() {
            continue;
        }
        if let Err(e) = fs::rename(&from, &to) {
            if e.raw_os_error() == Some(18) {
                // Cross-filesystem again — copy this child and remove
                // the source. Recurse into dirs; copy files.
                let ty = entry.file_type()?;
                if ty.is_dir() {
                    copy_dir_all(&from, &to)?;
                    fs::remove_dir_all(&from)?;
                } else {
                    fs::copy(&from, &to)?;
                    fs::remove_file(&from)?;
                }
            } else {
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{lock_data_dir, setup_test_data_dir};

    /// Set `CLAUDE_CONFIG_DIR` alongside `CLAUDEPOT_DATA_DIR` to fully
    /// isolate the migration target pair. Returns both roots so the
    /// caller can assert on file state.
    fn isolated_roots() -> (tempfile::TempDir, tempfile::TempDir) {
        let claude = tempfile::tempdir().unwrap();
        let claudepot = setup_test_data_dir();
        std::env::set_var("CLAUDE_CONFIG_DIR", claude.path());
        (claude, claudepot)
    }

    #[test]
    fn no_op_when_legacy_absent() {
        let _lock = lock_data_dir();
        let (_claude, claudepot) = isolated_roots();
        migrate_repair_tree().unwrap();
        // Target was not materialized since there was nothing to move.
        assert!(!claudepot.path().join("repair").exists());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn moves_legacy_into_target_when_target_absent() {
        let _lock = lock_data_dir();
        let (claude, claudepot) = isolated_roots();

        // Seed a journal, a lock, and a snapshot in the legacy layout.
        let legacy = claude.path().join("claudepot");
        fs::create_dir_all(legacy.join("journals")).unwrap();
        fs::create_dir_all(legacy.join("locks")).unwrap();
        fs::create_dir_all(legacy.join("snapshots")).unwrap();
        fs::write(legacy.join("journals").join("move-1.json"), "j").unwrap();
        fs::write(legacy.join("locks").join("foo.lock"), "l").unwrap();
        fs::write(legacy.join("snapshots").join("snap.json"), "s").unwrap();

        migrate_repair_tree().unwrap();

        let target = claudepot.path().join("repair");
        assert!(!legacy.exists(), "legacy root should be gone");
        assert_eq!(
            fs::read_to_string(target.join("journals").join("move-1.json"))
                .unwrap(),
            "j"
        );
        assert_eq!(
            fs::read_to_string(target.join("locks").join("foo.lock")).unwrap(),
            "l"
        );
        assert_eq!(
            fs::read_to_string(target.join("snapshots").join("snap.json"))
                .unwrap(),
            "s"
        );

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn merges_when_both_exist_preserving_target_on_conflict() {
        let _lock = lock_data_dir();
        let (claude, claudepot) = isolated_roots();

        let legacy = claude.path().join("claudepot");
        let target = claudepot.path().join("repair");
        fs::create_dir_all(legacy.join("journals")).unwrap();
        fs::create_dir_all(target.join("journals")).unwrap();

        // Both sides have a journal named the same but with different
        // contents. Target's version must survive.
        fs::write(legacy.join("journals").join("m.json"), "legacy").unwrap();
        fs::write(target.join("journals").join("m.json"), "target").unwrap();

        // Only-in-legacy lock should move over.
        fs::create_dir_all(legacy.join("locks")).unwrap();
        fs::write(legacy.join("locks").join("only.lock"), "only").unwrap();

        migrate_repair_tree().unwrap();

        assert_eq!(
            fs::read_to_string(target.join("journals").join("m.json")).unwrap(),
            "target",
            "target's copy must win on conflict"
        );
        assert_eq!(
            fs::read_to_string(target.join("locks").join("only.lock"))
                .unwrap(),
            "only"
        );
        // Legacy root kept because "journals" subdir still holds the
        // unmerged conflict.
        assert!(legacy.exists());

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn is_idempotent() {
        let _lock = lock_data_dir();
        let (claude, claudepot) = isolated_roots();

        let legacy = claude.path().join("claudepot");
        fs::create_dir_all(legacy.join("journals")).unwrap();
        fs::write(legacy.join("journals").join("m.json"), "x").unwrap();

        migrate_repair_tree().unwrap();
        migrate_repair_tree().unwrap(); // second call is a no-op

        let target = claudepot.path().join("repair");
        assert_eq!(
            fs::read_to_string(target.join("journals").join("m.json")).unwrap(),
            "x"
        );
        assert!(!legacy.exists());

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
}
