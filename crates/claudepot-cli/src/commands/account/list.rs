//! `list` verb — table of registered accounts with live usage.
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

pub async fn list(ctx: &AppContext) -> Result<()> {
    // Reconcile the active_cli pointer with what CC's keychain actually
    // holds. This catches the case where a running CC process refreshed
    // its token and overwrote the blob Claudepot swapped in. The /profile
    // call adds ~200ms but prevents stale active-CLI display — the same
    // sync the GUI already performs on startup.
    match claudepot_core::services::account_service::sync_from_current_cc(&ctx.store).await {
        Ok(Some(_)) => {} // synced — pointer may have been corrected
        Ok(None) => {}    // CC has no blob or no matching account
        Err(e) => {
            // Best-effort: if /profile fails (network, token revoked),
            // show the DB-sourced list with a warning rather than failing.
            if !ctx.quiet {
                eprintln!(
                    "\u{26a0}  Couldn't verify CC credentials ({}). Active CLI pointer may be stale.",
                    e
                );
            }
        }
    }

    let accounts = ctx.store.list()?;

    // Collect UUIDs for accounts with credentials to batch-fetch usage.
    //
    // No terminal-status filter here: `fetch_batch` takes the store and
    // applies `identity_gate` itself, so a drifted slot cannot reach
    // `/usage` from this call site or any future one. Filtering here as
    // well would be a second copy of that rule, and the copy is what
    // goes stale.
    let uuids: Vec<uuid::Uuid> = accounts
        .iter()
        .filter(|a| a.has_cli_credentials)
        .map(|a| a.uuid)
        .collect();

    let usage_map = if uuids.is_empty() {
        std::collections::HashMap::new()
    } else {
        ctx.usage_cache
            .fetch_batch(&ctx.store, &uuids)
            .await
            .into_iter()
            .filter_map(|(id, result)| {
                let report = result.ok().flatten()?;
                let row = output::AccountUsageRow {
                    five_hour: report.five_hour.as_ref().map(|w| w.utilization),
                    seven_day: report.seven_day.as_ref().map(|w| w.utilization),
                };
                Some((id, row))
            })
            .collect()
    };

    let formatted = output::format_account_list(&accounts, &usage_map, ctx.json);
    println!("{formatted}");
    Ok(())
}
