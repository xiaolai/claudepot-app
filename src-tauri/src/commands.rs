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
    Ok(accounts.iter().map(AccountSummary::from).collect())
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
    cli_backend::swap::switch(
        &store,
        current_id,
        target.uuid,
        platform.as_ref(),
        true,
        &refresher,
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

#[tauri::command]
pub async fn account_add_from_token(refresh_token: String) -> Result<RegisterOutcome, String> {
    let store = open_store()?;
    let result = services::account_service::register_from_token(&store, &refresh_token)
        .await
        .map_err(|e| format!("register failed: {e}"))?;
    Ok(RegisterOutcome {
        email: result.email,
        org_name: result.org_name,
        subscription_type: result.subscription_type,
    })
}

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
