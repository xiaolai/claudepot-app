//! Subprocess interactions with `git` and `gh`.
//!
//! All three callers are async via `tokio::process::Command` and
//! wrapped in a 10-second `tokio::time::timeout` apiece — a hanging
//! `gh` or `git` would otherwise wedge the tick loop, since
//! `usage_snapshot::run_tick` `await`s `tick_all` to completion
//! before the next phase. The timeout fires a kill on the child
//! and surfaces as `GhError::Timeout`, which the orchestrator
//! caches as a miss.
//!
//! 10 s is well above the p99 of either tool on a healthy network
//! and well below the 5-minute tick cadence — a complete tick of
//! 30 timed-out projects still finishes inside one tick window.

use crate::proc_utils::NoWindowExt;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time::timeout;

use super::remote::parse_github_origin;
use super::{GhError, PrInfo, PrState};

/// Per-subprocess deadline. Generous for healthy networks, strict
/// enough that a hung tool can't wedge the tick.
const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(10);

/// Get the current branch name. Returns `Ok(None)` on detached HEAD
/// (git prints an empty line and exits 0) or when the directory is
/// not a git repo (git exits non-zero). `Err(MissingCli)` only when
/// `git` itself is absent.
pub async fn current_branch(repo_root: &Path) -> Result<Option<String>, GhError> {
    let output = run_with_timeout(
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("branch")
            .arg("--show-current"),
        "git",
    )
    .await?;
    if !output.status.success() {
        // Most common failure: repo_root isn't a git repo. The
        // badge layer treats this identically to "no current
        // branch," so surface as Ok(None) rather than Err.
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

/// `true` when the repo has an `origin` remote pointing at GitHub.
/// `false` (not an error) for non-GitHub remotes or no `origin`
/// at all.
pub async fn has_github_origin(repo_root: &Path) -> Result<bool, GhError> {
    let output = run_with_timeout(
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("remote")
            .arg("get-url")
            .arg("origin"),
        "git",
    )
    .await?;
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
pub async fn view_pr(repo_root: &Path, branch: &str) -> Result<Option<PrInfo>, GhError> {
    let output = run_with_timeout(
        Command::new("gh")
            .arg("pr")
            .arg("view")
            .arg(branch)
            .arg("--json")
            .arg("number,url,state,headRefName")
            .current_dir(repo_root),
        "gh",
    )
    .await?;

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

/// Spawn the given command and await its output with a hard
/// deadline. Sets `kill_on_drop` so a timeout cleanly reaps the
/// child process when the `wait_with_output` future is dropped at
/// function exit. Maps the common error cases (missing CLI, spawn
/// failure, timeout) to typed `GhError` variants.
async fn run_with_timeout(
    cmd: &mut Command,
    tool: &'static str,
) -> Result<std::process::Output, GhError> {
    cmd.kill_on_drop(true).no_window();
    // A Dock/Finder-launched app inherits a minimal PATH that lacks
    // Homebrew — where `gh` lives for nearly every user. Enrich it
    // so `git`/`gh` resolve regardless of how Claudepot was started.
    cmd.env("PATH", crate::path_env::enriched_path());
    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GhError::MissingCli(tool)
        } else {
            GhError::Io(tool, e)
        }
    })?;
    match timeout(SUBPROCESS_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(GhError::Io(tool, e)),
        // Timeout: the wait_with_output future drops and tokio
        // reaps the child via kill_on_drop. Caller gets a typed
        // Timeout error and the orchestrator caches it as a miss.
        Err(_elapsed) => Err(GhError::Timeout(tool)),
    }
}
