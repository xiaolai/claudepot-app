//! Group sessions that live under the same git repository root —
//! "worktree" grouping in the claude-devtools sense.
//!
//! CC projects are keyed by `cwd`. When the same repo is checked out
//! as multiple worktrees (`git worktree add ../feature-x`), each
//! worktree is its own CC project with its own slug, but they share an
//! upstream repo. Grouping lets the Sessions tab render them as
//! children of one repository card instead of separate unrelated rows.
//!
//! This module is filesystem-aware: it walks each session's
//! `project_path` upwards to find the enclosing `.git` directory or
//! `.git` worktree pointer file, caches the result by path, and emits
//! `RepositoryGroup`s.

use crate::session::SessionRow;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One repository, possibly containing many sessions from multiple
/// worktrees.
#[derive(Debug, Clone, Serialize)]
pub struct RepositoryGroup {
    /// Path to the `.git` parent (the main working tree of the repo).
    /// `None` means "sessions outside any git repo" — rendered in a
    /// separate bucket.
    pub repo_root: Option<PathBuf>,
    /// Human label — basename of `repo_root`, or `"(no repo)"`.
    pub label: String,
    /// Sessions attached to this repo, sorted newest-first by
    /// `last_ts`/`last_modified` (preserves whatever order the caller
    /// passed in).
    pub sessions: Vec<SessionRow>,
    /// Unique set of git branch names observed across the sessions.
    pub branches: Vec<String>,
    /// Unique worktree paths (distinct project_paths under this repo).
    pub worktree_paths: Vec<PathBuf>,
}

/// Group an input vector of rows by repository root.
///
/// Stable ordering:
///
/// * Groups are sorted by the newest `last_ts` in each group (newest
///   repo first).
/// * Sessions inside a group keep their incoming order.
pub fn group_by_repo(rows: Vec<SessionRow>) -> Vec<RepositoryGroup> {
    // Resolve each row's repo root once, cache by project_path to
    // avoid re-walking for sessions in the same directory.
    let mut cache: HashMap<String, Option<PathBuf>> = HashMap::new();
    let roots: Vec<Option<PathBuf>> = rows
        .iter()
        .map(|r| {
            cache
                .entry(r.project_path.clone())
                .or_insert_with(|| find_repo_root(Path::new(&r.project_path)))
                .clone()
        })
        .collect();

    // Bucket into groups keyed by Option<PathBuf>.
    let mut buckets: HashMap<Option<PathBuf>, RepositoryGroup> = HashMap::new();
    for (row, root) in rows.into_iter().zip(roots) {
        let entry = buckets
            .entry(root.clone())
            .or_insert_with(|| RepositoryGroup {
                label: label_for(&root),
                repo_root: root.clone(),
                sessions: Vec::new(),
                branches: Vec::new(),
                worktree_paths: Vec::new(),
            });
        if let Some(b) = &row.git_branch {
            if !entry.branches.iter().any(|existing| existing == b) {
                entry.branches.push(b.clone());
            }
        }
        let wt_path = PathBuf::from(&row.project_path);
        if !entry
            .worktree_paths
            .iter()
            .any(|existing| existing == &wt_path)
        {
            entry.worktree_paths.push(wt_path);
        }
        entry.sessions.push(row);
    }

    let mut groups: Vec<RepositoryGroup> = buckets.into_values().collect();
    groups.sort_by(|a, b| newest_ts(b).cmp(&newest_ts(a)));
    for g in &mut groups {
        g.branches.sort();
        g.worktree_paths.sort();
    }
    groups
}

/// Walk up from `start` looking for a `.git` directory or file.
/// A `.git` file means `start` is a worktree — the file points to the
/// real git dir, but the *parent* of the pointer is still the effective
/// worktree root, which is what we want for grouping.
///
/// Returns `None` when:
/// * `start` doesn't exist on disk (orphaned CC projects whose cwd was
///   deleted — we shouldn't synthesize an ancestor walk of a fake path),
/// * no ancestor contains `.git`.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    // Orphaned project paths that no longer exist must NOT be walked;
    // their `ancestors()` can fortuitously land on a live repo above,
    // which would mis-group the orphan.
    let canonical = fs::canonicalize(start).ok()?;
    for ancestor in canonical.ancestors() {
        let git = ancestor.join(".git");
        if git.exists() {
            // The actual repo root is the ancestor itself. For a
            // worktree (`.git` is a file), `ancestor` is still the
            // worktree root — but to group multiple worktrees together
            // we want the *main* repo root, which we can read out of
            // the pointer file when it's in the form
            // `gitdir: <main-repo>/.git/worktrees/<name>`.
            if git.is_file() {
                if let Some(main) = read_gitdir_pointer(&git) {
                    return Some(main);
                }
            }
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

/// Parse a `.git` pointer file to find the main repo root.
///
/// Pointer format: `gitdir: /path/to/main/.git/worktrees/<name>\n`
/// The main repo root is two levels up from the pointed-to `<name>`
/// directory (`/path/to/main`).
fn read_gitdir_pointer(git_file: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(git_file).ok()?;
    let line = content.lines().find(|l| l.starts_with("gitdir:"))?;
    let pointed = line.trim_start_matches("gitdir:").trim();
    let p = Path::new(pointed);

    // Git writes the pointer as an absolute path in most deployments,
    // but `git worktree add --relative` produces entries like
    // `gitdir: ../.git/worktrees/foo`. Resolve relative pointers
    // against the .git file's own parent — *not* the process cwd,
    // which has no relationship to the worktree.
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        match git_file.parent() {
            Some(base) => base.join(p),
            None => return None,
        }
    };

    // Expected shape: `/.../.git/worktrees/<name>` → parent is `worktrees`,
    // its parent is `.git`, its parent is the main repo root.
    let wt = abs.parent()?; // worktrees
    let git_dir = wt.parent()?; // .git
    let main_root = git_dir.parent()?; // /.../repo
    // Canonicalize so matches stay stable across symlinked tempdirs
    // (e.g. `/var` vs `/private/var` on macOS).
    Some(fs::canonicalize(main_root).unwrap_or_else(|_| main_root.to_path_buf()))
}

fn label_for(root: &Option<PathBuf>) -> String {
    match root {
        Some(p) => p
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| p.display().to_string()),
        None => "(no repo)".to_string(),
    }
}

fn newest_ts(g: &RepositoryGroup) -> i64 {
    g.sessions
        .iter()
        .map(|s| {
            s.last_ts
                .map(|t| t.timestamp_millis())
                .or_else(|| {
                    s.last_modified
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                })
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::TokenUsage;
    use chrono::{DateTime, Utc};
    use tempfile::TempDir;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse::<DateTime<Utc>>().unwrap())
    }

    fn row(project: &str, branch: Option<&str>, last: Option<DateTime<Utc>>) -> SessionRow {
        SessionRow {
            session_id: format!("sess-{project}"),
            slug: project.replace('/', "-"),
            file_path: PathBuf::new(),
            file_size_bytes: 0,
            last_modified: None,
            project_path: project.into(),
            project_from_transcript: true,
            first_ts: last,
            last_ts: last,
            event_count: 0,
            message_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            first_user_prompt: None,
            models: vec![],
            tokens: TokenUsage::default(),
            git_branch: branch.map(String::from),
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    #[test]
    fn rows_outside_git_land_in_no_repo_bucket() {
        let tmp = TempDir::new().unwrap();
        // No `.git` anywhere — all rows go into `None`.
        let p = tmp.path().to_string_lossy().to_string();
        let rows = vec![row(&p, None, None)];
        let groups = group_by_repo(rows);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].repo_root.is_none());
        assert_eq!(groups[0].label, "(no repo)");
    }

    #[test]
    fn plain_git_repo_groups_children_together() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let nested = repo.join("subdir");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();

        let rows = vec![
            row(repo.to_str().unwrap(), Some("main"), ts("2026-04-10T10:00:00Z")),
            row(nested.to_str().unwrap(), Some("main"), ts("2026-04-10T11:00:00Z")),
        ];
        let groups = group_by_repo(rows);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].sessions.len(), 2);
        assert_eq!(groups[0].branches, vec!["main".to_string()]);
        assert_eq!(groups[0].label, "repo");
    }

    #[test]
    fn worktree_pointer_links_back_to_main_repo() {
        let tmp = TempDir::new().unwrap();
        let main = tmp.path().join("repo");
        let wt = tmp.path().join("repo-wt");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(&wt).unwrap();
        // Main repo has a real .git directory.
        fs::create_dir(main.join(".git")).unwrap();
        fs::create_dir_all(main.join(".git").join("worktrees").join("feature"))
            .unwrap();
        // Worktree has a `.git` file pointing into the main repo's
        // `.git/worktrees/feature`.
        let pointer_target = main.join(".git").join("worktrees").join("feature");
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", pointer_target.display()),
        )
        .unwrap();

        let rows = vec![
            row(main.to_str().unwrap(), Some("main"), ts("2026-04-10T10:00:00Z")),
            row(wt.to_str().unwrap(), Some("feature"), ts("2026-04-10T11:00:00Z")),
        ];
        let groups = group_by_repo(rows);
        assert_eq!(groups.len(), 1, "both worktrees must share the same root");
        assert_eq!(groups[0].sessions.len(), 2);
        let mut branches = groups[0].branches.clone();
        branches.sort();
        assert_eq!(branches, vec!["feature".to_string(), "main".to_string()]);
        assert_eq!(groups[0].worktree_paths.len(), 2);
    }

    #[test]
    fn groups_are_sorted_by_newest_session() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("aRepo");
        let b = tmp.path().join("bRepo");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::create_dir(a.join(".git")).unwrap();
        fs::create_dir(b.join(".git")).unwrap();

        let rows = vec![
            row(a.to_str().unwrap(), None, ts("2026-04-10T10:00:00Z")),
            row(b.to_str().unwrap(), None, ts("2026-04-15T10:00:00Z")),
        ];
        let groups = group_by_repo(rows);
        assert_eq!(groups[0].label, "bRepo");
        assert_eq!(groups[1].label, "aRepo");
    }

    #[test]
    fn duplicate_branches_collapse() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        let rows = vec![
            row(repo.to_str().unwrap(), Some("main"), None),
            row(repo.to_str().unwrap(), Some("main"), None),
            row(repo.to_str().unwrap(), Some("feature"), None),
        ];
        let groups = group_by_repo(rows);
        assert_eq!(groups.len(), 1);
        let mut b = groups[0].branches.clone();
        b.sort();
        assert_eq!(b, vec!["feature".to_string(), "main".to_string()]);
    }

    #[test]
    fn relative_gitdir_pointer_resolves_against_the_git_file_parent() {
        let tmp = TempDir::new().unwrap();
        let main = tmp.path().join("repo");
        let wt = tmp.path().join("repo-wt");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::create_dir(main.join(".git")).unwrap();
        fs::create_dir_all(main.join(".git").join("worktrees").join("feature"))
            .unwrap();
        // Relative pointer — git worktree add --relative.
        // git_file is at `<tmp>/repo-wt/.git`, so `../repo/.git/worktrees/feature`
        // resolves back to the main repo.
        fs::write(
            wt.join(".git"),
            "gitdir: ../repo/.git/worktrees/feature\n",
        )
        .unwrap();

        let rows = vec![
            row(main.to_str().unwrap(), Some("main"), ts("2026-04-10T10:00:00Z")),
            row(wt.to_str().unwrap(), Some("feature"), ts("2026-04-10T11:00:00Z")),
        ];
        let groups = group_by_repo(rows);
        assert_eq!(
            groups.len(),
            1,
            "relative-pointer worktree must share root with its main repo"
        );
    }

    #[test]
    fn nonexistent_project_path_falls_into_no_repo_bucket() {
        let rows = vec![row("/definitely/does/not/exist/abc", None, None)];
        let groups = group_by_repo(rows);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].repo_root.is_none());
    }
}
