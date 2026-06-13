//! Phase 8: auto-memory dir move when git root changes.
//!
//! Per spec §3 and §4.2 P8 (verified against CC source
//! `memdir/paths.ts:203-235`), CC stores project-scoped memory at:
//!
//!   `<memoryBase>/projects/<sanitizePath(canonicalGitRoot || projectRoot)>/memory/`
//!
//! When a project dir is renamed:
//!   - If the project IS the git root (or there's no git repo): the
//!     sanitized key changes, memory dir must move.
//!   - If the project is inside a git repo and the rename doesn't
//!     change the git root (e.g. renaming a worktree or a subdirectory
//!     of a repo): memory dir key is unchanged, P8 is a no-op.
//!
//! We detect git root by walking up from the target path looking for
//! `.git` (either a dir or a file — worktree git files are plain
//! files containing a gitdir reference). This matches what
//! `git rev-parse --show-toplevel` would return, without requiring git
//! in PATH.

use crate::error::ProjectError;
use crate::project_sanitize::sanitize_path;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct MemoryMoveResult {
    pub git_root_changed: bool,
    pub memory_dir_moved: bool,
    pub old_memory_dir: Option<PathBuf>,
    pub new_memory_dir: Option<PathBuf>,
    pub snapshot_path: Option<PathBuf>,
    /// Non-fatal issues encountered during memory-dir move. Currently
    /// always empty — the existing implementation either succeeds or
    /// returns a hard Err, no half-success cases. Kept on the struct
    /// so the move pipeline's result type can accumulate warnings if
    /// we later add partial-failure modes (e.g. old-wins merge
    /// conflicts). If this field is still empty after a release or
    /// two, drop it.
    pub warnings: Vec<String>,
}

/// Walk upward from `start` looking for a `.git` entry (dir or file).
/// Returns the directory that contains it (the canonical git root).
/// Returns `None` if no `.git` is found before hitting the filesystem
/// root. Normalized via existence-preserving canonicalization.
pub fn find_canonical_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur: PathBuf = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };
    loop {
        if cur.join(".git").exists() {
            let canon = cur.canonicalize().ok()?;
            // Strip Windows verbatim prefix (`\\?\`) so the returned
            // path feeds `sanitize_path` in the same form CC uses.
            let simplified = crate::path_utils::simplify_windows_path(&canon.to_string_lossy());
            return Some(PathBuf::from(simplified));
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Move the auto-memory dir if the git root changed as a result of a
/// rename. Collision policy mirrors P4 (`--merge` / `--overwrite`).
pub fn move_memory_dir_if_needed(
    config_dir: &Path,
    old_norm: &str,
    new_norm: &str,
    merge: bool,
    overwrite: bool,
    snapshots_dir: Option<&Path>,
) -> Result<MemoryMoveResult, ProjectError> {
    let mut result = MemoryMoveResult::default();

    let old_git_root = find_canonical_git_root(Path::new(old_norm))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| old_norm.to_string());
    let new_git_root = find_canonical_git_root(Path::new(new_norm))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| new_norm.to_string());

    if old_git_root == new_git_root {
        tracing::debug!("git root unchanged; P8 no-op");
        return Ok(result);
    }
    result.git_root_changed = true;

    let old_san = sanitize_path(&old_git_root);
    let new_san = sanitize_path(&new_git_root);
    let projects_base = config_dir.join("projects");
    let old_mem = projects_base.join(&old_san).join("memory");
    let new_mem = projects_base.join(&new_san).join("memory");
    result.old_memory_dir = Some(old_mem.clone());
    result.new_memory_dir = Some(new_mem.clone());

    if !old_mem.exists() {
        tracing::debug!("no old memory dir to move; P8 no-op");
        return Ok(result);
    }

    // Ensure the NEW sanitized parent exists (it may not; the project
    // dir gets created lazily).
    if let Some(parent) = new_mem.parent() {
        fs::create_dir_all(parent).map_err(ProjectError::Io)?;
    }

    if new_mem.exists() {
        let is_empty = fs::read_dir(&new_mem)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if is_empty {
            fs::remove_dir(&new_mem).map_err(ProjectError::Io)?;
            fs::rename(&old_mem, &new_mem).map_err(ProjectError::Io)?;
            result.memory_dir_moved = true;
        } else if overwrite {
            // Destructive: snapshot the target before remove_dir_all
            // (spec §6, §4.2 P8).
            if let Some(snaps) = snapshots_dir {
                fs::create_dir_all(snaps).map_err(ProjectError::Io)?;
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let safe_san: String = new_san
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let snap = snaps.join(format!("{ts}-{safe_san}-P8.snap"));
                crate::fs_utils::copy_dir_recursive(&new_mem, &snap).map_err(ProjectError::Io)?;
                result.snapshot_path = Some(snap);
            }
            fs::remove_dir_all(&new_mem).map_err(ProjectError::Io)?;
            fs::rename(&old_mem, &new_mem).map_err(ProjectError::Io)?;
            result.memory_dir_moved = true;
        } else if merge {
            crate::project_helpers::merge_project_dirs_pub(&old_mem, &new_mem)?;
            fs::remove_dir_all(&old_mem).map_err(ProjectError::Io)?;
            result.memory_dir_moved = true;
        } else {
            // Spec §4.3 + §4.2 P8: non-empty target without explicit
            // --merge/--overwrite is a hard error, not a warning.
            return Err(ProjectError::Ambiguous(format!(
                "auto-memory dir exists at both old and new git roots \
                 ({old_mem:?} and {new_mem:?}); use --merge or --overwrite"
            )));
        }
    } else {
        fs::rename(&old_mem, &new_mem).map_err(ProjectError::Io)?;
        result.memory_dir_moved = true;
    }

    tracing::info!(
        old = ?result.old_memory_dir,
        new = ?result.new_memory_dir,
        moved = result.memory_dir_moved,
        "P8 auto-memory dir move complete"
    );
    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_utils::simplify_windows_path;

    /// Canonicalize a temp-dir path and strip Windows `\\?\` so test
    /// fixtures see the same shape `find_canonical_git_root` returns.
    fn canonical_test_path(p: &Path) -> PathBuf {
        let canon = p.canonicalize().unwrap();
        PathBuf::from(simplify_windows_path(&canon.to_string_lossy()))
    }

    #[test]
    fn test_find_canonical_git_root_finds_containing_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = canonical_test_path(tmp.path());
        fs::create_dir(repo.join(".git")).unwrap();
        let sub = repo.join("src").join("lib");
        fs::create_dir_all(&sub).unwrap();

        let root = find_canonical_git_root(&sub).unwrap();
        assert_eq!(root, repo);
    }

    #[test]
    fn test_find_canonical_git_root_no_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let p = canonical_test_path(tmp.path()).join("deep").join("path");
        fs::create_dir_all(&p).unwrap();
        // No .git anywhere above (except possibly our real repo —
        // filter by tempdir prefix).
        let root = find_canonical_git_root(&p);
        // May resolve to the repo of the workspace running the test.
        // We just ensure it doesn't panic and if it finds a root it's
        // a real filesystem path.
        if let Some(r) = root {
            assert!(r.exists());
        }
    }

    #[test]
    fn test_move_memory_dir_no_git_root_change() {
        // Both paths resolve to same git root → no-op.
        let tmp = tempfile::tempdir().unwrap();
        let base = canonical_test_path(tmp.path());
        let repo = base.join("repo");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();

        let sub1 = repo.join("sub1");
        let sub2 = repo.join("sub2");
        fs::create_dir(&sub1).unwrap();
        fs::create_dir(&sub2).unwrap();

        let result = move_memory_dir_if_needed(
            &base.join("config"),
            &sub1.to_string_lossy(),
            &sub2.to_string_lossy(),
            false,
            false,
            None,
        )
        .unwrap();
        assert!(!result.git_root_changed);
        assert!(!result.memory_dir_moved);
    }

    #[test]
    fn test_move_memory_dir_project_is_git_root() {
        let tmp = tempfile::tempdir().unwrap();
        let base = canonical_test_path(tmp.path());
        let config = base.join("config");
        fs::create_dir_all(config.join("projects")).unwrap();

        // Two independent project roots with .git directories.
        let old_root = base.join("old-project");
        let new_root = base.join("new-project");
        fs::create_dir(&old_root).unwrap();
        fs::create_dir(old_root.join(".git")).unwrap();
        fs::create_dir(&new_root).unwrap();
        fs::create_dir(new_root.join(".git")).unwrap();

        // Seed old memory dir.
        let old_san = sanitize_path(&old_root.to_string_lossy());
        let old_mem = config.join("projects").join(&old_san).join("memory");
        fs::create_dir_all(&old_mem).unwrap();
        fs::write(old_mem.join("MEMORY.md"), "history").unwrap();

        let result = move_memory_dir_if_needed(
            &config,
            &old_root.to_string_lossy(),
            &new_root.to_string_lossy(),
            false,
            false,
            None,
        )
        .unwrap();

        assert!(result.git_root_changed);
        assert!(result.memory_dir_moved);

        let new_san = sanitize_path(&new_root.to_string_lossy());
        let new_mem = config.join("projects").join(&new_san).join("memory");
        assert!(new_mem.exists());
        assert_eq!(
            fs::read_to_string(new_mem.join("MEMORY.md")).unwrap(),
            "history"
        );
        assert!(!old_mem.exists());
    }
}
