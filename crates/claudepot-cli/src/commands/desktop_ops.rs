use crate::AppContext;
use anyhow::Result;

pub async fn status(ctx: &AppContext) -> Result<()> {
    use claudepot_core::desktop_backend;

    let platform = desktop_backend::create_platform();
    let platform = match platform {
        Some(p) => p,
        None => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({"error": "Desktop not supported on this platform"})
                );
            } else {
                println!("Claude Desktop is not supported on this platform.");
            }
            return Ok(());
        }
    };

    let data_dir = platform.data_dir();
    let installed = data_dir.as_ref().is_some_and(|d| d.exists());

    let active_uuid = ctx.store.active_desktop_uuid()?;
    let active_account = active_uuid
        .and_then(|u| u.parse::<uuid::Uuid>().ok())
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten());

    let is_running = platform.is_running().await;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "installed": installed,
                "running": is_running,
                "active": active_account.as_ref().map(|a| &a.email),
            })
        );
    } else {
        if !installed {
            println!("Claude Desktop is not installed.");
            return Ok(());
        }
        match &active_account {
            Some(a) => println!("Active Desktop account: {}", a.email),
            None => println!("No active Desktop account."),
        }
        println!(
            "  Desktop: {}",
            if is_running { "running" } else { "not running" }
        );
    }

    Ok(())
}

pub async fn use_account(ctx: &AppContext, email_input: &str, no_launch: bool) -> Result<()> {
    use claudepot_core::desktop_backend;
    use claudepot_core::resolve::resolve_email;

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Claude Desktop is not supported on this platform"))?;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let target = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    // Check if target has a Desktop profile
    let profile_dir = claudepot_core::paths::desktop_profile_dir(target.uuid);
    if !profile_dir.exists() {
        anyhow::bail!(
            "no Desktop profile stored for {email}. \
             Sign in to Claude Desktop as this account first, then use \
             `claudepot desktop use` to switch."
        );
    }

    let current_uuid = ctx
        .store
        .active_desktop_uuid()?
        .and_then(|s| s.parse::<uuid::Uuid>().ok());

    if current_uuid == Some(target.uuid) {
        ctx.info(&format!("Already active: {email}"));
        return Ok(());
    }

    let from_email = current_uuid
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten())
        .map(|a| a.email)
        .unwrap_or_else(|| "(none)".to_string());

    ctx.info(&format!("Switching Desktop: {from_email} → {email}"));

    desktop_backend::swap::switch(
        platform.as_ref(),
        &ctx.store,
        current_uuid,
        target.uuid,
        no_launch,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "from": from_email,
                "to": email,
                "launched": !no_launch,
            })
        );
    } else {
        println!("Desktop: {from_email} → {email}");
        if no_launch {
            println!("Desktop was not relaunched (--no-launch).");
        }
    }

    Ok(())
}
