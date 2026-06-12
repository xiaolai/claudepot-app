//! `status` verb — show the active CLI account.
//!
//! Sub-module of `commands/cli_ops.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

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
