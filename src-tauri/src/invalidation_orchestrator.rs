//! Bridges the pure invalidation logic to the Tauri runtime.
//!
//! Once per tick, for each project that has accepted, code-anchored
//! lessons, ask git whether the anchored files have changed since the
//! commit the lesson was accepted at. Any that changed flip to
//! `suspect` and return to the triage queue, and a `lesson-suspect`
//! event fires so the UI can surface it.
//!
//! Zero cost when nothing is anchored: the first query returns no
//! candidates and the tick ends. The `git diff` only runs for projects
//! that actually have accepted lessons with a commit anchor.
//!
//! Pure decision logic lives in
//! `claudepot_core::shared_memory::invalidate`; this file is the I/O
//! shell (open the DB, shell out to git, emit the event).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use tauri::{AppHandle, Emitter};

use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::invalidate::{anchored_claims, apply, evaluate};

/// Event emitted when one or more lessons are invalidated.
const EVENT_LESSON_SUSPECT: &str = "lesson-suspect";

pub async fn tick(app: &AppHandle) {
    let app = app.clone();
    // All of this is synchronous SQLite + subprocess work; keep it off
    // the async reactor.
    let emitted = tauri::async_runtime::spawn_blocking(move || run(&app))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "invalidation_orchestrator: join failed");
            0
        });
    if emitted > 0 {
        tracing::info!(count = emitted, "invalidation_orchestrator: lessons flagged suspect");
    }
}

/// Returns the number of lessons flipped to suspect this tick.
fn run(app: &AppHandle) -> u32 {
    let db = claudepot_core::paths::claudepot_data_dir().join("sessions.db");
    if !db.exists() {
        return 0;
    }
    let idx = match SessionIndex::open(&db) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "invalidation_orchestrator: open sessions.db failed");
            return 0;
        }
    };

    // Which projects even have accepted lessons? Sweeping every project
    // in the index would shell out to git in directories that may not be
    // repos at all. Ask the DB first.
    let projects = match projects_with_accepted_lessons(&idx) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "invalidation_orchestrator: project scan failed");
            return 0;
        }
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut total = 0u32;
    for project in projects {
        let claims = match anchored_claims(&idx, &project) {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!(project = %project, error = %e, "anchored_claims failed");
                continue;
            }
        };

        // Cache `git diff` per commit within this project — several
        // lessons often share an anchor commit, and each diff is a
        // process spawn. `RefCell` because `evaluate` takes an `Fn`, and
        // memoizing means mutating the cache from inside it.
        let diff_cache: RefCell<HashMap<String, Option<Vec<String>>>> =
            RefCell::new(HashMap::new());
        let project_ref = Path::new(&project);
        let changed_since = |commit: &str| -> Option<Vec<String>> {
            if let Some(hit) = diff_cache.borrow().get(commit) {
                return hit.clone();
            }
            let out = git_changed_since(project_ref, commit);
            diff_cache
                .borrow_mut()
                .insert(commit.to_string(), out.clone());
            out
        };

        let report = evaluate(&claims, &changed_since);
        if report.invalidated.is_empty() {
            continue;
        }
        match apply(&idx, &report, now_ms) {
            Ok(n) => {
                total += n;
                let _ = app.emit(
                    EVENT_LESSON_SUSPECT,
                    serde_json::json!({
                        "project_path": project,
                        "invalidated": report.invalidated,
                    }),
                );
            }
            Err(e) => {
                tracing::warn!(project = %project, error = %e, "apply invalidation failed");
            }
        }
    }
    total
}

fn projects_with_accepted_lessons(idx: &SessionIndex) -> Result<Vec<String>, String> {
    let claims =
        claudepot_core::shared_memory::invalidate::anchored_claims_all(idx).map_err(|e| e.to_string())?;
    let mut seen = std::collections::BTreeSet::new();
    for c in claims {
        seen.insert(c.project_path);
    }
    Ok(seen.into_iter().collect())
}

/// `git -C <project> diff --name-only <commit>`. Returns `None` when the
/// commit is not in the repo (rebase / shallow clone / not a repo at
/// all) — which the pure layer treats as "unknowable", NOT "unchanged".
fn git_changed_since(project: &Path, commit: &str) -> Option<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(project)
        // `--` terminates revision parsing so a stored commit value that
        // happens to start with `-` (a corrupt/hostile anchor) is treated
        // as a revision, never a git option.
        .args(["diff", "--name-only", commit, "--"])
        .output()
        .ok()?;
    if !out.status.success() {
        // Non-zero: the commit is unknown here, or this isn't a repo.
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    //! The pure decision logic is covered exhaustively in
    //! `claudepot_core::shared_memory::invalidate`. Here we verify the
    //! one thing that only exists at this layer: that `git_changed_since`
    //! reads a REAL repository correctly, including the "commit not in
    //! this repo" → `None` contract the pure layer depends on.
    use super::git_changed_since;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn detects_a_change_to_an_anchored_file_in_a_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("foo.rs"), "fn foo() {}\n").unwrap();
        std::fs::write(dir.join("bar.rs"), "fn bar() {}\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "seed"]);
        let anchor = git(dir, &["rev-parse", "HEAD"]);

        // Nothing changed yet → the anchor commit sees no diff.
        assert_eq!(git_changed_since(dir, &anchor), Some(vec![]));

        // Change the anchored file.
        std::fs::write(dir.join("foo.rs"), "fn foo() { todo!() }\n").unwrap();
        let changed = git_changed_since(dir, &anchor).unwrap();
        assert!(changed.iter().any(|f| f == "foo.rs"));
        assert!(!changed.iter().any(|f| f == "bar.rs"));
    }

    #[test]
    fn an_unknown_commit_returns_none_not_an_empty_diff() {
        // The whole "unknowable ≠ unchanged" contract rests on this. A
        // rebased-away anchor must read as None, so the pure layer leaves
        // the claim alone rather than falsely confirming it fresh.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("x"), "1").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "seed"]);

        assert_eq!(git_changed_since(dir, "0000000000000000000000000000000000000000"), None);
    }

    #[test]
    fn a_non_repo_directory_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(git_changed_since(tmp.path(), "abc1234"), None);
    }
}
