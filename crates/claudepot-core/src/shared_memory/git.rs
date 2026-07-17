//! The sanctioned `git` invocations in shared memory: resolving a
//! project's current HEAD for lesson anchoring, and its repo root for
//! guard compilation.
//!
//! [`super::invalidate`] stays pure by design ("git is I/O the
//! orchestrator owns") — [`head_commit`] exists because BOTH accept
//! paths (the CLI's `lesson accept` and the GUI accept command) need
//! the identical `git -C <project> rev-parse HEAD`, and a drifted copy
//! would anchor lessons inconsistently. Callers run in blocking
//! contexts (sync CLI handlers, `spawn_blocking` closures), so these
//! are synchronous subprocesses on purpose — an async fn would force
//! a runtime handle into those closures for no gain.

use std::path::Path;

/// HEAD of the git repo containing `project`. `None` when the
/// directory isn't a repo (or `git` isn't installed) — callers treat
/// that as "the lesson stays unanchored", not as an error.
///
/// Runs `git -C <project>` so the commit belongs to the lesson's own
/// project, not wherever the calling process happened to be invoked.
pub fn head_commit(project: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// Root of the git repo containing `project` (`git -C <project>
/// rev-parse --show-toplevel`). `None` when the directory isn't a repo
/// (or `git` isn't installed) — [`super::compile`] turns that into its
/// "not a git repository" error.
pub fn repo_root(project: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn head_commit_resolves_in_a_repo() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "init"]);
        let sha = head_commit(dir).expect("HEAD in a fresh repo");
        assert_eq!(sha.len(), 40, "full sha expected, got: {sha}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn head_commit_is_none_outside_a_repo() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(head_commit(tmp.path()), None);
    }

    #[test]
    fn repo_root_resolves_the_toplevel() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        git(dir, &["init", "-q"]);
        let root = repo_root(dir).expect("root in a fresh repo");
        // Canonicalize both sides: macOS TempDirs live behind the
        // /var → /private/var symlink, and git reports the real path.
        assert_eq!(
            std::fs::canonicalize(&root).unwrap(),
            std::fs::canonicalize(dir).unwrap()
        );
    }

    #[test]
    fn repo_root_is_none_outside_a_repo() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(repo_root(tmp.path()), None);
    }
}
