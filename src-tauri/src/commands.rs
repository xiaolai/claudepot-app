//! Tauri command layer — thin async wrappers over `claudepot-core`.
//!
//! Per `.claude/rules/architecture.md`, NO business logic lives here. Each
//! command opens the store, calls a core function, and serializes the result.
//! Errors become user-facing strings at this boundary.

use crate::dto::{AccountSummary, AccountUsageDto, AppStatus, RegisterOutcome, RemoveOutcome};
use claudepot_core::account::{Account, AccountStore};
use claudepot_core::cli_backend;
use claudepot_core::desktop_backend;
use claudepot_core::paths;
use claudepot_core::services;
use claudepot_core::services::usage_cache::UsageCache;
use std::collections::HashMap;
use uuid::Uuid;

fn resolve_target(store: &AccountStore, email: &str) -> Result<Account, String> {
    let target_email = claudepot_core::resolve::resolve_email(store, email)
        .map_err(|e| format!("resolve failed: {e}"))?;
    store
        .find_by_email(&target_email)
        .map_err(|e| format!("lookup failed: {e}"))?
        .ok_or_else(|| format!("account not found: {target_email}"))
}

fn active_id<E>(
    store: &AccountStore,
    f: fn(&AccountStore) -> Result<Option<String>, E>,
) -> Option<Uuid> {
    f(store)
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok())
}

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

    let email_for = |id: Option<Uuid>| -> Option<String> {
        id.and_then(|u| {
            accounts
                .iter()
                .find(|a| a.uuid == u)
                .map(|a| a.email.clone())
        })
    };
    let cli_active_email = email_for(active_id(&store, AccountStore::active_cli_uuid));
    let desktop_active_email = email_for(active_id(&store, AccountStore::active_desktop_uuid));

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
    let target = resolve_target(&store, &email)?;
    let current_id = active_id(&store, AccountStore::active_cli_uuid);

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
    let target = resolve_target(&store, &email)?;

    // Preflight: refuse to quit Desktop if the target has no stored profile.
    let target_profile_dir = paths::desktop_profile_dir(target.uuid);
    if !target_profile_dir.exists() {
        return Err(format!(
            "{} has no Desktop profile yet \u{2014} sign in via the Desktop app first",
            target.email
        ));
    }

    let outgoing_id = active_id(&store, AccountStore::active_desktop_uuid);

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| "Desktop not supported on this platform".to_string())?;
    desktop_backend::swap::switch(&*platform, &store, outgoing_id, target.uuid, no_launch)
        .await
        .map_err(|e| format!("desktop switch failed: {e}"))
}

/// macOS-only: request a keychain unlock via the system's native dialog.
/// Spawns `security unlock-keychain` without -p so macOS shows its built-in
/// "Unlock Keychain" password prompt. The user's password never reaches
/// Claudepot (it goes to macOS's trusted process).
#[tauri::command]
pub async fn unlock_keychain() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tokio::process::Command;
        // `security unlock-keychain` with no password attempts an interactive
        // unlock. In a GUI process context, macOS surfaces the standard
        // Keychain Access unlock panel.
        let out = Command::new("/usr/bin/security")
            .arg("unlock-keychain")
            .output()
            .await
            .map_err(|e| format!("security spawn failed: {e}"))?;
        if !out.status.success() {
            // Exit 51 is common when the user cancels the prompt.
            let code = out.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("unlock-keychain exited {code}: {}", stderr.trim()));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("keychain unlock is macOS-only".to_string())
    }
}

/// Idempotent startup sync: if CC is currently signed in as one of the
/// registered Claudepot accounts, make sure Claudepot's stored blob
/// and active_cli match. Lets users who ran `claude auth login`
/// externally see healthy credentials the moment the GUI opens,
/// without clicking anything.
///
/// Returns the email that was synced, empty string if nothing matched,
/// or a clearly-prefixed error message for user-facing conditions (like
/// a locked keychain) that the UI should surface prominently.
#[tauri::command]
pub async fn sync_from_current_cc() -> Result<String, String> {
    let store = open_store()?;
    match services::account_service::sync_from_current_cc(&store).await {
        Ok(Some(uuid)) => Ok(store
            .find_by_uuid(uuid)
            .ok()
            .flatten()
            .map(|a| a.email)
            .unwrap_or_default()),
        Ok(None) => Ok(String::new()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("keychain is locked") {
                // User-actionable error: bubble up to the GUI so it can
                // show a banner, not a hidden log line.
                return Err(msg);
            }
            tracing::warn!("sync_from_current_cc: {e}");
            Ok(String::new())
        }
    }
}

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
        let mut slot = state.active.lock().unwrap();
        if slot.is_some() {
            return Err("a login is already in progress".to_string());
        }
        *slot = Some(notify.clone());
    }

    let result = services::account_service::login_and_reimport(&store, id, Some(notify)).await;

    // Clear the slot regardless of outcome so the next login can run.
    state.active.lock().unwrap().take();

    result.map_err(|e| format!("login failed: {e}"))
}

/// Abort the in-flight `account_login` subprocess, if any. Safe to call
/// when nothing is running — returns Ok either way.
#[tauri::command]
pub fn account_login_cancel(
    state: tauri::State<'_, crate::state::LoginState>,
) -> Result<(), String> {
    if let Some(notify) = state.active.lock().unwrap().as_ref() {
        notify.notify_one();
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

// Intentionally NOT exposed to the webview: a command that accepts a raw
// refresh token would force the secret through JS memory and the IPC bridge.
// Token-based onboarding stays CLI-only; the GUI will support it via a future
// core-owned browser flow (register_from_browser) that never materializes the
// token in JS.

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

/// Fetch usage for all accounts that have credentials.
///
/// Returns a map of UUID → usage data. Accounts with no credentials,
/// expired tokens, or any error (including rate limits) are silently
/// omitted — the UI sees `null` and shows nothing. Rate-limit errors
/// are never exposed to the frontend.
#[tauri::command]
pub async fn fetch_all_usage(
    cache: tauri::State<'_, UsageCache>,
) -> Result<HashMap<String, AccountUsageDto>, String> {
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

    let batch = cache.fetch_batch_graceful(&uuids).await;

    let mut out = HashMap::new();
    for (uuid, maybe_response) in batch {
        match maybe_response {
            Some(response) => {
                tracing::info!(account = %uuid, "usage fetched");
                out.insert(uuid.to_string(), AccountUsageDto::from_response(&response));
            }
            None => tracing::warn!(account = %uuid, "usage returned None (no creds / refresh failed / fetch failed)"),
        }
    }
    Ok(out)
}

/// Reconcile every account's blob identity against `/api/oauth/profile`.
///
/// Iterates all accounts with credentials, calls
/// `services::identity::verify_account_identity` for each (staggered by
/// the usage cache's BATCH_STAGGER so the endpoint doesn't see a burst),
/// and returns the refreshed AccountSummary list so the GUI can re-render
/// with new `verify_status` / `verified_email` / `drift` fields.
///
/// This is what the Refresh button calls; the GUI may also auto-invoke
/// it on window focus (debounced) so drift surfaces without a click.
#[tauri::command]
pub async fn verify_all_accounts() -> Result<Vec<AccountSummary>, String> {
    use claudepot_core::cli_backend::swap::DefaultProfileFetcher;
    use claudepot_core::services::identity;
    use std::time::Duration;

    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    let fetcher = DefaultProfileFetcher;

    let mut first = true;
    for account in &accounts {
        if !account.has_cli_credentials {
            continue;
        }
        if !first {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        first = false;
        match identity::verify_account_identity(&store, account.uuid, &fetcher).await {
            Ok(outcome) => tracing::info!(
                account = %account.uuid,
                status = outcome.as_str(),
                "verify_all_accounts: result"
            ),
            Err(e) => tracing::warn!(
                account = %account.uuid,
                "verify_all_accounts: error {e}"
            ),
        }
    }

    // Re-list to pick up the freshly persisted verify_status columns.
    let refreshed = store.list().map_err(|e| format!("list failed: {e}"))?;
    Ok(refreshed.iter().map(AccountSummary::from).collect())
}
