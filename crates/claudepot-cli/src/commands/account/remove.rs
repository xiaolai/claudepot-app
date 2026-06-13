//! `remove` verb — delete a registered account (with confirmation).
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

pub async fn remove(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::account_service;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    if !ctx.yes {
        eprint!("Remove account \"{email}\"? [y/N] ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    let result =
        account_service::remove_account(&ctx.store, account.uuid, Some(&ctx.usage_cache)).await?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "removed": true,
                "email": result.email,
                "was_cli_active": result.was_cli_active,
                "was_desktop_active": result.was_desktop_active,
                "had_desktop_profile": result.had_desktop_profile,
                "warnings": result.warnings,
            })
        );
    } else {
        if result.had_desktop_profile {
            ctx.info("Deleted Desktop profile snapshot.");
        }
        if result.was_cli_active {
            ctx.info("Note: this was the active CLI account. CLI slot is now empty.");
        }
        if result.was_desktop_active {
            ctx.info("Note: this was the active Desktop account. Desktop slot is now empty.");
        }
        for warning in &result.warnings {
            eprintln!("Warning: {warning}");
        }
        ctx.info(&format!("Removed: {}", result.email));
    }
    Ok(())
}
