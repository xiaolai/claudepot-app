//! `--include-worktree` — per-project worktree state.
//!
//! See `dev-docs/project-migrate-spec.md` §3.2 (`project-scoped/`),
//! §4 Bucket A (`projectSettings`), and §4 settings tier policy.
//!
//! What travels under `--include-worktree`:
//!   - `<cwd>/.claude/**` — project-scoped settings, agents, skills,
//!     commands, hooks (subject to trust gate).
//!   - `<cwd>/CLAUDE.md` — project-level user prefs.
//!
//! What is **always excluded** (even with `--include-worktree`):
//!   - `<cwd>/.claude/settings.local.json` — local by name; spec
//!     §4 settings tier table.
//!   - `<cwd>/.claude/managed-settings.json` and `managed-settings.d/`
//!     — org-policy controlled.
//!
//! Layout inside the bundle:
//!   `projects/<id>/project-scoped/<rel>` — mirrors the on-disk shape
//!   under `<cwd>` for the carried entries.

use crate::migrate::bundle::BundleWriter;
use crate::migrate::error::MigrateError;
use std::fs;
use std::path::{Path, PathBuf};

/// Return the bundle-relative prefix where project-scoped content
/// lives. Single source of truth for export and import.
pub fn project_scoped_prefix(project_id: &str) -> String {
    format!("projects/{project_id}/project-scoped")
}

/// Filenames excluded from project-scoped bundling. Spec §4 settings
/// tier table — these are always local, never travel.
pub const EXCLUDED_BASENAMES: &[&str] =
    &["settings.local.json", "managed-settings.json"];

/// Excluded directory prefixes (relative to `<cwd>/.claude/`).
pub const EXCLUDED_DIRS: &[&str] = &["managed-settings.d"];

/// Walk `<cwd>` and append the project-scoped surfaces to the bundle.
/// Returns the count of files appended (mostly for the export
/// receipt's `file_count`).
pub fn append_worktree(
    cwd: &Path,
    project_id: &str,
    writer: &mut BundleWriter,
) -> Result<usize, MigrateError> {
    let mut count = 0;
    let prefix = project_scoped_prefix(project_id);

    // <cwd>/CLAUDE.md
    let claude_md = cwd.join("CLAUDE.md");
    if claude_md.exists() && claude_md.is_file() {
        writer.append_file(&format!("{prefix}/CLAUDE.md"), &claude_md, None)?;
        count += 1;
    }

    // <cwd>/.claude/** with exclusions.
    let dot_claude = cwd.join(".claude");
    if dot_claude.exists() && dot_claude.is_dir() {
        let prefix_claude = format!("{prefix}/.claude");
        count += walk_with_exclusions(&dot_claude, &dot_claude, &prefix_claude, writer)?;
    }
    Ok(count)
}

fn walk_with_exclusions(
    root: &Path,
    base: &Path,
    bundle_prefix: &str,
    writer: &mut BundleWriter,
) -> Result<usize, MigrateError> {
    let mut count = 0;
    for entry in fs::read_dir(root).map_err(MigrateError::from)? {
        let entry = entry.map_err(MigrateError::from)?;
        let ft = entry.file_type().map_err(MigrateError::from)?;
        let path = entry.path();
        if ft.is_symlink() {
            return Err(MigrateError::IntegrityViolation(format!(
                "symlink in worktree content: {}",
                path.display()
            )));
        }
        let rel = path
            .strip_prefix(base)
            .map_err(|e| MigrateError::Io(std::io::Error::other(format!("strip: {e}"))))?
            .to_string_lossy()
            .replace('\\', "/");
        if is_excluded(&rel) {
            continue;
        }
        if ft.is_dir() {
            count += walk_with_exclusions(&path, base, bundle_prefix, writer)?;
        } else if ft.is_file() {
            let bp = format!("{bundle_prefix}/{rel}");
            writer.append_file(&bp, &path, None)?;
            count += 1;
        }
    }
    Ok(count)
}

fn is_excluded(rel: &str) -> bool {
    let basename = rel.rsplit('/').next().unwrap_or(rel);
    if EXCLUDED_BASENAMES.contains(&basename) {
        return true;
    }
    for prefix in EXCLUDED_DIRS {
        if rel == *prefix || rel.starts_with(&format!("{prefix}/")) {
            return true;
        }
    }
    false
}

/// Apply worktree content to the target cwd. The cwd must exist (the
/// user is expected to have cloned the project's git repo at the
/// target before importing). Files land verbatim; collisions write
/// `<name>.imported` next to the existing file (same policy as
/// global content).
pub fn apply_worktree(
    staging_project_root: &Path,
    target_cwd: &Path,
) -> Result<Vec<WorktreeApplyStep>, MigrateError> {
    let bundle_root = staging_project_root.join("project-scoped");
    if !bundle_root.exists() {
        return Ok(Vec::new());
    }
    let mut steps = Vec::new();
    if !target_cwd.exists() {
        return Err(MigrateError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "target cwd {} does not exist — the user must clone or \
                 create the project tree before --include-worktree import",
                target_cwd.display()
            ),
        )));
    }

    let mut stack: Vec<PathBuf> = vec![bundle_root.clone()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).map_err(MigrateError::from)? {
            let entry = entry.map_err(MigrateError::from)?;
            let ft = entry.file_type().map_err(MigrateError::from)?;
            let p = entry.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let rel = p
                .strip_prefix(&bundle_root)
                .map_err(|e| MigrateError::Io(std::io::Error::other(format!("strip: {e}"))))?
                .to_string_lossy()
                .replace('\\', "/");
            // Defense-in-depth: even if a tampered bundle squeezed
            // through the bundle reader, we re-check the exclusions
            // here. A `settings.local.json` in the bundle stays out.
            if is_excluded(&rel) {
                continue;
            }
            let target = target_cwd.join(&rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(MigrateError::from)?;
            }
            let kind = if target.exists() {
                let cur = fs::read(&target).map_err(MigrateError::from)?;
                let new = fs::read(&p).map_err(MigrateError::from)?;
                if cur == new {
                    WorktreeApplyKind::SkippedIdentical
                } else {
                    let imported = imported_sibling(&target);
                    fs::copy(&p, &imported).map_err(MigrateError::from)?;
                    steps.push(WorktreeApplyStep {
                        after: imported.to_string_lossy().to_string(),
                        kind: WorktreeApplyKind::SideBySide,
                    });
                    continue;
                }
            } else {
                fs::copy(&p, &target).map_err(MigrateError::from)?;
                WorktreeApplyKind::Created
            };
            if kind == WorktreeApplyKind::SkippedIdentical {
                continue;
            }
            steps.push(WorktreeApplyStep {
                after: target.to_string_lossy().to_string(),
                kind,
            });
        }
    }
    Ok(steps)
}

#[derive(Debug, Clone)]
pub struct WorktreeApplyStep {
    pub after: String,
    pub kind: WorktreeApplyKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeApplyKind {
    Created,
    SideBySide,
    SkippedIdentical,
}

fn imported_sibling(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = target
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    parent.join(format!("{stem}.imported{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::bundle::{BundleReader, BundleWriter};
    use crate::migrate::manifest::{BundleManifest, ExportFlags, SCHEMA_VERSION};

    fn fixture_manifest() -> BundleManifest {
        BundleManifest {
            schema_version: SCHEMA_VERSION,
            claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
            cc_version: None,
            created_at: "2026-04-27T00:00:00Z".to_string(),
            source_os: "macos".to_string(),
            source_arch: "aarch64".to_string(),
            host_identity: "ab".repeat(32),
            source_home: "/Users/joker".to_string(),
            source_claude_config_dir: "/Users/joker/.claude".to_string(),
            projects: vec![],
            flags: ExportFlags {
                include_worktree: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn append_worktree_excludes_local_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().join("p");
        fs::create_dir_all(cwd.join(".claude")).unwrap();
        fs::write(cwd.join(".claude/settings.json"), "{}").unwrap();
        fs::write(cwd.join(".claude/settings.local.json"), "{\"secret\":1}").unwrap();
        fs::write(
            cwd.join(".claude/managed-settings.json"),
            "{\"policy\":1}",
        )
        .unwrap();
        fs::write(cwd.join("CLAUDE.md"), "# project prefs\n").unwrap();

        let bundle_path = tmp.path().join("w.tar.zst");
        let mut w = BundleWriter::create(&bundle_path).unwrap();
        let n = append_worktree(&cwd, "abc", &mut w).unwrap();
        w.finalize(&fixture_manifest()).unwrap();

        // CLAUDE.md + settings.json = 2 files; the local + managed
        // are excluded.
        assert_eq!(n, 2);

        let r = BundleReader::open(&bundle_path).unwrap();
        assert_eq!(
            r.read_entry("projects/abc/project-scoped/CLAUDE.md").unwrap(),
            b"# project prefs\n"
        );
        assert!(r
            .read_entry("projects/abc/project-scoped/.claude/settings.json")
            .is_ok());
        // Local settings excluded.
        let err = r.read_entry("projects/abc/project-scoped/.claude/settings.local.json");
        assert!(err.is_err());
    }

    #[test]
    fn apply_worktree_creates_files_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let staged_root = tmp.path().join("staged/projects/abc");
        fs::create_dir_all(staged_root.join("project-scoped/.claude")).unwrap();
        fs::write(
            staged_root.join("project-scoped/CLAUDE.md"),
            "# from bundle\n",
        )
        .unwrap();
        fs::write(
            staged_root.join("project-scoped/.claude/settings.json"),
            "{}",
        )
        .unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let steps = apply_worktree(&staged_root, &target).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(target.join("CLAUDE.md").exists());
        assert!(target.join(".claude/settings.json").exists());
    }

    #[test]
    fn apply_worktree_writes_side_by_side_for_differing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let staged_root = tmp.path().join("staged/projects/abc");
        fs::create_dir_all(staged_root.join("project-scoped")).unwrap();
        fs::write(staged_root.join("project-scoped/CLAUDE.md"), "from bundle").unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("CLAUDE.md"), "from target").unwrap();
        let steps = apply_worktree(&staged_root, &target).unwrap();
        let side = steps
            .iter()
            .find(|s| s.kind == WorktreeApplyKind::SideBySide)
            .unwrap();
        assert!(side.after.ends_with("CLAUDE.imported.md"));
        // Original target untouched.
        assert_eq!(
            fs::read_to_string(target.join("CLAUDE.md")).unwrap(),
            "from target"
        );
    }

    #[test]
    fn apply_worktree_refuses_when_target_cwd_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let staged_root = tmp.path().join("staged/projects/abc");
        fs::create_dir_all(staged_root.join("project-scoped")).unwrap();
        fs::write(staged_root.join("project-scoped/CLAUDE.md"), "x").unwrap();
        let target = tmp.path().join("never-existed");
        let err = apply_worktree(&staged_root, &target).unwrap_err();
        match err {
            MigrateError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn excluded_paths_classifier() {
        assert!(is_excluded("settings.local.json"));
        assert!(is_excluded(".claude/settings.local.json"));
        assert!(is_excluded("managed-settings.json"));
        assert!(is_excluded("managed-settings.d"));
        assert!(is_excluded("managed-settings.d/foo.json"));
        assert!(!is_excluded("settings.json"));
        assert!(!is_excluded("CLAUDE.md"));
    }
}
