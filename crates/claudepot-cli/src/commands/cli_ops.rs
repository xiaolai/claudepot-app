use crate::AppContext;
use anyhow::Result;

pub async fn status(ctx: &AppContext) -> Result<()> {
    // Audit M1: reconcile DB pointer with CC's shared slot before
    // reporting, so external state changes (`claude auth login` / a
    // running Claude that rotated tokens) are reflected. Previously
    // `cli status` read the stored pointer directly and could report
    // the wrong active account. Best-effort — on keychain-locked or
    // other sync failures we still report what the DB knows.
    if let Err(e) =
        claudepot_core::services::account_service::sync_from_current_cc(&ctx.store).await
    {
        tracing::debug!("cli status: sync_from_current_cc best-effort failure: {e}");
    }
    let active_uuid = ctx.store.active_cli_uuid()?;
    match active_uuid {
        None => {
            if ctx.json {
                println!("{}", serde_json::json!({"active": null}));
            } else {
                println!("No active CLI account.");
            }
        }
        Some(uuid_str) => {
            let uuid: uuid::Uuid = uuid_str.parse()?;
            match ctx.store.find_by_uuid(uuid)? {
                Some(account) => {
                    if ctx.json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "active": account.email,
                                "uuid": account.uuid.to_string(),
                                "plan": account.subscription_type,
                            })
                        );
                    } else {
                        println!("Active CLI account: {}", account.email);
                        if let Some(ref plan) = account.subscription_type {
                            println!("  Plan: {plan}");
                        }
                        if let Some(ref ts) = account.last_cli_switch {
                            println!(
                                "  Switched: {}",
                                crate::time_fmt::format_local_datetime(
                                    &ts.with_timezone(&chrono::Local)
                                )
                            );
                        }
                    }
                }
                None => {
                    ctx.store.clear_active_cli()?;
                    if ctx.json {
                        println!(
                            "{}",
                            serde_json::json!({"active": null, "error": "orphaned pointer cleared"})
                        );
                    } else {
                        println!("Active pointer was orphaned (account removed). Cleared.");
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn use_account(ctx: &AppContext, email_input: &str, no_refresh: bool, force: bool) -> Result<()> {
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
            eprintln!(
                "\u{26a0}  Couldn't verify CC state ({e}); proceeding with DB view."
            );
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
            &ctx.store, current_uuid, target.uuid,
            platform.as_ref(), !no_refresh, &refresher, &fetcher,
        )
        .await?;
    } else {
        cli_backend::swap::switch(
            &ctx.store, current_uuid, target.uuid,
            platform.as_ref(), !no_refresh, &refresher, &fetcher,
        )
        .await?;
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
    } else {
        println!("CLI: {from} → {email}");
        // Ask the core whether a CC process is alive only once, then
        // decide what to print. Skipping the probe entirely on non-force
        // paths (the old behaviour) meant we printed the "running
        // claude processes will continue" note even when no CC was
        // running — misleading.
        let cc_running =
            claudepot_core::cli_backend::swap::is_cc_process_running_public().await;
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

pub async fn clear(ctx: &AppContext) -> Result<()> {
    use claudepot_core::services::cli_service;

    cli_service::clear_credentials(&ctx.store).await?;

    if ctx.json {
        println!("{}", serde_json::json!({"cleared": true}));
    } else {
        println!("CC credentials cleared.");
    }

    Ok(())
}

pub async fn run(
    ctx: &AppContext,
    email_input: &str,
    print_token: bool,
    args: &[String],
) -> Result<()> {
    use claudepot_core::launcher;
    use claudepot_core::resolve::resolve_email;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    if !account.has_cli_credentials {
        anyhow::bail!("no credentials stored for {email}");
    }

    match classify_run_mode(print_token, args).map_err(|e| anyhow::anyhow!("{e}"))? {
        RunMode::PrintToken => {
            eprintln!(
                "⚠ WARNING: outputting raw access token. Do not log or share this value."
            );
            let token = launcher::get_access_token(account.uuid)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("{token}");
            Ok(())
        }
        RunMode::Exec => {
            // Drop the internal "Mode D" jargon — users shouldn't need
            // to know the implementation-plan's mode-letter taxonomy to
            // read a progress line. Show the bin name instead.
            let bin = args
                .first()
                .map(String::as_str)
                .unwrap_or("<cmd>");
            ctx.info(&format!("Running {bin} as {email}..."));
            let exit_code = launcher::run(account.uuid, args)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            std::process::exit(exit_code);
        }
    }
}

/// What `cli run` should do given the flag + argv combination. Pure,
/// testable, no I/O.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// `--print-token` alone: refresh + print the access token.
    PrintToken,
    /// At least one positional arg: env-inject + exec.
    Exec,
}

/// Mis-combined `cli run` flags. Each variant has a stable `Display`
/// impl that maps 1:1 to a user-visible CLI error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunArgsError {
    /// `--print-token` was passed alongside a command. Previously these
    /// extra args were silently dropped; we now refuse to hide the
    /// mismatch.
    PrintTokenWithArgs,
    /// No `--print-token` and no command — nothing to do.
    NoCommand,
}

impl std::fmt::Display for RunArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrintTokenWithArgs => write!(
                f,
                "--print-token does not take a command; remove the extra args \
                 or drop --print-token"
            ),
            Self::NoCommand => write!(
                f,
                "no command specified. Usage: claudepot cli run <email> [--] <cmd...>"
            ),
        }
    }
}
impl std::error::Error for RunArgsError {}

pub(crate) fn classify_run_mode(
    print_token: bool,
    args: &[String],
) -> Result<RunMode, RunArgsError> {
    match (print_token, args.is_empty()) {
        (true, true) => Ok(RunMode::PrintToken),
        (true, false) => Err(RunArgsError::PrintTokenWithArgs),
        (false, true) => Err(RunArgsError::NoCommand),
        (false, false) => Ok(RunMode::Exec),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- classify_run_mode ---------------------------------------

    #[test]
    fn test_run_mode_print_token_alone_is_print_token() {
        assert_eq!(classify_run_mode(true, &[]), Ok(RunMode::PrintToken));
    }

    #[test]
    fn test_run_mode_args_without_print_token_is_exec() {
        let args = vec!["echo".to_string(), "hi".to_string()];
        assert_eq!(classify_run_mode(false, &args), Ok(RunMode::Exec));
    }

    #[test]
    fn test_run_mode_print_token_with_args_errors() {
        // Regression guard: old code silently ignored `echo hi` here.
        // The user might reasonably think the command ran as them, but
        // nothing actually ran. Refuse instead.
        let args = vec!["echo".to_string(), "hi".to_string()];
        assert_eq!(
            classify_run_mode(true, &args),
            Err(RunArgsError::PrintTokenWithArgs)
        );
    }

    #[test]
    fn test_run_mode_no_print_token_no_args_is_no_command() {
        assert_eq!(classify_run_mode(false, &[]), Err(RunArgsError::NoCommand));
    }

    #[test]
    fn test_run_args_error_messages_are_stable() {
        // User-facing error strings — locked down so future edits don't
        // silently reshape script-visible error output.
        assert_eq!(
            RunArgsError::NoCommand.to_string(),
            "no command specified. Usage: claudepot cli run <email> [--] <cmd...>"
        );
        let msg = RunArgsError::PrintTokenWithArgs.to_string();
        assert!(msg.starts_with("--print-token does not take a command"));
    }

    // ---- swap_completion_note ------------------------------------

    #[test]
    fn test_swap_note_force_and_running_shows_split_brain_warning() {
        // Full 13-line warning — the only state worth interrupting the
        // user with, since the swap genuinely may be reverted by CC.
        assert_eq!(
            swap_completion_note(true, true),
            SwapNote::ForceWithRunning
        );
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
