//! `use` verb — switch the active CLI account (the swap), plus the
//! post-swap advisory classification it prints.
//!
//! Sub-module of `commands/cli_ops.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;
use claudepot_core::error::SwapError;

pub async fn use_account(
    ctx: &AppContext,
    email_input: &str,
    no_refresh: bool,
    force: bool,
) -> Result<()> {
    use claudepot_core::cli_backend;
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::account_service;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let target = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    // Reconcile DB's active_cli pointer with CC's actual keychain state
    // BEFORE checking "already active". Otherwise, if a running CC
    // process refreshed its token and reverted a prior swap, our DB
    // still thinks the old target is active — and this command would
    // falsely report "Already active" without actually fixing the
    // keychain. Best-effort; network/profile failures fall through.
    if let Err(e) = account_service::sync_from_current_cc(&ctx.store).await {
        if !ctx.quiet {
            eprintln!("\u{26a0}  Couldn't verify CC state ({e}); proceeding with DB view.");
        }
    }

    let current_uuid = ctx
        .store
        .active_cli_uuid()?
        .and_then(|s| s.parse::<uuid::Uuid>().ok());

    if current_uuid == Some(target.uuid) {
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({"already_active": true, "email": email})
            );
        } else {
            ctx.info(&format!("Already active: {email}"));
        }
        return Ok(());
    }

    let platform = cli_backend::create_platform();

    ctx.info(&format!("Switching CLI to {email}..."));
    let refresher = cli_backend::swap::DefaultRefresher;
    let fetcher = cli_backend::swap::DefaultProfileFetcher;
    if force {
        cli_backend::swap::switch_force(
            &ctx.store,
            current_uuid,
            target.uuid,
            platform.as_ref(),
            !no_refresh,
            &refresher,
            &fetcher,
        )
        .await
        .map_err(annotate_swap_err)?;
    } else {
        cli_backend::swap::switch(
            &ctx.store,
            current_uuid,
            target.uuid,
            platform.as_ref(),
            !no_refresh,
            &refresher,
            &fetcher,
        )
        .await
        .map_err(annotate_swap_err)?;
    }

    let from = current_uuid
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten())
        .map(|a| a.email)
        .unwrap_or_else(|| "(none)".to_string());

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "from": from,
                "to": email,
            })
        );
    } else if !ctx.quiet {
        println!("CLI: {from} → {email}");
        // Ask the core whether a CC process is alive only once, then
        // decide what to print. Skipping the probe entirely on non-force
        // paths (the old behaviour) meant we printed the "running
        // claude processes will continue" note even when no CC was
        // running — misleading.
        let cc_running = claudepot_core::cli_backend::swap::is_cc_process_running_public().await;
        match swap_completion_note(force, cc_running) {
            SwapNote::ForceWithRunning => {
                eprintln!();
                eprintln!(
                    "\u{26a0}  Warning: Claude Code is running. Restart it to apply this swap cleanly."
                );
                eprintln!();
                eprintln!("   Until you quit the running session, you'll see split-brain state:");
                eprintln!("     • Session identity (header, org name) stays as {from}");
                eprintln!("       — cached in memory at startup, can't be changed.");
                eprintln!("     • API calls (/usage, completions, billing) use {email}");
                eprintln!("       — they re-read the keychain on each request.");
                eprintln!("     • Next OAuth token refresh (typically within the hour) will");
                eprintln!("       overwrite the keychain back to {from}, silently reverting");
                eprintln!("       this swap for future sessions too.");
                eprintln!();
                eprintln!("   Quit Claude Code before the next refresh for the swap to stick.");
            }
            SwapNote::BackgroundProcess => {
                eprintln!(
                    "\nNote: a running Claude Code process will keep using {from} until restarted."
                );
            }
            SwapNote::Silent => {}
        }
    }

    Ok(())
}

/// Re-attach CLI-specific remediation copy to surface-agnostic
/// `SwapError` variants. The core error message is intentionally
/// neutral so the same variant can fan out to the CLI (which gets
/// `--force`) and the GUI/tray (which gets an Override button).
fn annotate_swap_err(e: SwapError) -> anyhow::Error {
    match e {
        SwapError::LiveSessionConflict => {
            anyhow::anyhow!("{e}\n\nQuit Claude Code first, or pass --force to proceed anyway.")
        }
        other => other.into(),
    }
}

/// Post-swap advisory classification. Pure function so the decision
/// table is unit-testable without mocking process-lookup.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SwapNote {
    /// `--force` *and* CC is running — full split-brain warning.
    ForceWithRunning,
    /// CC is running (no force flag) — short "will keep using old
    /// until restart" note.
    BackgroundProcess,
    /// Nothing to warn about: CC isn't running at all.
    Silent,
}

pub(crate) fn swap_completion_note(force: bool, cc_running: bool) -> SwapNote {
    if !cc_running {
        // A `--force` swap with no CC process running is functionally
        // identical to a normal swap — no split-brain risk, no process
        // to restart. Emit nothing rather than a misleading note.
        SwapNote::Silent
    } else if force {
        SwapNote::ForceWithRunning
    } else {
        SwapNote::BackgroundProcess
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- swap_completion_note ------------------------------------

    #[test]
    fn test_swap_note_force_and_running_shows_split_brain_warning() {
        // Full 13-line warning — the only state worth interrupting the
        // user with, since the swap genuinely may be reverted by CC.
        assert_eq!(swap_completion_note(true, true), SwapNote::ForceWithRunning);
    }

    #[test]
    fn test_swap_note_running_without_force_is_short_note() {
        // Normal case when CC happens to be running in the background
        // — short reminder that in-process sessions won't pick up the
        // new token until restart.
        assert_eq!(
            swap_completion_note(false, true),
            SwapNote::BackgroundProcess
        );
    }

    #[test]
    fn test_swap_note_force_no_running_is_silent() {
        // Regression guard: `--force` with no CC running is equivalent
        // to a normal swap. Old behaviour printed the "running claude
        // processes" note regardless — misleading.
        assert_eq!(swap_completion_note(true, false), SwapNote::Silent);
    }

    #[test]
    fn test_swap_note_nothing_running_is_silent() {
        // Regression guard: the old behaviour printed "running claude
        // processes will continue using the previous account" even
        // when no CC process existed. Must be silent.
        assert_eq!(swap_completion_note(false, false), SwapNote::Silent);
    }
}
