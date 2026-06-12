//! `use` verb — switch the active Desktop account.
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the per-verb layout rationale.

use super::*;

pub async fn use_account(ctx: &AppContext, email_input: &str, no_launch: bool) -> Result<()> {
    use claudepot_core::desktop_backend;
    use claudepot_core::desktop_lock;
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::desktop_service;

    // Acquire the cross-process operation lock so CLI use_account
    // can't race with a GUI-initiated adopt/clear/switch. Codex
    // follow-up review D1: CLI switch was bypassing the flock.
    let _lock = desktop_lock::try_acquire().map_err(|e| anyhow::anyhow!("{e}"))?;

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Claude Desktop is not supported on this platform"))?;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let target = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    let current_uuid = ctx
        .store
        .active_desktop_uuid()?
        .and_then(|s| s.parse::<uuid::Uuid>().ok());

    if current_uuid == Some(target.uuid) {
        // Audit M2: in --json mode, emit a structured payload so
        // scripted callers can distinguish "already active" from an
        // empty/failed command. Previously `ctx.info` printed to
        // stderr and the command returned no stdout at all under
        // --json, which is indistinguishable from a crash to anything
        // parsing stdout as JSON.
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "already_active": true,
                    "email": email,
                })
            );
        } else {
            ctx.info(&format!("Already active: {email}"));
        }
        return Ok(());
    }

    let from_email = current_uuid
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten())
        .map(|a| a.email)
        .unwrap_or_else(|| "(none)".to_string());

    ctx.info(&format!("Switching Desktop: {from_email} → {email}"));

    // Route through desktop_service::switch so CLI gets the same
    // snapshot preflight + verbatim error message as the GUI command.
    let outcome = desktop_service::switch(platform.as_ref(), &ctx.store, target.uuid, no_launch)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "from": outcome.outgoing_email.clone().unwrap_or_else(|| "(none)".to_string()),
                "to": outcome.email,
                "launched": !no_launch,
            })
        );
    } else {
        let from_display = outcome
            .outgoing_email
            .clone()
            .unwrap_or_else(|| "(none)".to_string());
        println!("Desktop: {from_display} → {}", outcome.email);
        if no_launch {
            println!("Desktop was not relaunched (--no-launch).");
        }
    }

    Ok(())
}
