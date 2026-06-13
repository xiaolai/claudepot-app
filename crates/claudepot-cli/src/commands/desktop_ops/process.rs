//! `launch` / `quit` verb group — Desktop process control. Grouped
//! per the commands.md verb-group guidance: both are trivial wrappers
//! over the same platform process surface.
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the layout rationale.

use super::*;

pub async fn launch(ctx: &AppContext) -> Result<()> {
    use claudepot_core::desktop_backend;
    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Desktop not supported on this platform"))?;
    platform
        .launch()
        .await
        .map_err(|e| anyhow::anyhow!("launch failed: {e}"))?;
    if ctx.json {
        println!("{}", serde_json::json!({ "launched": true }));
    } else {
        println!("Launch requested.");
    }
    Ok(())
}

pub async fn quit(ctx: &AppContext) -> Result<()> {
    use claudepot_core::desktop_backend;
    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Desktop not supported on this platform"))?;
    if platform.is_running().await {
        platform
            .quit()
            .await
            .map_err(|e| anyhow::anyhow!("quit failed: {e}"))?;
        if ctx.json {
            println!("{}", serde_json::json!({ "quit": true }));
        } else {
            println!("Desktop quit.");
        }
    } else if ctx.json {
        println!("{}", serde_json::json!({ "quit": false }));
    } else {
        println!("Desktop was not running.");
    }
    Ok(())
}
