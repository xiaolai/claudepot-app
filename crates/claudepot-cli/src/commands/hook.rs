//! `claudepot hook …` — verbs Claude Code invokes, not the user.
//!
//! **Every failure path here prints nothing and exits 0.** Claude Code
//! treats an absent decision as "draw the normal prompt", so silence
//! degrades to exactly the behaviour of a machine with no hook
//! installed. That makes silence the correct response to a corrupt
//! file, a disabled surface, an unparseable payload, or a phone nobody
//! is holding — and it is why this feature cannot strand a session.
//!
//! The one outcome that does *not* degrade safely is being killed by
//! CC's own hook timeout, which for a `PreToolUse` hook blocks the tool
//! call. So the wait is bounded well inside the installed timeout and
//! this process always finishes on its own terms.

use std::io::Read;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
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

/// Answer a Claude Code permission prompt from a paired device.
///
/// Returns `Ok(())` in every case; the decision, if any, is on stdout.
pub async fn permission_request_cmd() -> Result<()> {
    // Read stdin before anything else. Exiting without draining the
    // pipe would hand CC a broken-pipe write on a path where our whole
    // contract is to be invisible.
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return Ok(());
    }

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
