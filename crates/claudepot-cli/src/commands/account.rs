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
    } else if let Some(_token) = from_token {
        anyhow::bail!("--from-token not yet implemented (Step 5)")
    } else {
        anyhow::bail!("browser-based add not yet implemented (Step 5)")
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

pub fn inspect(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;

    let email = resolve_email(&ctx.store, email_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx.store.find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    if ctx.json {
        println!("{}", serde_json::json!({
            "uuid": account.uuid.to_string(),
            "email": account.email,
            "org_uuid": account.org_uuid,
            "org_name": account.org_name,
            "subscription_type": account.subscription_type,
            "rate_limit_tier": account.rate_limit_tier,
            "created_at": account.created_at.to_rfc3339(),
            "cli_active": account.is_cli_active,
            "desktop_active": account.is_desktop_active,
            "has_cli_credentials": account.has_cli_credentials,
            "has_desktop_profile": account.has_desktop_profile,
        }));
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
        println!("  Added:     {}", account.created_at.format("%Y-%m-%d %H:%M"));
        println!("  CLI:       {}", if account.is_cli_active { "active" } else { "—" });
        println!("  Desktop:   {}", if account.is_desktop_active { "active" } else if account.has_desktop_profile { "profile stored" } else { "—" });
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
