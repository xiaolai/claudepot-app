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
    if from_current {
        add_from_current(ctx).await
    } else if let Some(token) = from_token {
        add_from_token(ctx, &token).await
    } else {
        add_via_browser(ctx).await
    }
}

async fn add_from_current(ctx: &AppContext) -> Result<()> {
    use claudepot_core::blob::CredentialBlob;
    use claudepot_core::cli_backend;
    use claudepot_core::account::Account;
    use claudepot_core::oauth::profile;
    use chrono::Utc;
    use uuid::Uuid;

    ctx.info("Reading current CC credentials...");

    // Read from CC's storage
    let platform = cli_backend::create_platform();
    let blob_str = platform.read_default().await?
        .ok_or_else(|| anyhow::anyhow!(
            "No CC credentials found. Log in with `claude auth login` first."
        ))?;

    let blob = CredentialBlob::from_json(&blob_str)?;
    ctx.info("Fetching account profile...");

    // Call profile API to get email + org info
    let prof = profile::fetch(&blob.claude_ai_oauth.access_token).await?;

    // Check if already registered
    if let Some(existing) = ctx.store.find_by_email(&prof.email)? {
        anyhow::bail!("Already registered: {} (uuid: {})", existing.email, existing.uuid);
    }

    let account_id = Uuid::new_v4();

    // Save credential to keyring
    cli_backend::swap::save_private(account_id, &blob_str)?;

    // Insert into account store
    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    ctx.store.insert(&account)?;

    if ctx.json {
        println!("{}", serde_json::json!({
            "registered": true,
            "email": prof.email,
            "org": prof.org_name,
            "plan": prof.subscription_type,
            "uuid": account_id.to_string(),
        }));
    } else {
        println!("Registered: {} ({} {})",
            prof.email,
            capitalize(&prof.subscription_type),
            prof.rate_limit_tier.as_deref()
                .and_then(|t| t.split('_').last())
                .unwrap_or("")
        );
    }

    Ok(())
}

pub fn remove(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;

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

    // Delete credential from keyring
    let _ = claudepot_core::cli_backend::swap::delete_private(account.uuid);

    // Remove from store
    ctx.store.remove(account.uuid)?;

    if account.is_cli_active {
        ctx.store.clear_active_cli()?;
        ctx.info("Note: this was the active CLI account. CLI slot is now empty.");
    }

    ctx.info(&format!("Removed: {email}"));
    Ok(())
}

pub async fn inspect(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::cli_backend::swap::load_private;
    use claudepot_core::blob::CredentialBlob;
    use claudepot_core::oauth::usage;

    let email = resolve_email(&ctx.store, email_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx.store.find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    // Try to fetch usage if we have credentials
    let usage_result = if account.has_cli_credentials {
        if let Ok(blob_str) = load_private(account.uuid) {
            if let Ok(blob) = CredentialBlob::from_json(&blob_str) {
                if !blob.is_expired(0) {
                    usage::fetch(&blob.claude_ai_oauth.access_token).await.ok()
                } else {
                    None
                }
            } else { None }
        } else { None }
    } else { None };

    // Token health
    let token_status = if account.has_cli_credentials {
        if let Ok(blob_str) = load_private(account.uuid) {
            if let Ok(blob) = CredentialBlob::from_json(&blob_str) {
                let remaining_mins = (blob.claude_ai_oauth.expires_at - chrono::Utc::now().timestamp_millis()) / 60_000;
                if remaining_mins > 0 {
                    format!("valid ({}m remaining)", remaining_mins)
                } else {
                    "expired".to_string()
                }
            } else { "corrupt blob".to_string() }
        } else { "missing".to_string() }
    } else { "no credentials".to_string() };

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
            "token_status": token_status,
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
        println!("  Token:     {}", token_status);
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

async fn add_from_token(ctx: &AppContext, refresh_token: &str) -> Result<()> {
    use claudepot_core::oauth::{profile, refresh};
    use claudepot_core::cli_backend::swap;
    use claudepot_core::account::Account;
    use chrono::Utc;
    use uuid::Uuid;

    ctx.info("Exchanging refresh token...");
    let token_resp = refresh::refresh(refresh_token).await
        .map_err(|e| anyhow::anyhow!("token exchange failed: {e}"))?;

    ctx.info("Fetching account profile...");
    let prof = profile::fetch(&token_resp.access_token).await?;

    if let Some(existing) = ctx.store.find_by_email(&prof.email)? {
        anyhow::bail!("Already registered: {} (uuid: {})", existing.email, existing.uuid);
    }

    let account_id = Uuid::new_v4();
    let blob_str = refresh::build_blob(&token_resp);
    swap::save_private(account_id, &blob_str)?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid),
        org_name: Some(prof.org_name),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    ctx.store.insert(&account)?;

    if ctx.json {
        println!("{}", serde_json::json!({"registered": true, "email": prof.email}));
    } else {
        println!("Registered: {} ({})", prof.email, capitalize(&prof.subscription_type));
    }
    Ok(())
}

async fn add_via_browser(ctx: &AppContext) -> Result<()> {
    use claudepot_core::onboard;
    use claudepot_core::oauth::profile;
    use claudepot_core::cli_backend::swap;
    use claudepot_core::account::Account;
    use chrono::Utc;
    use uuid::Uuid;

    ctx.info("Opening browser for OAuth login...");
    ctx.info("(Complete the login in your browser)");

    let config_dir = onboard::run_auth_login().await?;

    ctx.info("Reading credentials from login...");
    let blob_str = match onboard::read_credentials_from_dir(&config_dir) {
        Ok(b) => b,
        Err(e) => {
            onboard::cleanup(&config_dir).await;
            return Err(anyhow::anyhow!("failed to read credentials: {e}"));
        }
    };

    let blob = claudepot_core::blob::CredentialBlob::from_json(&blob_str)?;

    ctx.info("Fetching account profile...");
    let prof = match profile::fetch(&blob.claude_ai_oauth.access_token).await {
        Ok(p) => p,
        Err(e) => {
            onboard::cleanup(&config_dir).await;
            return Err(anyhow::anyhow!("profile fetch failed: {e}"));
        }
    };

    if let Some(existing) = ctx.store.find_by_email(&prof.email)? {
        onboard::cleanup(&config_dir).await;
        anyhow::bail!("Already registered: {} (uuid: {})", existing.email, existing.uuid);
    }

    let account_id = Uuid::new_v4();
    swap::save_private(account_id, &blob_str)?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid),
        org_name: Some(prof.org_name),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    ctx.store.insert(&account)?;

    // Cleanup temp dir + hashed keychain
    onboard::cleanup(&config_dir).await;

    if ctx.json {
        println!("{}", serde_json::json!({"registered": true, "email": prof.email}));
    } else {
        println!("Registered: {} ({})", prof.email, capitalize(&prof.subscription_type));
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
