//! Per-project PR detection.
//!
//! Borrowed from `claude-manager`'s `gh pr view`-with-caching pattern.
//! The module asks one question — "does this repo's current branch
//! have an open PR on `origin`?" — and answers it cheaply enough that
//! the orchestrator can poll every project on each
//! `usage_snapshot::run_tick`.
//!
//! Scope: `gh` CLI path only. The reference plan also called for a
//! REST fallback using `github_token_resolve`, but in practice every
//! Claudepot user who uses PRs has `gh` installed (it's also the
//! CC-recommended CLI). A REST fallback would add two error paths,
//! a token prompt, and network surface for marginal gain. When this
//! omission becomes a bug report, the right home is a new
//! `github_pr::api` submodule alongside the existing `cli`.
//!
//! Skipped projects (no `gh`, no remote, on `main`/`master`/`trunk`,
//! detached HEAD, etc.) return `Ok(None)` — never an error. Errors
//! are reserved for cases where we tried and failed in a way the
//! caller should know about (subprocess crash, malformed JSON from
//! `gh`). The orchestrator treats both `None` and `Err` as "no
//! badge"; the distinction is for logging.

pub mod cache;
pub mod cli;
pub mod remote;

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

impl PrState {
    /// Parse the value GitHub's REST + `gh` JSON output uses.
    /// `gh pr view --json state` returns uppercase (`OPEN`); the
    /// REST API returns lowercase (`open`). Be lenient about either
    /// so a future REST fallback drops in without a parser swap.
    fn from_str_ci(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "open" => Some(Self::Open),
            "merged" => Some(Self::Merged),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    /// The head branch — what the PR is on. Useful for verifying we
    /// matched the right PR when a project has multiple open ones.
    pub head_ref_name: String,
}

#[derive(Debug, Error)]
pub enum GhError {
    /// `gh` not on PATH, or `git` not on PATH. Treated as "no badge,
    /// no problem" by the orchestrator.
    #[error("required CLI not available: {0}")]
    MissingCli(&'static str),
    /// Subprocess ran but exited non-zero. Carries stderr for logs.
    /// Common case: `gh pr view` returns 1 with "no pull requests
    /// found" — the cli module translates that to `Ok(None)` before
    /// surfacing this error.
    #[error("{tool} exited {code}: {stderr}")]
    Subprocess {
        tool: &'static str,
        code: i32,
        stderr: String,
    },
    /// JSON shape did not match what we asked for. Indicates either
    /// a `gh` version skew or an authentication error returning a
    /// non-JSON page.
    #[error("could not parse {0} output: {1}")]
    BadOutput(&'static str, #[source] serde_json::Error),
    /// I/O failure spawning a subprocess.
    #[error("io error running {0}: {1}")]
    Io(&'static str, #[source] std::io::Error),
    /// Subprocess exceeded the per-call deadline and was killed
    /// (kill_on_drop). The orchestrator caches the miss so the
    /// next tick can retry without wedging this tick.
    #[error("{0} timed out")]
    Timeout(&'static str),
}

impl GhError {
    /// Whether this error is "we genuinely tried and failed and the
    /// user might want to know" — used by the orchestrator to decide
    /// whether to log. `MissingCli` is silent; everything else logs
    /// once per cache window.
    pub fn is_noteworthy(&self) -> bool {
        !matches!(self, Self::MissingCli(_))
    }
}

/// Outcome of a single PR detection. Always carries the branch we
/// queried — the orchestrator uses it to key its cache without
/// shelling out to `git branch` a second time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectOutcome {
    /// Branch HEAD was on when we asked. Empty when HEAD is
    /// detached or the repo isn't a git repo.
    pub branch: String,
    /// The PR if one was found. `None` covers "no PR for this
    /// branch", "no GitHub remote", "trunk branch", or "detached
    /// HEAD" — every flavor of "nothing to render."
    pub pr: Option<PrInfo>,
}

/// Detect whether `repo_root`'s current branch has an open PR.
///
/// Always returns the branch (empty on detached HEAD / non-repo) so
/// the orchestrator can cache by branch without a second `git`
/// invocation. `Ok(DetectOutcome { pr: None, .. })` covers:
///   * `repo_root` is not a git repo
///   * no `origin` remote, or `origin` doesn't point at GitHub
///   * HEAD is detached
///   * the branch is a trunk-shaped name (`main`/`master`/`trunk`)
///     — by definition no PR
///   * `gh` reports no open PR for the branch
///
/// Returns `Err(GhError::MissingCli)` when `gh` or `git` isn't on
/// PATH — the orchestrator treats this identically to "no PR" from
/// the user's perspective, but the typed distinction lets the
/// orchestrator flip its global gh-absent short-circuit and skip
/// subsequent `git` calls for the rest of the session.
pub async fn detect_pr(repo_root: &Path) -> Result<DetectOutcome, GhError> {
    let branch = match cli::current_branch(repo_root).await? {
        Some(b) => b,
        // Detached HEAD or non-repo: no branch, no PR.
        None => {
            return Ok(DetectOutcome {
                branch: String::new(),
                pr: None,
            });
        }
    };
    if is_trunk_branch(&branch) {
        return Ok(DetectOutcome { branch, pr: None });
    }
    if !cli::has_github_origin(repo_root).await? {
        return Ok(DetectOutcome { branch, pr: None });
    }
    let pr = cli::view_pr(repo_root, &branch).await?;
    Ok(DetectOutcome { branch, pr })
}

/// Return `true` when the branch name is a standard trunk that
/// never has a PR pointing into it. The set is intentionally
/// hardcoded — branch names are a closed-set convention in practice.
/// A user with a non-standard trunk simply sees no badge when on
/// it, which is correct behavior (a trunk doesn't have an open PR
/// against itself).
pub fn is_trunk_branch(name: &str) -> bool {
    matches!(name, "main" | "master" | "trunk" | "develop")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_state_parses_uppercase_and_lowercase() {
        assert_eq!(PrState::from_str_ci("OPEN"), Some(PrState::Open));
        assert_eq!(PrState::from_str_ci("open"), Some(PrState::Open));
        assert_eq!(PrState::from_str_ci("MERGED"), Some(PrState::Merged));
        assert_eq!(PrState::from_str_ci("closed"), Some(PrState::Closed));
        assert_eq!(PrState::from_str_ci("foobar"), None);
    }

    #[test]
    fn trunk_branch_detection_includes_common_names() {
        for name in ["main", "master", "trunk", "develop"] {
            assert!(is_trunk_branch(name), "{name} should be trunk");
        }
        for name in ["feature/x", "release-1.0", "fix-bug", "topic/foo"] {
            assert!(!is_trunk_branch(name), "{name} should not be trunk");
        }
    }

    #[test]
    fn gh_error_missing_cli_is_silent() {
        assert!(!GhError::MissingCli("gh").is_noteworthy());
    }

    #[test]
    fn gh_error_subprocess_is_noteworthy() {
        let e = GhError::Subprocess {
            tool: "gh",
            code: 1,
            stderr: "rate limit".into(),
        };
        assert!(e.is_noteworthy());
    }
}
