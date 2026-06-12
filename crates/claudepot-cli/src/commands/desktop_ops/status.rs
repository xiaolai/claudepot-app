//! `status` verb — show the active Desktop account and running state.
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the per-verb layout rationale.

use super::*;

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
