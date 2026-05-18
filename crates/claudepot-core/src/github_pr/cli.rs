//! Subprocess interactions with `git` and `gh`.
//!
//! All three callers (`current_branch`, `has_github_origin`,
//! `view_pr`) are synchronous; the orchestrator runs them on a
//! `tokio::task::spawn_blocking` boundary so the tick loop stays
//! non-blocking. `gh pr view` is bounded by its own per-process
//! timeout and `gh` configuration, so we don't impose a second
//! timeout here.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use super::remote::parse_github_origin;
use super::{GhError, PrInfo, PrState};

/// Get the current branch name. Returns `Ok(None)` on detached HEAD
/// (git prints an empty line and exits 0); `Err(MissingCli)` if
/// `git` itself is absent.
pub fn current_branch(repo_root: &Path) -> Result<Option<String>, GhError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("branch")
        .arg("--show-current")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::MissingCli("git")
            } else {
                GhError::Io("git", e)
            }
        })?;
    if !output.status.success() {
        // Most common failure: repo_root isn't a git repo. We
        // surface this as "no current branch" rather than an error,
        // because the badge layer treats them identically.
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

/// `true` when the repo has an `origin` remote that points at
/// GitHub. False (not an error) for non-GitHub remotes or no
/// `origin` at all.
pub fn has_github_origin(repo_root: &Path) -> Result<bool, GhError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::MissingCli("git")
            } else {
                GhError::Io("git", e)
            }
        })?;
    if !output.status.success() {
        // No `origin` configured — silent miss.
        return Ok(false);
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(parse_github_origin(&url).is_some())
}

/// `gh pr view` JSON shape. Field names match `--json
/// number,url,state,headRefName` exactly so serde decodes without
/// rename ceremony.
#[derive(Deserialize, Debug)]
struct GhPrViewOutput {
    number: u64,
    url: String,
    state: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
}

/// Run `gh pr view --json number,url,state,headRefName` against the
/// given branch. Returns `Ok(None)` for "no PR found for this
/// branch" (which `gh` reports as exit 1 with a specific stderr
/// shape — we match on that to disambiguate from a real error).
pub fn view_pr(repo_root: &Path, branch: &str) -> Result<Option<PrInfo>, GhError> {
    let output = Command::new("gh")
        .arg("pr")
        .arg("view")
        .arg(branch)
        .arg("--json")
        .arg("number,url,state,headRefName")
        .current_dir(repo_root)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::MissingCli("gh")
            } else {
                GhError::Io("gh", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // `gh` prints `no pull requests found` on stderr when the
        // branch has no open or merged PR. Treat that as "nothing
        // to show" rather than a noisy error.
        if stderr.contains("no pull requests found") || stderr.contains("no open pull requests") {
            return Ok(None);
        }
        return Err(GhError::Subprocess {
            tool: "gh",
            code: output.status.code().unwrap_or(-1),
            stderr: stderr.into_owned(),
        });
    }

    let parsed: GhPrViewOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| GhError::BadOutput("gh", e))?;
    let state = PrState::from_str_ci(&parsed.state).unwrap_or(PrState::Closed);
    Ok(Some(PrInfo {
        number: parsed.number,
        url: parsed.url,
        state,
        head_ref_name: parsed.head_ref_name,
    }))
}
