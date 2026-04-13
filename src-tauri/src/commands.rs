//! Tauri command layer — thin async wrappers over `claudepot-core`.
//!
//! Per `.claude/rules/architecture.md`, NO business logic lives here. Each
//! command opens the store, calls a core function, and serializes the result.
//! Errors become user-facing strings at this boundary.

use crate::dto::{AccountSummary, AppStatus, RegisterOutcome, RemoveOutcome};
use claudepot_core::account::AccountStore;
use claudepot_core::cli_backend;
use claudepot_core::desktop_backend;
use claudepot_core::paths;
use claudepot_core::services;
use uuid::Uuid;

/// Open the production store (single instance per command). Cheap: a sqlite
/// open + schema check. Keeps the command layer stateless.
fn open_store() -> Result<AccountStore, String> {
    let db = paths::claudepot_data_dir().join("accounts.db");
    AccountStore::open(&db).map_err(|e| format!("store open failed: {e}"))
}

#[tauri::command]
pub fn account_list() -> Result<Vec<AccountSummary>, String> {
    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    let summaries: Vec<AccountSummary> = accounts.iter().map(AccountSummary::from).collect();

    // Opportunistically sync DB flags with reality: if an account's flag
    // claims credentials exist but the stored blob is missing/corrupt,
    // flip the flag false. Best-effort — ignore DB errors.
    for (acct, sum) in accounts.iter().zip(summaries.iter()) {
        if acct.has_cli_credentials && !sum.credentials_healthy {
            let _ = store.update_credentials_flag(acct.uuid, false);
        }
    }

    Ok(summaries)
}

#[tauri::command]
pub fn app_status() -> Result<AppStatus, String> {
    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;

    let cli_active_email = store
        .active_cli_uuid()
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .and_then(|u| {
            accounts
                .iter()
                .find(|a| a.uuid == u)
                .map(|a| a.email.clone())
        });
    let desktop_active_email = store
        .active_desktop_uuid()
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .and_then(|u| {
            accounts
                .iter()
                .find(|a| a.uuid == u)
                .map(|a| a.email.clone())
        });

    let desktop_installed = desktop_backend::create_platform()
        .and_then(|p| p.data_dir())
        .map(|d| d.exists())
        .unwrap_or(false);

    Ok(AppStatus {
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cli_active_email,
        desktop_active_email,
        desktop_installed,
        data_dir: paths::claudepot_data_dir().display().to_string(),
        account_count: accounts.len(),
    })
}

#[tauri::command]
pub async fn cli_use(email: String) -> Result<(), String> {
    let store = open_store()?;
    let target_email = claudepot_core::resolve::resolve_email(&store, &email)
        .map_err(|e| format!("resolve failed: {e}"))?;
    let target = store
        .find_by_email(&target_email)
        .map_err(|e| format!("lookup failed: {e}"))?
        .ok_or_else(|| format!("account not found: {target_email}"))?;

    let current_id = store
        .active_cli_uuid()
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok());

    let platform = cli_backend::create_platform();
    let refresher = cli_backend::swap::DefaultRefresher;
    let fetcher = cli_backend::swap::DefaultProfileFetcher;
    cli_backend::swap::switch(
        &store,
        current_id,
        target.uuid,
        platform.as_ref(),
        true,
        &refresher,
        &fetcher,
    )
    .await
    .map_err(|e| format!("cli switch failed: {e}"))
}

#[tauri::command]
pub async fn cli_clear() -> Result<(), String> {
    let store = open_store()?;
    services::cli_service::clear_credentials(&store)
        .await
        .map_err(|e| format!("clear failed: {e}"))
}

#[tauri::command]
pub async fn desktop_use(email: String, no_launch: bool) -> Result<(), String> {
    let store = open_store()?;
    let target_email = claudepot_core::resolve::resolve_email(&store, &email)
        .map_err(|e| format!("resolve failed: {e}"))?;
    let target = store
        .find_by_email(&target_email)
        .map_err(|e| format!("lookup failed: {e}"))?
        .ok_or_else(|| format!("account not found: {target_email}"))?;

    // Preflight: refuse to quit Desktop if the target has no stored profile.
    // Without this, switch() would quit Claude, snapshot the outgoing account,
    // then fail on NoStoredProfile and leave the user without an open Desktop.
    //
    // The filesystem is authoritative — has_desktop_profile in the DB can lag
    // (e.g. user deleted the profile directory manually). Check the dir.
    let target_profile_dir = paths::desktop_profile_dir(target.uuid);
    if !target_profile_dir.exists() {
        return Err(format!(
            "{} has no Desktop profile yet \u{2014} sign in via the Desktop app first",
            target.email
        ));
    }

    let outgoing_id = store
        .active_desktop_uuid()
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok());

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| "Desktop not supported on this platform".to_string())?;
    desktop_backend::swap::switch(&*platform, &store, outgoing_id, target.uuid, no_launch)
        .await
        .map_err(|e| format!("desktop switch failed: {e}"))
}

#[tauri::command]
pub async fn account_reimport_from_current(uuid: String) -> Result<(), String> {
    let store = open_store()?;
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    services::account_service::reimport_from_current(&store, id)
        .await
        .map_err(|e| format!("reimport failed: {e}"))
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

// Intentionally NOT exposed to the webview: a command that accepts a raw
// refresh token would force the secret through JS memory and the IPC bridge.
// Token-based onboarding stays CLI-only; the GUI will support it via a future
// core-owned browser flow (register_from_browser) that never materializes the
// token in JS.

#[tauri::command]
pub fn account_remove(uuid: String) -> Result<RemoveOutcome, String> {
    let store = open_store()?;
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let result = services::account_service::remove_account(&store, id)
        .map_err(|e| format!("remove failed: {e}"))?;
    Ok(RemoveOutcome {
        email: result.email,
        was_cli_active: result.was_cli_active,
        was_desktop_active: result.was_desktop_active,
        had_desktop_profile: result.had_desktop_profile,
        warnings: result.warnings,
    })
}
