//! `claudepot hook …` — verbs Claude Code invokes, not the user.
//!
//! **Every failure path here prints nothing and exits 0.** Claude Code
//! treats an absent decision as "decide as usual", so silence degrades
//! to exactly the behaviour of a machine with no hook installed. That
//! makes silence the correct response to a corrupt file, a disabled
//! surface, an unparseable payload, or a phone nobody is holding — and
//! it is why neither verb can strand a session.
//!
//! The one outcome that does *not* degrade safely is being killed by
//! CC's own hook timeout, which for a `PreToolUse` hook blocks the tool
//! call. So every wait is bounded well inside the installed timeout and
//! each process always finishes on its own terms.
//!
//! Two verbs, two events, two lifetimes — deliberately not one:
//!
//! - [`permission_request_cmd`] answers `PermissionRequest` from a
//!   paired phone. Installed while `remote serve` runs, gated at
//!   runtime on the server's heartbeat, and it *waits*.
//! - [`pre_tool_use_cmd`] answers `PreToolUse` from a permission
//!   grant. Installed while a grant is live, decided from one file
//!   read, and it never waits. Grants use `PreToolUse` rather than
//!   sharing the first verb because in auto mode `PermissionRequest`
//!   never fires for a call the classifier approves — see
//!   `claudepot_core::permission::hook`.

use std::io::Read;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use claudepot_core::permission::{hook as grant_hook, store as grant_store};
use claudepot_core::remote::approval::{self, store, ApprovalRequest, Decision, HookInput, WAIT};

/// How often to look for a tap. Fast enough that the phone feels
/// immediate, slow enough that a two-minute wait is a few hundred
/// `stat` calls rather than a spin.
const POLL: Duration = Duration::from_millis(300);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Read the whole payload first. Exiting without draining the pipe
/// would hand CC a broken-pipe write on a path where our whole contract
/// is to be invisible.
fn read_stdin() -> Option<String> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).ok()?;
    Some(raw)
}

/// Answer a Claude Code permission prompt from a paired device.
///
/// Returns `Ok(())` in every case; the decision, if any, is on stdout.
pub async fn permission_request_cmd() -> Result<()> {
    let Some(raw) = read_stdin() else {
        return Ok(());
    };

    // The runtime half of the gate. An entry left in settings.json by a
    // crash, a hand-edit or an uninstall must do nothing at all —
    // otherwise it pauses every permission prompt on the machine for
    // two minutes each, with no phone anywhere to answer them.
    if !store::gate() {
        return Ok(());
    }

    let Ok(input) = serde_json::from_str::<HookInput>(&raw) else {
        return Ok(());
    };

    let dir = store::dir();
    let started = now_ms();
    store::sweep(&dir, started);

    let id = uuid::Uuid::new_v4().to_string();
    let request = ApprovalRequest::new(&input, id.clone(), started);
    if store::put_request(&dir, &request).is_err() {
        return Ok(());
    }

    let decision = wait_for_tap(&dir, &id).await;
    store::clear(&dir, &id);

    if let Some(decision) = decision {
        // The only bytes this command ever writes to stdout.
        println!("{}", approval::decision_output(decision));
    }
    Ok(())
}

/// Poll until someone taps or the window closes.
async fn wait_for_tap(dir: &std::path::Path, id: &str) -> Option<Decision> {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(answer) = store::take_decision(dir, id) {
            return Some(answer.decision);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Answer a Claude Code `PreToolUse` check from a permission grant.
///
/// One file read, no waiting, and the file is never written or moved:
/// this runs inside every tool call while a grant is live. Prints the
/// allow decision when a live grant covers the session's working
/// directory; otherwise nothing. Returns `Ok(())` in every case.
pub fn pre_tool_use_cmd() -> Result<()> {
    let Some(raw) = read_stdin() else {
        return Ok(());
    };
    let Ok(input) = serde_json::from_str::<grant_hook::PreToolUseInput>(&raw) else {
        return Ok(());
    };
    let Some(file) = grant_hook::load_readonly(&grant_store::grants_path()) else {
        return Ok(());
    };
    if grant_hook::covering_grant_resolved(&file, &input.cwd, chrono::Utc::now()).is_some() {
        // The only bytes this command ever writes to stdout.
        println!("{}", grant_hook::decision_output());
    }
    Ok(())
}
