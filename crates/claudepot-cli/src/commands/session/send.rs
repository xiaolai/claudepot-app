//! `session live` and `session send` — addressing a *running* session.
//!
//! Sub-module of `commands/session.rs`; see that file's header for the
//! verb-group rationale. Both handlers are thin wrappers over
//! `claudepot_core::peer` — target resolution and outcome
//! classification are core's, per `.claude/rules/architecture.md`.

use super::*;
use claudepot_core::peer::{self, Outcome, Priority};

fn priority_from(s: &str) -> Priority {
    match s {
        "now" => Priority::Now,
        "later" => Priority::Later,
        _ => Priority::Next,
    }
}

pub fn live_cmd(ctx: &AppContext) -> Result<()> {
    let dir = claudepot_core::session_live::registry::default_sessions_dir();
    let sessions = peer::list_addressable(&dir).context("list addressable sessions")?;

    if ctx.json {
        let rows: Vec<_> = sessions
            .iter()
            .map(|a| {
                serde_json::json!({
                    "pid": a.record.pid,
                    "session_id": a.record.session_id,
                    "name": a.record.name,
                    "cwd": a.record.cwd,
                    "kind": a.record.kind,
                    "status": a.record.status,
                })
            })
            .collect();
        print_json(&rows)?;
        return Ok(());
    }

    if sessions.is_empty() {
        println!("No running session is addressable.");
        println!(
            "A session exposes an inbox only while Claude Code's \
             cross-session messaging is enabled for it."
        );
        return Ok(());
    }

    for a in &sessions {
        // Name first: it is what a human will type. The short id is the
        // fallback for a session CC could not derive a name for.
        let label = a.name().unwrap_or_else(|| a.short_id());
        println!(
            "{:<24} {:<9} pid {:<7} {}",
            label,
            a.record.status.as_deref().unwrap_or("-"),
            a.record.pid,
            a.record.cwd
        );
    }
    println!("\n{} addressable session(s).", sessions.len());
    Ok(())
}

pub async fn send_cmd(
    ctx: &AppContext,
    target: &str,
    prompt: &str,
    priority: &str,
    wait_secs: u64,
) -> Result<()> {
    let sessions_dir = claudepot_core::session_live::registry::default_sessions_dir();
    let config_dir = paths::claude_config_dir();

    // Close an expired remote-control window before doing anything
    // else. The GUI orchestrator is the primary reconciler, but it may
    // not have run for days; every entry point into this feature owes
    // the deadline the same respect.
    if let Err(e) = claudepot_core::peer::inbound::tick(chrono::Utc::now()) {
        ctx.info(&format!(
            "warning: could not reconcile the inbound window: {e}"
        ));
    }

    let candidates = peer::list_addressable(&sessions_dir).context("list addressable sessions")?;
    let chosen = peer::resolve(&candidates, target)?;
    let peer_target = peer::PeerTarget::from_record(&chosen.record)?;
    let session_id = peer_target.session_id.clone();

    // Snapshot the transcript BEFORE sending, so `--wait` can only ever
    // classify what this send produced.
    let watch = peer::begin_watch(&config_dir, &session_id);

    ctx.info(&format!(
        "Sending to {} (pid {})…",
        chosen.name().unwrap_or_else(|| chosen.short_id()),
        chosen.record.pid
    ));

    let handoff =
        peer::send_prompt(&peer_target, &sessions_dir, prompt, priority_from(priority)).await?;

    let outcome = if wait_secs > 0 {
        Some(
            peer::await_outcome(
                &config_dir,
                &watch,
                &session_id,
                &handoff.uuid,
                std::time::Duration::from_secs(wait_secs),
            )
            .await?,
        )
    } else {
        None
    };

    if ctx.json {
        print_json(&serde_json::json!({
            "pid": handoff.pid,
            "session_id": handoff.session_id,
            "uuid": handoff.uuid,
            "outcome": match outcome {
                Some(Outcome::Delivered) => "delivered",
                Some(Outcome::Held) => "held",
                Some(Outcome::Undetermined) => "undetermined",
                // Never "sent": the caller did not ask us to look, so we
                // do not know. Reporting success here would be a guess.
                None => "handed_off",
            },
        }))?;
        return Ok(());
    }

    match outcome {
        Some(Outcome::Delivered) => {
            println!(
                "Delivered — the session took it as a turn. (uuid {})",
                handoff.uuid
            );
        }
        Some(Outcome::Held) => {
            println!("Held — waiting for approval in that session. Claude has NOT seen it.");
            println!(
                "  Approve it there, or start the session with \
                 crossSessionInbound=\"accept\" to deliver without a prompt."
            );
        }
        Some(Outcome::Undetermined) => {
            println!(
                "Handed off, but nothing conclusive in {wait_secs}s — the session may be busy."
            );
            println!("  Check the transcript for uuid {}.", handoff.uuid);
        }
        None => {
            println!("Handed off to pid {} (uuid {}).", handoff.pid, handoff.uuid);
            println!(
                "  This does not mean delivered: the session may hold it for \
                 approval. Use --wait to find out."
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------
// `session inbound` — the time-boxed remote-control window.
// ---------------------------------------------------------------

use claudepot_core::peer::inbound::{self, Decision};

fn describe(d: &Decision) -> String {
    match d {
        Decision::Idle => "closed — peer messages are held for your approval".into(),
        Decision::Active { remaining_secs } => {
            let m = remaining_secs / 60;
            format!("OPEN — peer messages deliver without asking ({m}m left)")
        }
        Decision::Revert { .. } => "expired — closing now".into(),
        Decision::Superseded { observed } => {
            format!("changed by hand since the grant ({observed:?}) — record dropped")
        }
    }
}

pub fn inbound_status_cmd(ctx: &AppContext) -> Result<()> {
    let now = chrono::Utc::now();
    // Reconcile before reporting: a window whose deadline passed while
    // Claudepot was closed must not be reported as open.
    let decision = inbound::tick(now)?;
    if ctx.json {
        print_json(&serde_json::json!({
            "state": match &decision {
                Decision::Idle => "closed",
                Decision::Active { .. } => "open",
                Decision::Revert { .. } => "expired",
                Decision::Superseded { .. } => "superseded",
            },
            "remaining_secs": match &decision {
                Decision::Active { remaining_secs } => Some(*remaining_secs),
                _ => None,
            },
        }))?;
        return Ok(());
    }
    println!("Remote-control window: {}", describe(&decision));
    Ok(())
}

pub fn inbound_grant_cmd(ctx: &AppContext, duration: &str, reason: Option<&str>) -> Result<()> {
    let std_dur = parse_duration(duration)?;
    let chrono_dur = chrono::Duration::from_std(std_dur)
        .with_context(|| format!("duration out of range: {duration}"))?;

    let grant = inbound::open(chrono_dur, reason.map(str::to_string), chrono::Utc::now())?;

    if ctx.json {
        print_json(&serde_json::json!({
            "granted": grant.granted.as_wire(),
            "previous": grant.previous.map(|p| p.as_wire()),
            "granted_at": grant.granted_at.to_rfc3339(),
            "expires_at": grant.expires_at.to_rfc3339(),
        }))?;
        return Ok(());
    }

    println!(
        "Remote-control window OPEN until {}.",
        grant.expires_at.to_rfc3339()
    );
    // Say the cost out loud every time. This is machine-wide by
    // necessity — Claude Code only honours `accept` from user scope —
    // so the deadline is the only thing containing it.
    println!(
        "  While open, ANY local process that can reach a session's socket \
         can drive it without asking you."
    );
    println!("  Claudepot reverts this automatically. `session inbound revoke` closes it now.");
    Ok(())
}

pub fn inbound_revoke_cmd(ctx: &AppContext) -> Result<()> {
    let revoked = inbound::revoke(chrono::Utc::now())?;
    if ctx.json {
        print_json(&serde_json::json!({ "revoked": revoked.is_some() }))?;
        return Ok(());
    }
    match revoked {
        Some(_) => println!("Window closed. Peer messages are held for approval again."),
        None => println!("No window was open."),
    }
    Ok(())
}
