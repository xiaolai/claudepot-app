//! Tauri commands for account onboarding, removal, login, verification,
//! and usage fetch.
//!
//! `account_list` / `account_list_basic` live in `commands.rs` because
//! they're the default read surface every section mounts against. The
//! mutating / async-heavy verbs live here.

use crate::commands::open_store;
use crate::dto::{AccountSummary, RegisterOutcome, RemoveOutcome, UsageEntryDto};
use claudepot_core::services;
use claudepot_core::services::usage_cache::UsageCache;
use std::collections::HashMap;
use uuid::Uuid;

/// Spawn `claude auth login` (browser opens), wait for the user to
/// complete OAuth, then import CC's fresh blob into the existing
/// account's slot with identity verification.
///
/// Registers a cancellation Notify in `LoginState` so the companion
/// `account_login_cancel` command can abort the in-flight subprocess.
/// Only one login may run at a time; concurrent calls are rejected.
#[tauri::command]
pub async fn account_login(
    uuid: String,
    state: tauri::State<'_, crate::state::LoginState>,
) -> Result<(), String> {
    let store = open_store()?;
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;

    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    {
        let mut slot = state
            .active
            .lock()
            .map_err(|e| format!("login state lock poisoned: {e}"))?;
        if slot.is_some() {
            return Err("a login is already in progress".to_string());
        }
        *slot = Some(notify.clone());
    }

    let result = services::account_service::login_and_reimport(&store, id, Some(notify)).await;

    // Clear the slot regardless of outcome so the next login can run.
    if let Ok(mut slot) = state.active.lock() {
        slot.take();
    }

    result.map_err(|e| format!("login failed: {e}"))
}

/// Abort the in-flight `account_login` subprocess, if any. Safe to call
/// when nothing is running — returns Ok either way.
#[tauri::command]
pub async fn account_login_cancel(
    state: tauri::State<'_, crate::state::LoginState>,
) -> Result<(), String> {
    if let Ok(guard) = state.active.lock() {
        if let Some(notify) = guard.as_ref() {
            notify.notify_one();
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn account_add_from_current() -> Result<RegisterOutcome, String> {
    let store = open_store()?;
    let result = services::account_service::register_from_current(&store)
        .await
        .map_err(|e| format!("register failed: {e}"))?;
    Ok(RegisterOutcome {
        email: result.email,
        org_name: result.org_name,
        subscription_type: result.subscription_type,
    })
}

/// Browser-OAuth onboarding: spawn `claude auth login` in a temp
/// config dir, wait for the user to finish, then register the fresh
/// identity. The refresh token never crosses the IPC bridge — the
/// blob is read directly on the Rust side.
///
/// Registers a cancellation `Notify` in `LoginState` so the existing
/// `account_login_cancel` command aborts this flow too. Only one
/// browser login (register or re-login) may run at a time; concurrent
/// calls are rejected with a descriptive error instead of silently
/// sharing the same temp dir.
#[tauri::command]
pub async fn account_register_from_browser(
    state: tauri::State<'_, crate::state::LoginState>,
) -> Result<RegisterOutcome, String> {
    let store = open_store()?;

    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    {
        let mut slot = state
            .active
            .lock()
            .map_err(|e| format!("login state lock poisoned: {e}"))?;
        if slot.is_some() {
            return Err("a login is already in progress".to_string());
        }
        *slot = Some(notify.clone());
    }

    let result =
        services::account_service::register_from_browser_cancellable(&store, Some(notify)).await;

    // Clear the slot regardless of outcome so the next login can run.
    if let Ok(mut slot) = state.active.lock() {
        slot.take();
    }

    match result {
        Ok(r) => Ok(RegisterOutcome {
            email: r.email,
            org_name: r.org_name,
            subscription_type: r.subscription_type,
        }),
        Err(e) => Err(format!("register failed: {e}")),
    }
}

// Intentionally NOT exposed to the webview: a command that accepts a raw
// refresh token would force the secret through JS memory and the IPC bridge.
// Token-based onboarding stays CLI-only. `account_register_from_browser`
// above is the GUI-side equivalent — the OAuth flow runs entirely in core,
// so the refresh token never materialises in JS.

#[tauri::command]
pub async fn account_remove(uuid: String) -> Result<RemoveOutcome, String> {
    let store = open_store()?;
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let result = services::account_service::remove_account(&store, id, None)
        .await
        .map_err(|e| format!("remove failed: {e}"))?;
    Ok(RemoveOutcome {
        email: result.email,
        was_cli_active: result.was_cli_active,
        was_desktop_active: result.was_desktop_active,
        had_desktop_profile: result.had_desktop_profile,
        warnings: result.warnings,
    })
}

/// Invalidate cache + cooldown for a single account, then refetch.
/// Used by per-row "Retry" affordances when one account is rate-limited
/// or failing while others are fine — refetching everyone would be
/// wasteful and can itself trigger rate limits on healthy accounts.
#[tauri::command]
pub async fn refresh_usage_for(
    uuid: String,
    cache: tauri::State<'_, UsageCache>,
) -> Result<UsageEntryDto, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    cache.invalidate(id).await;
    // Identity-gated fetch: refuses to serve when the stored slot's
    // verify_status is drift/rejected so we never attribute another
    // account's usage to this UUID (audit H4).
    let store = open_store()?;
    let batch = cache.fetch_batch_detailed_verified(&store, &[id]).await;
    let outcome = batch
        .into_values()
        .next()
        .unwrap_or(claudepot_core::services::usage_cache::UsageOutcome::Error(
            "no outcome produced".to_string(),
        ));
    Ok(UsageEntryDto::from_outcome(outcome))
}

/// Fetch usage for every account that has credentials. Every input
/// account appears in the output map — accounts whose usage is
/// unavailable carry a `status` explaining *why* so the GUI can
/// render an inline placeholder ("Token expired", "Rate-limited",
/// etc.) instead of silently hiding the row.
///
/// Accounts without credentials are NOT included here; the UI already
/// knows this from `has_cli_credentials` on AccountSummary and handles
/// it separately (the sidebar shows a Log-in button).
#[tauri::command]
pub async fn fetch_all_usage(
    cache: tauri::State<'_, UsageCache>,
) -> Result<HashMap<String, UsageEntryDto>, String> {
    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;

    let uuids: Vec<Uuid> = accounts
        .iter()
        .filter(|a| a.has_cli_credentials)
        .map(|a| a.uuid)
        .collect();

    tracing::info!(
        total = accounts.len(),
        with_creds = uuids.len(),
        "fetch_all_usage starting"
    );

    if uuids.is_empty() {
        return Ok(HashMap::new());
    }

    // Identity-gated batch: any uuid whose stored verify_status is
    // drift/rejected returns an Error outcome instead of being served
    // against a misfiled token (audit H4 privacy bug). The UI renders
    // the gate failure as "Couldn't fetch usage" with the detail.
    let batch = cache.fetch_batch_detailed_verified(&store, &uuids).await;

    let mut out = HashMap::new();
    for (uuid, outcome) in batch {
        let entry = UsageEntryDto::from_outcome(outcome);
        tracing::info!(account = %uuid, status = %entry.status, "usage fetched");
        out.insert(uuid.to_string(), entry);
    }
    Ok(out)
}

/// Reconcile a single account's stored blob against `/api/oauth/profile`.
/// Mirrors `verify_all_accounts` but scoped — used by the per-account
/// context-menu / palette "Verify now" action.
///
/// Returns the refreshed `AccountSummary` for the target account so the
/// caller can patch the row without a full list round-trip.
#[tauri::command]
pub async fn verify_account(uuid: String) -> Result<AccountSummary, String> {
    use claudepot_core::cli_backend::swap::DefaultProfileFetcher;
    use claudepot_core::services::identity;

    let store = open_store()?;
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let fetcher = DefaultProfileFetcher;
    identity::verify_account_identity(&store, id, &fetcher)
        .await
        .map_err(|e| format!("verify failed: {e}"))?;
    let account = store
        .find_by_uuid(id)
        .map_err(|e| format!("lookup failed: {e}"))?
        .ok_or_else(|| "account not found".to_string())?;
    Ok(AccountSummary::from(&account))
}

/// Reconcile every account's blob identity against `/api/oauth/profile`.
/// Called by the Refresh button; the GUI may also auto-invoke it on
/// window focus (debounced) so drift surfaces without a click.
#[tauri::command]
pub async fn verify_all_accounts() -> Result<Vec<AccountSummary>, String> {
    use claudepot_core::cli_backend::swap::DefaultProfileFetcher;
    use claudepot_core::services::identity;
    use std::time::Duration;

    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    let fetcher = DefaultProfileFetcher;

    let mut first = true;
    // Single stagger counter; verify_account_identity already does its
    // own DB read to check the latest row, so we only iterate UUIDs.
    let uuids: Vec<uuid::Uuid> = accounts
        .iter()
        .filter(|a| a.has_cli_credentials)
        .map(|a| a.uuid)
        .collect();
    for uuid in uuids {
        if !first {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        first = false;
        match identity::verify_account_identity(&store, uuid, &fetcher).await {
            Ok(outcome) => tracing::info!(
                account = %uuid,
                status = outcome.as_str(),
                "verify_all_accounts: result"
            ),
            Err(e) => tracing::warn!(
                account = %uuid,
                "verify_all_accounts: error {e}"
            ),
        }
    }

    // Re-list once to pick up the freshly persisted verify_status
    // columns. DTO construction still recomputes token_health per row
    // (reads each blob from disk once) — acceptable O(n) disk reads, and
    // the values can differ from what verify_account_identity saw if a
    // refresh rotated the access_token in between.
    let refreshed = store.list().map_err(|e| format!("list failed: {e}"))?;
    Ok(refreshed.iter().map(AccountSummary::from).collect())
}
