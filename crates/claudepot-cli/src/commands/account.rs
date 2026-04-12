use anyhow::Result;
use crate::AppContext;
use crate::output;

pub fn list(ctx: &AppContext) -> Result<()> {
    let accounts = ctx.store.list()?;
    let formatted = output::format_account_list(&accounts, ctx.json);
    println!("{formatted}");
    Ok(())
}

pub async fn add(ctx: &AppContext, from_current: bool, from_token: Option<String>) -> Result<()> {
    use claudepot_core::services::account_service;

    let result = if from_current {
        ctx.info("Reading current CC credentials...");
        ctx.info("Fetching account profile...");
        account_service::register_from_current(&ctx.store).await?
    } else if let Some(token_arg) = from_token {
        // Read token from stdin if "-" is passed (avoids shell history exposure)
        let token = if token_arg == "-" {
            ctx.info("Reading refresh token from stdin...");
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            buf.trim().to_string()
        } else {
            token_arg
        };
        ctx.info("Exchanging refresh token...");
        ctx.info("Fetching account profile...");
        account_service::register_from_token(&ctx.store, &token).await?
    } else {
        return add_via_browser(ctx).await;
    };

    if ctx.json {
        println!("{}", serde_json::json!({
            "registered": true,
            "email": result.email,
            "org": result.org_name,
            "plan": result.subscription_type,
            "uuid": result.uuid.to_string(),
        }));
    } else {
        println!("Registered: {} ({} {})",
            result.email,
            capitalize(&result.subscription_type),
            result.rate_limit_tier.as_deref()
                .and_then(|t| t.split('_').last())
                .unwrap_or("")
        );
    }
    Ok(())
}

/// Browser-based add delegates to core's register_from_browser.
async fn add_via_browser(ctx: &AppContext) -> Result<()> {
    use claudepot_core::services::account_service;

    ctx.info("Opening browser for OAuth login...");
    ctx.info("(Complete the login in your browser)");

    let result = account_service::register_from_browser(&ctx.store).await?;

    if ctx.json {
        println!("{}", serde_json::json!({
            "registered": true,
            "email": result.email,
            "org": result.org_name,
            "plan": result.subscription_type,
            "uuid": result.uuid.to_string(),
        }));
    } else {
        println!("Registered: {} ({} {})",
            result.email,
            capitalize(&result.subscription_type),
            result.rate_limit_tier.as_deref()
                .and_then(|t| t.split('_').last())
                .unwrap_or("")
        );
    }
    Ok(())
}

pub fn remove(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::account_service;

    let email = resolve_email(&ctx.store, email_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx.store.find_by_email(&email)?
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

    let result = account_service::remove_account(&ctx.store, account.uuid)?;

    if ctx.json {
        println!("{}", serde_json::json!({
            "removed": true,
            "email": result.email,
            "was_cli_active": result.was_cli_active,
            "was_desktop_active": result.was_desktop_active,
            "had_desktop_profile": result.had_desktop_profile,
            "warnings": result.warnings,
        }));
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

pub async fn inspect(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::account_service;

    let email = resolve_email(&ctx.store, email_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx.store.find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    let health = account_service::token_health(account.uuid, account.has_cli_credentials);
    let usage_result = account_service::fetch_usage(account.uuid).await;

    if ctx.json {
        let mut j = serde_json::json!({
            "uuid": account.uuid.to_string(),
            "email": account.email,
            "org_uuid": account.org_uuid,
            "org_name": account.org_name,
            "subscription_type": account.subscription_type,
            "rate_limit_tier": account.rate_limit_tier,
            "created_at": account.created_at.to_rfc3339(),
            "cli_active": account.is_cli_active,
            "desktop_active": account.is_desktop_active,
            "token_status": health.status,
        });
        if let Some(ref u) = usage_result {
            if let Some(ref fh) = u.five_hour {
                j["five_hour_pct"] = serde_json::json!(fh.utilization);
                j["five_hour_resets"] = serde_json::json!(fh.resets_at.to_rfc3339());
            }
            if let Some(ref sd) = u.seven_day {
                j["seven_day_pct"] = serde_json::json!(sd.utilization);
                j["seven_day_resets"] = serde_json::json!(sd.resets_at.to_rfc3339());
            }
        }
        println!("{}", serde_json::to_string_pretty(&j)?);
    } else {
        println!("Account: {}", account.email);
        println!("  Org:       {}", account.org_name.as_deref().unwrap_or("?"));
        println!("  Org UUID:  {}", account.org_uuid.as_deref().unwrap_or("?"));
        println!("  Plan:      {} {}",
            capitalize(account.subscription_type.as_deref().unwrap_or("?")),
            account.rate_limit_tier.as_deref()
                .and_then(|t| t.split('_').last())
                .unwrap_or("")
        );
        println!("  Token:     {}", health.status);
        println!("  Added:     {}", account.created_at.format("%Y-%m-%d %H:%M"));
        println!("  CLI:       {}", if account.is_cli_active { "active" } else { "—" });
        println!("  Desktop:   {}", if account.is_desktop_active { "active" } else if account.has_desktop_profile { "profile stored" } else { "—" });

        if let Some(ref u) = usage_result {
            if let Some(ref fh) = u.five_hour {
                println!("  5h usage:  {:.0}% (resets {})", fh.utilization, fh.resets_at.format("%H:%M UTC"));
            }
            if let Some(ref sd) = u.seven_day {
                println!("  7d usage:  {:.0}% (resets {})", sd.utilization, sd.resets_at.format("%b %d"));
            }
        } else if account.has_cli_credentials {
            println!("  Usage:     (could not fetch — token may be expired)");
        }
    }

    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}
