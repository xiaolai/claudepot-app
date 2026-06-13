//! `clear` verb — sign Desktop out (stashing a snapshot by default).
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the per-verb layout rationale.

use super::*;

pub async fn clear(ctx: &AppContext, keep_snapshot: bool) -> Result<()> {
    use claudepot_core::desktop_backend;
    use claudepot_core::services::desktop_service;

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Desktop not supported on this platform"))?;

    let outcome = desktop_service::clear_session(&*platform, &ctx.store, keep_snapshot)
        .await
        .map_err(|e| anyhow::anyhow!("clear failed: {e}"))?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "email": outcome.email,
                "snapshot_kept": outcome.snapshot_kept,
                "items_deleted": outcome.items_deleted,
            })
        );
    } else {
        match outcome.email {
            Some(e) => println!(
                "Signed Desktop out ({e}). Deleted {} item(s).",
                outcome.items_deleted
            ),
            None => println!(
                "Signed Desktop out. Deleted {} item(s). No active account was recorded.",
                outcome.items_deleted
            ),
        }
        if outcome.snapshot_kept {
            println!("Snapshot preserved.");
        }
    }
    Ok(())
}
