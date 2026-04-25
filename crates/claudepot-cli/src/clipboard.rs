//! Subprocess-based clipboard writer for the CLI.
//!
//! The CLI side of `claudepot session export --to clipboard` shells out
//! to whichever system tool is present:
//!
//!   * macOS    — `pbcopy`
//!   * Wayland  — `wl-copy`
//!   * X11      — `xclip -selection clipboard`
//!   * Windows  — `clip.exe` (also reachable from WSL)
//!
//! Each candidate is attempted in order; the first one that *spawns*
//! is the winner. We pipe the body through stdin and wait at most 2s
//! per child — wl-copy in particular daemonizes on success, but it
//! still closes stdin promptly; a hung child indicates a broken
//! environment, not a healthy daemon.
//!
//! This implementation lives in the CLI crate (not core) so the core
//! library stays free of subprocess concerns.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use claudepot_core::session_export_delivery::ClipboardWriter;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Per-child timeout. Long enough to absorb a slow first-launch of
/// `xclip` under pathological X servers, short enough that a wedged
/// daemon doesn't block the export forever.
const CHILD_TIMEOUT: Duration = Duration::from_secs(2);

/// Subprocess-based clipboard writer. Stateless — all candidate-tool
/// discovery happens per call so the CLI is correct in environments
/// where the user toggles between Wayland and X mid-session.
pub struct SubprocessClipboard;

#[async_trait]
impl ClipboardWriter for SubprocessClipboard {
    async fn write_text(&self, body: &str) -> Result<(), String> {
        // (program, args). Order matters — pbcopy first on macOS, etc.
        // The first one that spawns successfully is the winner; if it
        // exits non-zero we still treat that as "this clipboard is
        // broken" and surface the failure instead of falling through
        // to the next candidate (otherwise a half-configured wl-copy
        // would silently hand off to xclip on systems that have both).
        let candidates: &[(&str, &[&str])] = &[
            ("pbcopy", &[]),
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("clip.exe", &[]),
        ];
        let mut last_spawn_err: Option<String> = None;
        for (cmd, args) in candidates {
            match try_one(cmd, args, body).await {
                Ok(()) => return Ok(()),
                Err(WriteErr::Spawn(msg)) => {
                    // Tool not on PATH — try the next candidate.
                    last_spawn_err = Some(msg);
                    continue;
                }
                Err(WriteErr::Run(msg)) => {
                    // Tool exists but failed — surface that error,
                    // don't fall through.
                    return Err(msg);
                }
            }
        }
        Err(format!(
            "no clipboard command available (tried pbcopy, wl-copy, xclip, clip.exe); last spawn error: {}",
            last_spawn_err.unwrap_or_else(|| "none".to_string())
        ))
    }
}

enum WriteErr {
    Spawn(String),
    Run(String),
}

async fn try_one(cmd: &str, args: &[&str], body: &str) -> Result<(), WriteErr> {
    let mut child = match Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Err(WriteErr::Spawn(format!("{cmd}: {e}"))),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(body.as_bytes()).await {
            // Drop stdin first so the child sees EOF and exits even
            // if the write was partial.
            drop(stdin);
            let _ = child.kill().await;
            return Err(WriteErr::Run(format!("{cmd}: stdin write: {e}")));
        }
        // Closing stdin signals EOF — required for pbcopy/clip.exe
        // to exit promptly.
        drop(stdin);
    }
    let waited = match timeout(CHILD_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(WriteErr::Run(format!("{cmd}: wait: {e}"))),
        Err(_) => {
            // Timeout — the child wedged. Kill it and report.
            let _ = child.kill().await;
            return Err(WriteErr::Run(format!(
                "{cmd}: timed out after {}s",
                CHILD_TIMEOUT.as_secs()
            )));
        }
    };
    if waited.success() {
        Ok(())
    } else {
        Err(WriteErr::Run(format!(
            "{cmd}: exited with {:?}",
            waited.code()
        )))
    }
}
