use anyhow::Result;
use crate::AppContext;

pub fn status(ctx: &AppContext) -> Result<()> {
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
                        println!("{}", serde_json::json!({
                            "active": account.email,
                            "uuid": account.uuid.to_string(),
                            "plan": account.subscription_type,
                        }));
                    } else {
                        println!("Active CLI account: {}", account.email);
                        if let Some(ref plan) = account.subscription_type {
                            println!("  Plan: {plan}");
                        }
                        if let Some(ref ts) = account.last_cli_switch {
                            println!("  Switched: {}", ts.format("%Y-%m-%d %H:%M"));
                        }
                    }
                }
                None => {
                    ctx.store.clear_active_cli()?;
                    if ctx.json {
                        println!("{}", serde_json::json!({"active": null, "error": "orphaned pointer cleared"}));
                    } else {
                        println!("Active pointer was orphaned (account removed). Cleared.");
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn use_account(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::cli_backend;

    let email = resolve_email(&ctx.store, email_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let target = ctx.store.find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    let current_uuid = ctx.store.active_cli_uuid()?
        .and_then(|s| s.parse::<uuid::Uuid>().ok());

    if current_uuid == Some(target.uuid) {
        ctx.info(&format!("Already active: {email}"));
        return Ok(());
    }

    let platform = cli_backend::create_platform();

    ctx.info(&format!("Switching CLI to {email}..."));
    cli_backend::swap::switch(&ctx.store, current_uuid, target.uuid, platform.as_ref()).await?;

    let from = current_uuid
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten())
        .map(|a| a.email)
        .unwrap_or_else(|| "(none)".to_string());

    if ctx.json {
        println!("{}", serde_json::json!({
            "from": from,
            "to": email,
        }));
    } else {
        println!("CLI: {from} → {email}");
        eprintln!("\nNote: running claude processes will continue using the previous account until restarted.");
    }

    Ok(())
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
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::launcher;

    let email = resolve_email(&ctx.store, email_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx.store.find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    if !account.has_cli_credentials {
        anyhow::bail!("no credentials stored for {email}");
    }

    if print_token {
        eprintln!("⚠ WARNING: outputting raw access token. Do not log or share this value.");
        let token = launcher::get_access_token(account.uuid).await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("{token}");
        return Ok(());
    }

    if args.is_empty() {
        anyhow::bail!("no command specified. Usage: claudepot cli run <email> [--] <cmd...>");
    }

    ctx.info(&format!("Running as {} (Mode D)...", email));
    let exit_code = launcher::run(account.uuid, args).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    std::process::exit(exit_code);
}
