//! Tauri command layer — thin async wrappers over `claudepot-core`.
//!
//! Per `.claude/rules/architecture.md`, NO business logic lives here. Each
//! command opens the store, calls a core function, and serializes the result.
//! Errors become user-facing strings at this boundary.

use crate::dto;
use crate::dto::{
    AccountSummary, AppStatus, CcIdentity, CleanPreviewDto, DryRunPlanDto,
    JournalEntryDto, MoveArgsDto, ProjectDetailDto, ProjectInfoDto, ProtectedPathDto,
    RegisterOutcome, RemoveOutcome, UsageEntryDto,
};
use claudepot_core::account::{Account, AccountStore};
use claudepot_core::cli_backend;
use claudepot_core::desktop_backend;
use claudepot_core::paths;
use claudepot_core::project;
use claudepot_core::project_repair;
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
pub(crate) fn open_store() -> Result<AccountStore, String> {
    let db = paths::claudepot_data_dir().join("accounts.db");
    AccountStore::open(&db).map_err(|e| format!("store open failed: {e}"))
}

#[tauri::command]
pub fn account_list() -> Result<Vec<AccountSummary>, String> {
    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    let summaries: Vec<AccountSummary> = accounts.iter().map(AccountSummary::from).collect();

    // Opportunistically sync DB flags with reality. Best-effort — we
    // never surface DB write failures here; the flag is re-reconciled
    // on the next list.
    for (acct, sum) in accounts.iter().zip(summaries.iter()) {
        // CLI: flag claims credentials exist but the stored blob is
        // missing/corrupt → flip false.
        if acct.has_cli_credentials && !sum.credentials_healthy {
            let _ = store.update_credentials_flag(acct.uuid, false);
        }
    }

    // Desktop: delegate to the shared reconcile_flags service so the
    // hot path in account_list and the explicit `desktop_reconcile`
    // command run identical logic. Best-effort — any failure leaves
    // the flags as-is for the next list.
    let _ = services::desktop_service::reconcile_flags(&store);

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

    // Authoritative: app bundle / MSIX package present. Previously
    // we checked `data_dir().exists()` which false-negatived on
    // installed-but-never-launched AND false-negatived when the user
    // manually cleared the data dir. `is_installed()` is the correct
    // "is Claude Desktop on this machine" signal.
    let desktop_installed = desktop_backend::create_platform()
        .map(|p| p.is_installed())
        .unwrap_or(false);

    Ok(AppStatus {
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cli_active_email,
        desktop_active_email,
        desktop_installed,
        data_dir: paths::claudepot_data_dir().display().to_string(),
        cc_config_dir: paths::claude_config_dir().display().to_string(),
        account_count: accounts.len(),
    })
}

/// Preflight probe: is a `claude` process currently running? The GUI
/// uses this before `cli_use` to raise a split-brain confirmation
/// instead of letting the swap silently race with a live session's
/// next token refresh.
#[tauri::command]
pub async fn cli_is_cc_running() -> bool {
    claudepot_core::cli_backend::swap::is_cc_process_running_public().await
}

/// `force` defaults to false from the existing GUI; the frontend
/// can pass `true` to bypass the live-session gate after showing the
/// user a warning.
#[tauri::command]
pub async fn cli_use(email: String, force: Option<bool>) -> Result<(), String> {
    let store = open_store()?;
    let target = resolve_target(&store, &email)?;
    let current_id = active_id(&store, AccountStore::active_cli_uuid);

    let platform = cli_backend::create_platform();
    let refresher = cli_backend::swap::DefaultRefresher;
    let fetcher = cli_backend::swap::DefaultProfileFetcher;
    if force.unwrap_or(false) {
        cli_backend::swap::switch_force(
            &store, current_id, target.uuid,
            platform.as_ref(), true, &refresher, &fetcher,
        )
        .await
    } else {
        cli_backend::swap::switch(
            &store, current_id, target.uuid,
            platform.as_ref(), true, &refresher, &fetcher,
        )
        .await
    }
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

/// Ground-truth "who is Claude Desktop signed in as right now".
///
/// Mirrors [`current_cc_identity`]: reads the live data dir, probes
/// the signed-in identity, returns a DTO that never fails at the
/// Tauri boundary — failures ride in `error` so the UI can render
/// visible banners.
///
/// Phase 1: returns only the fast-path ("OrgUuidCandidate") result or
/// `None`. The `probe_method` field is the UI's trust gate — only
/// `Decrypted` (Phase 2+) is authoritative for mutation. See
/// `desktop_identity` module docs for the rationale.
#[tauri::command]
pub async fn current_desktop_identity() -> Result<dto::DesktopIdentity, String> {
    let now = chrono::Utc::now();
    let Some(platform) = desktop_backend::create_platform() else {
        return Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: Some("Desktop not supported on this platform".to_string()),
        });
    };
    let store = open_store()?;

    match claudepot_core::desktop_identity::probe_live_identity(
        &*platform,
        &store,
        claudepot_core::desktop_identity::ProbeOptions::default(),
    ) {
        Ok(None) => Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: None,
        }),
        Ok(Some(live)) => Ok(dto::DesktopIdentity {
            email: Some(live.email),
            org_uuid: Some(live.org_uuid),
            probe_method: match live.probe_method {
                claudepot_core::desktop_identity::ProbeMethod::OrgUuidCandidate => {
                    dto::DesktopProbeMethod::OrgUuidCandidate
                }
                claudepot_core::desktop_identity::ProbeMethod::Decrypted => {
                    dto::DesktopProbeMethod::Decrypted
                }
            },
            verified_at: now,
            error: None,
        }),
        Err(e) => Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: Some(e.to_string()),
        }),
    }
}

/// Explicit flag-vs-disk reconcile. The same logic runs
/// opportunistically inside `account_list`; this command surfaces
/// the outcome so the GUI or CLI can show "N flags were reconciled."
#[tauri::command]
pub async fn desktop_reconcile() -> Result<dto::DesktopReconcileOutcome, String> {
    let store = open_store()?;
    let outcome = services::desktop_service::reconcile_flags(&store)
        .map_err(|e| format!("reconcile failed: {e}"))?;
    Ok(dto::DesktopReconcileOutcome {
        flag_flips: outcome
            .flag_flips
            .into_iter()
            .map(|f| dto::DesktopFlagFlip {
                email: f.email,
                uuid: f.uuid.to_string(),
                new_value: f.new_value,
            })
            .collect(),
        orphan_pointer_cleared: outcome.orphan_pointer_cleared,
    })
}

/// Adopt the live Desktop session into `uuid`'s snapshot directory.
/// Verifies the live identity via the slow-path probe before mutating
/// anything — per plan v2 §D6+§VerifiedIdentity, fast-path candidate
/// identities cannot drive adoption.
#[tauri::command]
pub async fn desktop_adopt(
    uuid: String,
    overwrite: bool,
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<dto::DesktopAdoptOutcome, String> {
    use tauri::Emitter;

    let _guard = lock.0.lock().await;

    let target_uuid = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let store = open_store()?;
    let platform = claudepot_core::desktop_backend::create_platform()
        .ok_or_else(|| "Desktop not supported on this platform".to_string())?;

    // Verify identity: the authoritative Decrypted path. Fails here
    // if the live session isn't signed in, if the keychain secret
    // can't be read, or if /profile rejects the token.
    let verified = claudepot_core::desktop_identity::verify_live_identity(&*platform, &store)
        .await
        .map_err(|e| format!("identity probe failed: {e}"))?
        .ok_or_else(|| "no live Desktop identity — sign in via Desktop first".to_string())?;

    let outcome = services::desktop_service::adopt_current(
        &*platform,
        &store,
        target_uuid,
        &verified,
        overwrite,
    )
    .await
    .map_err(|e| format!("desktop adopt failed: {e}"))?;

    let _ = app.emit("desktop-adopted", &outcome.account_email);
    Ok(dto::DesktopAdoptOutcome {
        account_email: outcome.account_email,
        captured_items: outcome.captured_items,
        size_bytes: outcome.size_bytes,
    })
}

/// Sign Desktop out. Stashes the live session into the active
/// account's snapshot dir by default (`keep_snapshot=true`) so the
/// user can swap back in later.
#[tauri::command]
pub async fn desktop_clear(
    keep_snapshot: bool,
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<dto::DesktopClearOutcome, String> {
    use tauri::Emitter;

    let _guard = lock.0.lock().await;

    let store = open_store()?;
    let platform = claudepot_core::desktop_backend::create_platform()
        .ok_or_else(|| "Desktop not supported on this platform".to_string())?;

    let outcome = services::desktop_service::clear_session(&*platform, &store, keep_snapshot)
        .await
        .map_err(|e| format!("desktop clear failed: {e}"))?;

    let _ = app.emit("desktop-cleared", &outcome.email);
    Ok(dto::DesktopClearOutcome {
        email: outcome.email,
        snapshot_kept: outcome.snapshot_kept,
        items_deleted: outcome.items_deleted,
    })
}

/// Startup/window-focus sync. Never mutates the filesystem — at most
/// caches the `active_desktop` pointer when the live identity maps to
/// a registered account that already has a snapshot. UI subscribes
/// to the returned `DesktopSyncOutcome` variants (AdoptionAvailable,
/// Stranger, CandidateOnly) to surface banners.
#[tauri::command]
pub async fn sync_from_current_desktop(
    lock: tauri::State<'_, crate::state::DesktopOpState>,
) -> Result<dto::DesktopSyncOutcome, String> {
    let _guard = lock.0.lock().await;

    let store = open_store()?;
    let platform = match claudepot_core::desktop_backend::create_platform() {
        Some(p) => p,
        None => return Ok(dto::DesktopSyncOutcome::NoLive),
    };
    let outcome = services::desktop_service::sync_from_current(&*platform, &store)
        .await
        .map_err(|e| format!("sync failed: {e}"))?;
    Ok(match outcome {
        services::desktop_service::SyncOutcome::NoLive => dto::DesktopSyncOutcome::NoLive,
        services::desktop_service::SyncOutcome::Verified { email } => {
            dto::DesktopSyncOutcome::Verified { email }
        }
        services::desktop_service::SyncOutcome::AdoptionAvailable { email } => {
            dto::DesktopSyncOutcome::AdoptionAvailable { email }
        }
        services::desktop_service::SyncOutcome::Stranger { email } => {
            dto::DesktopSyncOutcome::Stranger { email }
        }
        services::desktop_service::SyncOutcome::CandidateOnly { email } => {
            dto::DesktopSyncOutcome::CandidateOnly { email }
        }
    })
}

#[tauri::command]
pub async fn desktop_is_running() -> Result<bool, String> {
    match claudepot_core::desktop_backend::create_platform() {
        Some(p) => Ok(p.is_running().await),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn desktop_launch(
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;
    let _guard = lock.0.lock().await;
    let platform = claudepot_core::desktop_backend::create_platform()
        .ok_or_else(|| "Desktop not supported on this platform".to_string())?;
    platform
        .launch()
        .await
        .map_err(|e| format!("launch failed: {e}"))?;
    let _ = app.emit("desktop-running-changed", true);
    Ok(())
}

#[tauri::command]
pub async fn desktop_quit(
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;
    let _guard = lock.0.lock().await;
    let platform = claudepot_core::desktop_backend::create_platform()
        .ok_or_else(|| "Desktop not supported on this platform".to_string())?;
    if platform.is_running().await {
        platform
            .quit()
            .await
            .map_err(|e| format!("quit failed: {e}"))?;
    }
    let _ = app.emit("desktop-running-changed", false);
    Ok(())
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

/// Reveal a path in the OS file manager. macOS: `open -R`, Linux:
/// open the parent directory with xdg-open, Windows: `explorer /select`.
/// Accepts an absolute path; returns an error if the path is empty or
/// does not exist.
#[tauri::command]
pub async fn reveal_in_finder(path: String) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".to_string());
    }
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        // Walk up to the nearest existing ancestor so "Open in Finder" on a
        // CC project whose source was deleted still opens the parent.
        let mut cur: Option<&std::path::Path> = p.parent();
        let fallback = loop {
            match cur {
                Some(parent) if parent.exists() => break Some(parent.to_path_buf()),
                Some(parent) => cur = parent.parent(),
                None => break None,
            }
        };
        let Some(target) = fallback else {
            return Err(format!("path does not exist: {path}"));
        };
        return spawn_reveal(&target).await;
    }
    spawn_reveal(&p).await
}

async fn spawn_reveal(p: &std::path::Path) -> Result<(), String> {
    use tokio::process::Command;
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("/usr/bin/open")
            .args(["-R"])
            .arg(p)
            .output()
            .await
            .map_err(|e| format!("open spawn failed: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("open exited: {}", stderr.trim()));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let target = if p.is_dir() {
            p.to_path_buf()
        } else {
            p.parent().unwrap_or(p).to_path_buf()
        };
        let out = Command::new("xdg-open")
            .arg(&target)
            .output()
            .await
            .map_err(|e| format!("xdg-open spawn failed: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("xdg-open exited: {}", stderr.trim()));
        }
    }
    #[cfg(target_os = "windows")]
    {
        // `explorer /select,<path>` quirks: it returns exit code 1 even
        // on success in some Windows versions, so we can't rely on the
        // status as strictly as macOS/Linux. Still check that spawning
        // worked, but fall through on non-zero rather than masking a
        // successful reveal with an error toast.
        let out = Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .output()
            .await
            .map_err(|e| format!("explorer spawn failed: {e}"))?;
        let code = out.status.code().unwrap_or(0);
        // Only bail on known-fatal exit codes. Explorer commonly returns
        // 1 on success.
        if code > 1 {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("explorer exited {code}: {}", stderr.trim()));
        }
    }
    Ok(())
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
pub fn account_login_cancel(
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

/// Ground-truth "what is CC actually authenticated as".
///
/// Reads CC's shared credential slot (the `Claude Code-credentials`
/// keychain item on macOS, file on Linux/Windows), calls
/// `/api/oauth/profile`, returns the verified email. This is what
/// `claude auth status` would print — useful when Claudepot's own
/// `active_cli` pointer has drifted from reality.
///
/// Never returns an error to the frontend: all failure modes land in
/// the `error` field of the returned DTO so the GUI can render them
/// as a visible banner instead of a toast that might get dismissed.
#[tauri::command]
pub async fn current_cc_identity() -> Result<CcIdentity, String> {
    use claudepot_core::blob::CredentialBlob;
    use claudepot_core::cli_backend::swap::{DefaultProfileFetcher, ProfileFetcher};

    let now = chrono::Utc::now();
    let platform = cli_backend::create_platform();
    let blob_str = match platform.read_default().await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Ok(CcIdentity {
                email: None,
                verified_at: now,
                error: None,
            });
        }
        Err(e) => {
            return Ok(CcIdentity {
                email: None,
                verified_at: now,
                error: Some(format!("couldn't read CC credentials: {e}")),
            });
        }
    };
    let blob = match CredentialBlob::from_json(&blob_str) {
        Ok(b) => b,
        Err(e) => {
            return Ok(CcIdentity {
                email: None,
                verified_at: now,
                error: Some(format!("CC blob is not valid JSON: {e}")),
            });
        }
    };
    let fetcher = DefaultProfileFetcher;
    match fetcher
        .fetch_email(&blob.claude_ai_oauth.access_token)
        .await
    {
        Ok(email) => Ok(CcIdentity {
            email: Some(email),
            verified_at: now,
            error: None,
        }),
        Err(e) => Ok(CcIdentity {
            email: None,
            verified_at: now,
            error: Some(format!("/profile returned error: {e}")),
        }),
    }
}

// ---------------------------------------------------------------------------
// Project read-only surface (Step 2 of gui-rename plan)
// ---------------------------------------------------------------------------

/// Default journal nag threshold per spec §8 Q7 — mirrors the CLI.
const JOURNAL_NAG_THRESHOLD_SECS: u64 = 86_400;

fn claudepot_home_dirs() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    paths::claudepot_repair_dirs()
}

#[tauri::command]
pub fn project_list() -> Result<Vec<ProjectInfoDto>, String> {
    let cfg = paths::claude_config_dir();
    let projects = project::list_projects(&cfg).map_err(|e| format!("list failed: {e}"))?;
    Ok(projects.iter().map(ProjectInfoDto::from).collect())
}

#[tauri::command]
pub fn project_show(path: String) -> Result<ProjectDetailDto, String> {
    let cfg = paths::claude_config_dir();
    let detail =
        project::show_project(&cfg, &path).map_err(|e| format!("show failed: {e}"))?;
    Ok(ProjectDetailDto::from(&detail))
}

/// Sentinel the client checks for and silently discards. Distinguished
/// from real failures so the preview pane doesn't flash an error
/// state just because the user kept typing.
const DRY_RUN_SUPERSEDED: &str = "__claudepot_dry_run_superseded__";

#[tauri::command]
pub fn project_move_dry_run(
    args: MoveArgsDto,
    registry: State<crate::state::DryRunRegistry>,
) -> Result<DryRunPlanDto, String> {
    use std::sync::atomic::Ordering;

    // Record this call's token as the latest. A later call with a
    // greater token will overwrite it; we compare again at exit so
    // we can bail on stale work without returning a misleading plan.
    //
    // `fetch_max` instead of `store`: if the client sends tokens out
    // of order (rare but possible with async dispatch), we want to
    // preserve the highest seen value so a "genuinely latest" call
    // wins regardless of arrival order.
    let my_token = args.cancel_token.unwrap_or(0);
    if my_token > 0 {
        registry.latest.fetch_max(my_token, Ordering::SeqCst);
    }

    // Short-circuit: if a newer token has already been seen before we
    // even start, bail immediately. Saves work on rapid typing.
    if my_token > 0 && registry.latest.load(Ordering::SeqCst) > my_token {
        return Err(DRY_RUN_SUPERSEDED.to_string());
    }

    let cfg = paths::claude_config_dir();
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    let repair_root = paths::claudepot_repair_dir();
    let snapshots_dir = Some(repair_root.join("snapshots"));
    let core_args = project::MoveArgs {
        old_path: args.old_path.into(),
        new_path: args.new_path.into(),
        config_dir: cfg,
        claude_json_path,
        snapshots_dir,
        no_move: args.no_move,
        merge: args.merge,
        overwrite: args.overwrite,
        force: args.force,
        dry_run: true, // enforced; caller cannot turn this off
        ignore_pending_journals: args.ignore_pending_journals,
        claudepot_state_dir: Some(repair_root),
    };
    let plan = project::plan_move(&core_args).map_err(|e| format!("dry-run failed: {e}"))?;

    // Final check: a newer token arrived while we were computing. The
    // plan is stale by definition — return the sentinel instead of
    // the old plan so the UI doesn't render a mismatched preview.
    if my_token > 0 && registry.latest.load(Ordering::SeqCst) > my_token {
        return Err(DRY_RUN_SUPERSEDED.to_string());
    }

    Ok(DryRunPlanDto::from(&plan))
}

// ---------------------------------------------------------------------------
// Project clean (orphan reclaim) surface
// ---------------------------------------------------------------------------

/// Return the set of projects that would be cleaned and the count of
/// unreachable candidates skipped. Read-only: no lock, no deletion.
/// The pending-journals gate is NOT applied here because this is just
/// a preview — the gate fires on `project_clean_execute`.
#[tauri::command]
pub fn project_clean_preview() -> Result<CleanPreviewDto, String> {
    let cfg = paths::claude_config_dir();
    let (_journals, locks, snaps) = claudepot_home_dirs();
    let (result, orphans) = project::clean_orphans(
        &cfg,
        None, // claude.json inspection during preview would be a read; skip for now — the execute path handles that and the preview just shows what will be removed.
        Some(snaps.as_path()),
        Some(locks.as_path()),
        true, // dry run
    )
    .map_err(|e| format!("clean preview failed: {e}"))?;

    let total_bytes = orphans.iter().map(|p| p.total_size_bytes).sum();
    // Disclose how many candidates fall under protection so the
    // confirmation modal can hint that sibling state will be preserved
    // for those (audit fix). Resolution uses the same fail-safe
    // fallback as the execute path so preview and execute agree.
    let protected = claudepot_core::protected_paths::resolved_set_or_defaults(
        &paths::claudepot_data_dir(),
    );
    let protected_count = orphans
        .iter()
        .filter(|p| !p.is_empty && protected.contains(&p.original_path))
        .count();
    Ok(CleanPreviewDto {
        orphans: orphans.iter().map(ProjectInfoDto::from).collect(),
        orphans_found: result.orphans_found,
        unreachable_skipped: result.unreachable_skipped,
        total_bytes,
        protected_count,
    })
}

/// Kick off a clean in the background, returning the op_id the UI
/// subscribes to on `op-progress::<op_id>`. Gated on no pending rename
/// journals; the `__clean__` lock is acquired inside `clean_orphans`
/// so two concurrent starts can't race (the loser errors out via the
/// terminal op event).
///
/// Replaces the earlier synchronous `project_clean_execute` which
/// blocked the Tauri worker and left the UI stuck without progress
/// for multi-GB cleans.
#[tauri::command]
pub fn project_clean_start(
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let (journals, locks, snaps) = claudepot_home_dirs();

    let actionable =
        project_repair::list_actionable(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
            .map_err(|e| format!("journal check failed: {e}"))?;
    if !actionable.is_empty() {
        return Err(format!(
            "refusing to clean while {} rename journal(s) are pending. Resolve them in the Repair view first.",
            actionable.len()
        ));
    }

    let op_id = new_op_id();
    let info = RunningOpInfo {
        op_id: op_id.clone(),
        kind: OpKind::CleanProjects,
        old_path: String::new(),
        new_path: String::new(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    };
    ops.insert(info);

    let app_for_task = app.clone();
    let ops_for_task = ops.inner().clone();
    let op_id_for_task = op_id.clone();
    let cfg = paths::claude_config_dir();
    let claude_json = dirs::home_dir().map(|h| h.join(".claude.json"));
    let snaps_for_task = snaps;
    let locks_for_task = locks;
    // Resolve protected paths once on the spawning thread so the
    // background task gets a snapshot — list mutations during a
    // multi-second clean must not change the rules mid-flight. On
    // read failure, fall back to built-in defaults (audit fix: an
    // empty set would silently disable protection for `/`, `~`,
    // `/Users`, etc.).
    let protected = claudepot_core::protected_paths::resolved_set_or_defaults(
        &paths::claudepot_data_dir(),
    );

    // std::thread::spawn, not tokio::task::spawn_blocking — Tauri's
    // sync #[command] runs outside a tokio runtime context on at least
    // some dispatch paths, and `spawn_blocking` panics with "no reactor
    // running" there. A plain OS thread is fine; our work is blocking
    // I/O (fs scans, remove_dir_all) with no await points anyway.
    std::thread::spawn(move || {
        let sink = crate::ops::TauriProgressSink {
            app: app_for_task.clone(),
            op_id: op_id_for_task.clone(),
            ops: ops_for_task.clone(),
        };
        let repair_root = paths::claudepot_repair_dir();
        let result = project::clean_orphans_with_progress(
            &cfg,
            claude_json.as_deref(),
            Some(snaps_for_task.as_path()),
            Some(locks_for_task.as_path()),
            Some(repair_root.as_path()),
            &protected,
            false,
            &sink,
        );
        match result {
            Ok((clean, _orphans)) => {
                let summary = crate::ops::CleanResultSummary::from_core(&clean);
                ops_for_task.update(&op_id_for_task, |op| {
                    op.clean_result = Some(summary);
                });
                crate::ops::emit_terminal(
                    &app_for_task,
                    &ops_for_task,
                    &op_id_for_task,
                    None,
                );
            }
            Err(e) => {
                crate::ops::emit_terminal(
                    &app_for_task,
                    &ops_for_task,
                    &op_id_for_task,
                    Some(format!("clean failed: {e}")),
                );
            }
        }
    });

    Ok(op_id)
}

/// Fetch the current state of an in-flight clean. Mirrors
/// `project_move_status`. Returns `None` after the post-terminal
/// grace window expires.
#[tauri::command]
pub fn project_clean_status(
    op_id: String,
    ops: State<RunningOps>,
) -> Result<Option<RunningOpInfo>, String> {
    Ok(ops.get(&op_id))
}

#[tauri::command]
pub fn repair_list() -> Result<Vec<JournalEntryDto>, String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )
    .map_err(|e| format!("repair list failed: {e}"))?;
    Ok(entries.iter().map(JournalEntryDto::from).collect())
}

/// Cheap count for the PendingJournalsBanner. Only counts *actionable*
/// entries — excludes the `abandoned` class so the banner doesn't
/// perpetually nag about a user-dismissed entry.
#[tauri::command]
pub fn repair_pending_count() -> Result<usize, String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_actionable(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
        .map_err(|e| format!("repair count failed: {e}"))?;
    Ok(entries.len())
}

/// Status-aware banner input: counts per journal class so the UI can
/// pick a neutral / warning tone based on staleness. Abandoned entries
/// are filtered out; running entries are surfaced separately so the
/// banner can suppress itself for them (RunningOpStrip already shows
/// the op live).
#[tauri::command]
pub fn repair_status_summary() -> Result<crate::dto::PendingJournalsSummaryDto, String> {
    use claudepot_core::project_journal::JournalStatus;
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )
    .map_err(|e| format!("repair summary failed: {e}"))?;

    let mut pending = 0usize;
    let mut stale = 0usize;
    let mut running = 0usize;
    for e in &entries {
        match e.status {
            JournalStatus::Pending => pending += 1,
            JournalStatus::Stale => stale += 1,
            JournalStatus::Running => running += 1,
            JournalStatus::Abandoned => {} // filtered
        }
    }
    Ok(crate::dto::PendingJournalsSummaryDto {
        pending,
        stale,
        running,
    })
}

// ---------------------------------------------------------------------------
// Repair execution surface (Step 4 of gui-rename plan)
// ---------------------------------------------------------------------------

use crate::ops::{
    emit_terminal, new_op_id, now_unix_secs, OpKind, OpStatus, RunningOpInfo, RunningOps,
    TauriProgressSink,
};
use tauri::{AppHandle, State};

#[derive(serde::Serialize)]
pub struct BreakLockOutcomeDto {
    pub prior_pid: u32,
    pub prior_hostname: String,
    pub prior_started: String,
    pub audit_path: String,
}

#[derive(serde::Serialize)]
pub struct GcOutcomeDto {
    pub removed_journals: usize,
    pub removed_snapshots: usize,
    pub bytes_freed: u64,
    pub would_remove: Vec<String>,
}

fn find_journal(id: &str) -> Result<claudepot_core::project_repair::JournalEntry, String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )
    .map_err(|e| format!("repair list failed: {e}"))?;
    entries
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("no journal with id '{id}'"))
}

fn spawn_repair_op(
    app: AppHandle,
    ops: RunningOps,
    kind: OpKind,
    entry: claudepot_core::project_repair::JournalEntry,
) -> String {
    let op_id = new_op_id();
    let info = RunningOpInfo {
        op_id: op_id.clone(),
        kind,
        old_path: entry.journal.old_path.clone(),
        new_path: entry.journal.new_path.clone(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    };
    ops.insert(info);

    let app_for_task = app.clone();
    let ops_for_task = ops.clone();
    let op_id_for_task = op_id.clone();
    let old_path_for_task = entry.journal.old_path.clone();
    // See `project_clean_start` for why this is std::thread::spawn.
    std::thread::spawn(move || {
        let sink = TauriProgressSink {
            app: app_for_task.clone(),
            op_id: op_id_for_task.clone(),
            ops: ops_for_task.clone(),
        };
        let cfg = paths::claude_config_dir();
        let claude_json = dirs::home_dir().map(|h| h.join(".claude.json"));
        let snaps = Some(paths::claudepot_repair_dir().join("snapshots"));
        let result = match kind {
            OpKind::RepairResume => {
                project_repair::resume(&entry, cfg, claude_json, snaps, &sink)
            }
            OpKind::RepairRollback => {
                project_repair::rollback(&entry, cfg, claude_json, snaps, &sink)
            }
            OpKind::MoveProject
            | OpKind::CleanProjects
            | OpKind::SessionPrune
            | OpKind::SessionSlim
            | OpKind::SessionShare => {
                unreachable!("wrong spawn path")
            }
        };
        finalize_op(
            &app_for_task,
            &ops_for_task,
            &op_id_for_task,
            &old_path_for_task,
            result,
        );
    });

    op_id
}

/// Shared finalizer for every op spawn: on Ok, stash the structured
/// result so the UI can render snapshot paths; on Err, look up the
/// newest journal whose `old_path` matches so the UI can deep-link
/// "Open Repair" at the exact failed entry. Emits the terminal event
/// either way.
fn finalize_op(
    app: &AppHandle,
    ops: &RunningOps,
    op_id: &str,
    old_path: &str,
    result: Result<claudepot_core::project::MoveResult, claudepot_core::error::ProjectError>,
) {
    match result {
        Ok(mv) => {
            let summary = crate::ops::MoveResultSummary::from_core(&mv);
            ops.update(op_id, |op| op.move_result = Some(summary));
            emit_terminal(app, ops, op_id, None);
        }
        Err(e) => {
            let msg = e.to_string();
            let journal_id = newest_journal_id_for(old_path);
            ops.update(op_id, |op| op.failed_journal_id = journal_id.clone());
            emit_terminal(app, ops, op_id, Some(msg));
        }
    }
}

/// Best-effort: scan the journals dir for the most recent journal
/// whose old_path matches, and return its id. None on lookup failure —
/// the UI falls back to "Open Repair" without a specific target.
fn newest_journal_id_for(old_path: &str) -> Option<String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )
    .ok()?;
    entries
        .into_iter()
        .filter(|e| e.journal.old_path == old_path)
        .max_by_key(|e| e.journal.started_unix_secs)
        .map(|e| e.id)
}

#[tauri::command]
pub fn project_move_start(
    args: MoveArgsDto,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let cfg = paths::claude_config_dir();
    let claude_json = dirs::home_dir().map(|h| h.join(".claude.json"));
    let repair_root = paths::claudepot_repair_dir();
    let snaps = Some(repair_root.join("snapshots"));
    // Defensive: ignore `dry_run` from the DTO — this endpoint always
    // actually executes. Callers that want dry-run use
    // `project_move_dry_run` instead.
    let core_args = project::MoveArgs {
        old_path: args.old_path.clone().into(),
        new_path: args.new_path.clone().into(),
        config_dir: cfg,
        claude_json_path: claude_json,
        snapshots_dir: snaps,
        no_move: args.no_move,
        merge: args.merge,
        overwrite: args.overwrite,
        force: args.force,
        dry_run: false,
        ignore_pending_journals: args.ignore_pending_journals,
        claudepot_state_dir: Some(repair_root),
    };

    let op_id = new_op_id();
    let info = RunningOpInfo {
        op_id: op_id.clone(),
        kind: OpKind::MoveProject,
        old_path: args.old_path.clone(),
        new_path: args.new_path.clone(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    };
    ops.insert(info);

    let app_for_task = app.clone();
    let ops_for_task = ops.inner().clone();
    let op_id_for_task = op_id.clone();
    let old_path_for_task = args.old_path.clone();
    // See `project_clean_start` for why this is std::thread::spawn
    // rather than tokio::task::spawn_blocking.
    std::thread::spawn(move || {
        let sink = TauriProgressSink {
            app: app_for_task.clone(),
            op_id: op_id_for_task.clone(),
            ops: ops_for_task.clone(),
        };
        let result = project::move_project(&core_args, &sink);
        finalize_op(
            &app_for_task,
            &ops_for_task,
            &op_id_for_task,
            &old_path_for_task,
            result,
        );
    });

    Ok(op_id)
}

#[tauri::command]
pub fn project_move_status(
    op_id: String,
    ops: State<RunningOps>,
) -> Result<Option<RunningOpInfo>, String> {
    Ok(ops.get(&op_id))
}

#[tauri::command]
pub fn repair_resume_start(
    id: String,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let entry = find_journal(&id)?;
    Ok(spawn_repair_op(
        app,
        ops.inner().clone(),
        OpKind::RepairResume,
        entry,
    ))
}

#[tauri::command]
pub fn repair_rollback_start(
    id: String,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let entry = find_journal(&id)?;
    Ok(spawn_repair_op(
        app,
        ops.inner().clone(),
        OpKind::RepairRollback,
        entry,
    ))
}

#[tauri::command]
pub fn repair_abandon(id: String) -> Result<(), String> {
    let entry = find_journal(&id)?;
    project_repair::abandon(&entry).map_err(|e| format!("abandon failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn repair_break_lock(path: String) -> Result<BreakLockOutcomeDto, String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let lock_path = project_repair::resolve_lock_file(&locks, &path)
        .ok_or_else(|| format!("no lock file found for '{path}'"))?;
    let broken = project_repair::break_lock_with_audit(&lock_path, &journals)
        .map_err(|e| format!("break-lock failed: {e}"))?;
    Ok(BreakLockOutcomeDto {
        prior_pid: broken.prior.pid,
        prior_hostname: broken.prior.hostname,
        prior_started: broken.prior.start_iso8601,
        audit_path: broken.audit_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn repair_gc(older_than_days: u64, dry_run: bool) -> Result<GcOutcomeDto, String> {
    let (journals, _locks, snapshots) = claudepot_home_dirs();
    let result = project_repair::gc(&journals, &snapshots, older_than_days, dry_run)
        .map_err(|e| format!("gc failed: {e}"))?;
    Ok(GcOutcomeDto {
        removed_journals: result.removed_journals,
        removed_snapshots: result.removed_snapshots,
        bytes_freed: result.bytes_freed,
        would_remove: result
            .would_remove
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    })
}

/// Snapshot of currently-tracked ops. UI's RunningOpStrip polls this
/// as a backstop if events drop.
#[tauri::command]
pub fn running_ops_list(ops: State<RunningOps>) -> Result<Vec<RunningOpInfo>, String> {
    Ok(ops.list())
}

// ---------------------------------------------------------------------------
// Session move commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn session_list_orphans() -> Result<Vec<crate::dto::OrphanedProjectDto>, String> {
    let cfg = paths::claude_config_dir();
    let orphans = claudepot_core::session_move::detect_orphaned_projects(&cfg)
        .map_err(|e| format!("orphan scan failed: {e}"))?;
    Ok(orphans
        .iter()
        .map(crate::dto::OrphanedProjectDto::from)
        .collect())
}

/// CC stores `.claude.json` at `$HOME/.claude.json` — a sibling of
/// `~/.claude/`. Central accessor so the Tauri layer agrees with CLI.
fn claude_json_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

#[tauri::command]
pub fn session_move(
    session_id: String,
    from_cwd: String,
    to_cwd: String,
    force_live: bool,
    force_conflict: bool,
    cleanup_source: bool,
) -> Result<crate::dto::MoveSessionReportDto, String> {
    let sid = Uuid::parse_str(&session_id)
        .map_err(|e| format!("invalid session id: {e}"))?;
    let cfg = paths::claude_config_dir();
    let opts = claudepot_core::session_move::MoveSessionOpts {
        force_live_session: force_live,
        force_sync_conflict: force_conflict,
        cleanup_source_if_empty: cleanup_source,
        claude_json_path: claude_json_path(),
    };
    let report = claudepot_core::session_move::move_session(
        &cfg,
        sid,
        std::path::Path::new(&from_cwd),
        std::path::Path::new(&to_cwd),
        opts,
    )
    .map_err(|e| format!("move failed: {e}"))?;
    Ok(crate::dto::MoveSessionReportDto::from(&report))
}

#[tauri::command]
pub fn session_adopt_orphan(
    slug: String,
    target_cwd: String,
) -> Result<crate::dto::AdoptReportDto, String> {
    let cfg = paths::claude_config_dir();
    let target = std::path::Path::new(&target_cwd);
    if !target.is_dir() {
        return Err(format!("target cwd does not exist: {target_cwd}"));
    }
    let report =
        claudepot_core::session_move::adopt_orphan_project(&cfg, &slug, target, claude_json_path())
            .map_err(|e| format!("adopt failed: {e}"))?;
    Ok(crate::dto::AdoptReportDto::from(&report))
}

// ---------------------------------------------------------------------------
// Session index — Sessions tab list + per-session detail (transcript).
// ---------------------------------------------------------------------------

/// Walk `<config>/projects/*/*.jsonl` and produce rich list rows with
/// token totals, first-prompt previews, and model sets. Returned
/// newest-first.
///
/// `async fn` is load-bearing: Tauri 2 dispatches sync `#[command] fn`
/// handlers on the main thread (the same thread that runs the OS
/// event loop and serves the webview). A sync handler that does
/// blocking I/O — and `list_all_sessions` reads from sessions.db and
/// can fall back to a full JSONL scan — would freeze the entire
/// window for the duration of the call. With `async fn`, Tauri runs
/// the body on a Tokio worker; the sync I/O blocks that worker but
/// the main thread stays free for the webview to keep painting.
#[tauri::command]
pub async fn session_list_all() -> Result<Vec<crate::dto::SessionRowDto>, String> {
    let cfg = paths::claude_config_dir();
    let rows = claudepot_core::session::list_all_sessions(&cfg)
        .map_err(|e| format!("session list failed: {e}"))?;
    Ok(rows.iter().map(crate::dto::SessionRowDto::from).collect())
}

/// Full JSONL parse for a single session, keyed by its UUID. Returns
/// the same row metadata as `session_list_all` plus the normalized
/// event stream for transcript rendering.
///
/// `async fn` to keep the JSONL parse off Tauri's main thread — see
/// `session_list_all` for the full rationale.
#[tauri::command]
pub async fn session_read(session_id: String) -> Result<crate::dto::SessionDetailDto, String> {
    let cfg = paths::claude_config_dir();
    let detail = claudepot_core::session::read_session_detail(&cfg, &session_id)
        .map_err(|e| format!("session read failed: {e}"))?;
    Ok(crate::dto::SessionDetailDto::from(&detail))
}

/// Full JSONL parse keyed by the transcript's on-disk path. Preferred
/// over `session_read` from the GUI because list rows point at a
/// specific file and two rows can legitimately share a session_id
/// (interrupted rescue or adopt). Path must live under
/// `<config>/projects/` and must end in `.jsonl`.
///
/// `async fn` for the same off-main-thread reason as `session_read`.
#[tauri::command]
pub async fn session_read_path(
    file_path: String,
) -> Result<crate::dto::SessionDetailDto, String> {
    let cfg = paths::claude_config_dir();
    let detail = claudepot_core::session::read_session_detail_at_path(
        &cfg,
        std::path::Path::new(&file_path),
    )
    .map_err(|e| format!("session read failed: {e}"))?;
    Ok(crate::dto::SessionDetailDto::from(&detail))
}

/// Drop every cached row in `sessions.db` and repopulate from disk.
/// The (size, mtime_ns) guard handles ~every realistic transcript
/// edit; this is the escape hatch for filesystems with coarse mtime
/// resolution, clock skew, or anything that defeats the guard. The
/// next `session_list_all` call re-scans everything from cold.
#[tauri::command]
pub fn session_index_rebuild() -> Result<(), String> {
    let data_dir = paths::claudepot_data_dir();
    let db_path = data_dir.join("sessions.db");
    let idx = claudepot_core::session_index::SessionIndex::open(&db_path)
        .map_err(|e| format!("open session index: {e}"))?;
    idx.rebuild()
        .map_err(|e| format!("rebuild session index: {e}"))
}

// ---------------------------------------------------------------------------
// Session debugger — chunks, linked tools, subagents, phases, context,
// export, search, worktree grouping. All read-only.
// ---------------------------------------------------------------------------

/// Chunked event stream plus per-chunk linked tools — the shape the
/// Sessions transcript renders from.
///
/// `async fn` because it parses the full JSONL via `load_detail_by_path`.
#[tauri::command]
pub async fn session_chunks(
    file_path: String,
) -> Result<Vec<crate::dto::SessionChunkDto>, String> {
    let detail = load_detail_by_path(&file_path)?;
    let chunks = claudepot_core::session_chunks::build_chunks(&detail.events);
    Ok(chunks.iter().map(crate::dto::SessionChunkDto::from).collect())
}

/// Visible-context token attribution across six categories.
///
/// `async fn` because it parses the full JSONL via `load_detail_by_path`.
#[tauri::command]
pub async fn session_context_attribution(
    file_path: String,
) -> Result<crate::dto::ContextStatsDto, String> {
    let detail = load_detail_by_path(&file_path)?;
    let stats = claudepot_core::session_context::attribute_context(&detail.events);
    Ok((&stats).into())
}

/// Export transcript to Markdown or JSON (sk-ant-* redacted). Kept as
/// an internal helper for `session_export_to_file` — not exposed
/// separately until the UI has a "copy to clipboard" flow that needs
/// the raw body.
fn session_export_text(file_path: String, format: String) -> Result<String, String> {
    let detail = load_detail_by_path(&file_path)?;
    let fmt = match format.as_str() {
        "md" | "markdown" => claudepot_core::session_export::ExportFormat::Markdown,
        "json" => claudepot_core::session_export::ExportFormat::Json,
        other => return Err(format!("unknown format: {other}")),
    };
    Ok(claudepot_core::session_export::export(&detail, fmt))
}

/// Export transcript directly to disk. The UI hands us an absolute
/// path chosen by the user via the native save dialog; we validate,
/// then create the file atomically with restrictive permissions.
///
/// Boundary checks:
/// * `output_path` must be absolute and may not contain any `..`
///   component (defence against UI-side bugs that would allow a
///   compromised webview to write outside the user's chosen dir).
/// * The file is created with `CREATE | TRUNCATE` and — on Unix —
///   an O_NOFOLLOW-like intent enforced by `OpenOptions.mode(0o600)`
///   *before* any bytes are written, so the window where the file
///   could be world-readable is closed.
/// * A pre-existing symlink at `output_path` is refused; if the user
///   really wants to overwrite a symlink target they can delete it
///   first.
/// * Chmod failure after the fact is treated as fatal (we'd otherwise
///   fail open on a filesystem that silently ignored the mode bits).
#[tauri::command]
pub fn session_export_to_file(
    file_path: String,
    format: String,
    output_path: String,
) -> Result<usize, String> {
    let output = std::path::Path::new(&output_path);
    if !output.is_absolute() {
        return Err(format!("output path must be absolute: {output_path}"));
    }
    if output
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "output path must not contain `..`: {output_path}"
        ));
    }
    // Refuse to overwrite a symlink — the user's chosen filesystem
    // might resolve to somewhere unexpected under our permissions.
    match std::fs::symlink_metadata(output) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "refusing to overwrite symlink: {output_path}"
            ));
        }
        _ => {}
    }

    let body = session_export_text(file_path, format)?;

    // Atomic write: render into a sibling temp file, fsync, then
    // rename into place. On Unix `rename(2)` is atomic within the same
    // filesystem. If we crash mid-write the user still sees the
    // previous file (or no file) — never a half-written transcript.
    let parent = output
        .parent()
        .ok_or_else(|| format!("output has no parent directory: {output_path}"))?;
    let final_name = output
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("output has no filename: {output_path}"))?;

    // Unique per-call suffix so concurrent exports don't stomp each other.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(".{final_name}.claudepot-tmp-{nonce}"));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // create_new + mode == file is born with 0600 on filesystems
        // that honor it. Filesystems that silently ignore mode still
        // benefit from the `unreadable-until-rename` property via umask,
        // and the post-write chmod fallback below catches the rest.
        opts.mode(0o600);
    }
    let mut file = opts
        .open(&tmp_path)
        .map_err(|e| format!("open tmp {}: {e}", tmp_path.display()))?;

    use std::io::Write as _;
    if let Err(e) = (|| -> std::io::Result<()> {
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })() {
        // Best-effort cleanup; ignore secondary errors.
        drop(file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("write tmp {}: {e}", tmp_path.display()));
    }
    drop(file);

    // Belt-and-braces permission check before the rename. If we can't
    // enforce 0600, delete the tmp file and refuse the export.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&tmp_path)
            .map_err(|e| format!("stat tmp: {e}"))?;
        if meta.permissions().mode() & 0o077 != 0 {
            if let Err(e) = std::fs::set_permissions(
                &tmp_path,
                std::fs::Permissions::from_mode(0o600),
            ) {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!("chmod tmp: {e}"));
            }
            let mode2 = std::fs::metadata(&tmp_path)
                .map_err(|e| format!("re-stat tmp: {e}"))?
                .permissions()
                .mode();
            if mode2 & 0o077 != 0 {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!(
                    "filesystem does not enforce 0600 permissions at {output_path}"
                ));
            }
        }
    }

    // Rename into place. Atomic on POSIX when src + dst are on the
    // same filesystem; Windows' `rename` is also atomic per MSFT docs
    // on the same volume. We prepared `tmp_path` in `parent`, so this
    // is always same-filesystem.
    if let Err(e) = std::fs::rename(&tmp_path, output) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("rename into {output_path}: {e}"));
    }

    Ok(body.len())
}

/// Cross-session text search. Returns up to `limit` hits.
///
/// `async fn` is mandatory here. The body opens every `.jsonl` that
/// doesn't match via the row-level fast path and scans line by line —
/// for a multi-thousand-session corpus this is many seconds of pure
/// blocking I/O. Run on Tauri's main thread (the default for sync
/// commands) it would freeze the OS event loop and the webview for
/// the duration; under `async fn` Tauri dispatches to a Tokio worker
/// and the webview keeps repainting. See `session_list_all` for the
/// same rationale.
#[tauri::command]
pub async fn session_search(
    query: String,
    limit: Option<usize>,
) -> Result<Vec<crate::dto::SearchHitDto>, String> {
    let cfg = paths::claude_config_dir();
    let rows = claudepot_core::session::list_all_sessions(&cfg)
        .map_err(|e| format!("list sessions: {e}"))?;
    let hits =
        claudepot_core::session_search::search_rows(&rows, &query, limit.unwrap_or(25))
            .map_err(|e| format!("search sessions: {e}"))?;
    Ok(hits.iter().map(crate::dto::SearchHitDto::from).collect())
}

/// Group all sessions by git repository (collapses worktrees into a
/// single repository row).
///
/// `async fn` for the same reason as `session_list_all` — this calls
/// `list_all_sessions` itself, then runs a pure-Rust grouping pass.
/// Sync dispatch would block the main thread for the SQLite read /
/// JSONL fallback.
#[tauri::command]
pub async fn session_worktree_groups() -> Result<Vec<crate::dto::RepositoryGroupDto>, String> {
    let cfg = paths::claude_config_dir();
    let rows = claudepot_core::session::list_all_sessions(&cfg)
        .map_err(|e| format!("list sessions: {e}"))?;
    let groups = claudepot_core::session_worktree::group_by_repo(rows);
    Ok(groups
        .iter()
        .map(crate::dto::RepositoryGroupDto::from)
        .collect())
}

fn load_detail_by_path(
    file_path: &str,
) -> Result<claudepot_core::session::SessionDetail, String> {
    let cfg = paths::claude_config_dir();
    claudepot_core::session::read_session_detail_at_path(
        &cfg,
        std::path::Path::new(file_path),
    )
    .map_err(|e| format!("session read failed: {e}"))
}

// ---------------------------------------------------------------------------
// Protected paths — Settings → Protected pane
// ---------------------------------------------------------------------------

/// Materialized list (defaults minus removed_defaults, plus user
/// entries). UI renders this directly.
#[tauri::command]
pub fn protected_paths_list() -> Result<Vec<ProtectedPathDto>, String> {
    let dir = paths::claudepot_data_dir();
    let list = claudepot_core::protected_paths::list(&dir)
        .map_err(|e| format!("protected paths list failed: {e}"))?;
    Ok(list.iter().map(ProtectedPathDto::from).collect())
}

/// Add a path. Returns the materialized entry (so the UI knows which
/// badge — default-revived vs new user — to render). Validation is in
/// core; map errors to user-facing strings here.
#[tauri::command]
pub fn protected_paths_add(path: String) -> Result<ProtectedPathDto, String> {
    let dir = paths::claudepot_data_dir();
    let added = claudepot_core::protected_paths::add(&dir, &path)
        .map_err(|e| format!("{e}"))?;
    Ok(ProtectedPathDto::from(&added))
}

/// Remove a path. Defaults are tombstoned; user entries are dropped.
#[tauri::command]
pub fn protected_paths_remove(path: String) -> Result<(), String> {
    let dir = paths::claudepot_data_dir();
    claudepot_core::protected_paths::remove(&dir, &path)
        .map_err(|e| format!("{e}"))
}

/// Restore the implicit defaults — clears both `removed_defaults` and
/// `user`. Returns the resulting materialized list so the UI can
/// refresh in one round-trip.
#[tauri::command]
pub fn protected_paths_reset() -> Result<Vec<ProtectedPathDto>, String> {
    let dir = paths::claudepot_data_dir();
    claudepot_core::protected_paths::reset(&dir)
        .map_err(|e| format!("protected paths reset failed: {e}"))?;
    let list = claudepot_core::protected_paths::list(&dir)
        .map_err(|e| format!("protected paths list failed: {e}"))?;
    Ok(list.iter().map(ProtectedPathDto::from).collect())
}

// ---------------------------------------------------------------------------
// Preferences — Settings → General pane
// ---------------------------------------------------------------------------

/// Read the current preferences snapshot. Cheap — a clone of the
/// mutex-guarded record.
#[tauri::command]
pub fn preferences_get(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
) -> Result<crate::preferences::Preferences, String> {
    Ok(state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?
        .clone())
}

/// Set the complete `activity_*` preference block in one call.
/// Takes an optional value for each field so the webview can flip
/// one toggle without re-sending the others (e.g. flipping
/// `activity_enabled` from the consent modal). Returns the
/// refreshed snapshot so the UI stays in sync.
#[tauri::command]
pub fn preferences_set_activity(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    live: tauri::State<'_, crate::state::LiveSessionState>,
    enabled: Option<bool>,
    consent_seen: Option<bool>,
    hide_thinking: Option<bool>,
    excluded_paths: Option<Vec<String>>,
) -> Result<crate::preferences::Preferences, String> {
    let mut prefs = state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?;
    if let Some(v) = enabled {
        prefs.activity_enabled = v;
    }
    if let Some(v) = consent_seen {
        prefs.activity_consent_seen = v;
    }
    if let Some(v) = hide_thinking {
        prefs.activity_hide_thinking = v;
    }
    if let Some(v) = excluded_paths {
        prefs.activity_excluded_paths = v.clone();
        // Propagate to the running runtime so the change takes
        // effect on the next tick instead of requiring a restart.
        // `set_excluded_paths` is async, so we fire-and-forget via
        // the tauri async runtime handle; the command itself stays
        // sync to keep its signature minimal.
        let runtime = live.runtime.clone();
        tauri::async_runtime::spawn(async move {
            runtime.set_excluded_paths(v).await;
        });
    }
    prefs.save()?;
    Ok(prefs.clone())
}

/// Set the `notify_*` preference block in one call. Same "optional
/// per field" shape as `preferences_set_activity` — callers send
/// only the fields they changed.
#[tauri::command]
pub fn preferences_set_notifications(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    on_error: Option<bool>,
    on_idle_done: Option<bool>,
    on_stuck_minutes: Option<Option<u32>>,
    on_spend_usd: Option<Option<f32>>,
) -> Result<crate::preferences::Preferences, String> {
    let mut prefs = state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?;
    if let Some(v) = on_error {
        prefs.notify_on_error = v;
    }
    if let Some(v) = on_idle_done {
        prefs.notify_on_idle_done = v;
    }
    if let Some(v) = on_stuck_minutes {
        prefs.notify_on_stuck_minutes = v;
    }
    if let Some(v) = on_spend_usd {
        prefs.notify_on_spend_usd = v;
    }
    prefs.save()?;
    Ok(prefs.clone())
}

/// Toggle the dock-icon visibility (macOS only). On non-macOS platforms
/// the call still persists the boolean so the UI round-trips cleanly,
/// but the activation policy is a no-op.
#[tauri::command]
pub fn preferences_set_hide_dock_icon(
    #[allow(unused_variables)] app: tauri::AppHandle,
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    hide: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let policy = if hide {
            tauri::ActivationPolicy::Accessory
        } else {
            tauri::ActivationPolicy::Regular
        };
        app.set_activation_policy(policy)
            .map_err(|e| format!("set_activation_policy: {e}"))?;
    }
    let mut p = state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?;
    p.hide_dock_icon = hide;
    p.save()
}

// ---------------------------------------------------------------------------
// Keys — ANTHROPIC_API_KEY + CLAUDE_CODE_OAUTH_TOKEN management.
//
// Metadata lives in `~/.claudepot/keys.db`; secrets live in the OS
// keychain via the `keyring` crate. The plaintext token crosses the
// Tauri bridge ONLY on deliberate `*_copy` / `*_add` — never on list,
// probe, or usage fetch.
// ---------------------------------------------------------------------------

use crate::dto::{ApiKeySummaryDto, OauthTokenSummaryDto};
use claudepot_core::keys::{
    classify_token, delete_api_secret, delete_oauth_secret, read_api_secret,
    read_oauth_secret, token_preview, write_api_secret, write_oauth_secret, KeyPrefix,
    KeyStore, OAUTH_TOKEN_VALIDITY_DAYS,
};

fn open_keys_store() -> Result<KeyStore, String> {
    let db = paths::claudepot_data_dir().join("keys.db");
    KeyStore::open(&db).map_err(|e| format!("keys store open failed: {e}"))
}

/// Build `uuid → email` from a single `accounts.list()` call so N-row
/// key tables don't fire N SELECTs when rendering the Keys section.
fn account_email_map(
    store: &AccountStore,
) -> Result<HashMap<Uuid, String>, String> {
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    Ok(accounts.into_iter().map(|a| (a.uuid, a.email)).collect())
}

fn parse_account_uuid(s: &str, email_map: &HashMap<Uuid, String>) -> Result<Uuid, String> {
    let id = Uuid::parse_str(s).map_err(|_| format!("invalid account uuid: {s}"))?;
    if !email_map.contains_key(&id) {
        return Err(format!("no registered account with uuid {s}"));
    }
    Ok(id)
}

fn oauth_summary(
    t: claudepot_core::keys::OauthToken,
    email_map: &HashMap<Uuid, String>,
) -> OauthTokenSummaryDto {
    let expires_at = t.created_at + chrono::Duration::days(OAUTH_TOKEN_VALIDITY_DAYS);
    // Ceil the remaining time so a token with 12 hours left reads
    // "1d left" instead of collapsing to 0 and tripping the UI's
    // `<= 0` Expired check. Past-expiry deltas stay negative.
    let secs_remaining = (expires_at - chrono::Utc::now()).num_seconds();
    let days_remaining = if secs_remaining > 0 {
        (secs_remaining + 86_399) / 86_400
    } else {
        secs_remaining / 86_400
    };
    OauthTokenSummaryDto {
        account_email: email_map.get(&t.account_uuid).cloned(),
        uuid: t.uuid.to_string(),
        label: t.label,
        token_preview: t.token_preview,
        account_uuid: t.account_uuid.to_string(),
        created_at: t.created_at,
        expires_at,
        days_remaining,
        last_probed_at: t.last_probed_at,
        last_probe_status: t.last_probe_status,
    }
}

fn api_summary(
    k: claudepot_core::keys::ApiKey,
    email_map: &HashMap<Uuid, String>,
) -> ApiKeySummaryDto {
    ApiKeySummaryDto {
        account_email: email_map.get(&k.account_uuid).cloned(),
        uuid: k.uuid.to_string(),
        label: k.label,
        token_preview: k.token_preview,
        account_uuid: k.account_uuid.to_string(),
        created_at: k.created_at,
        last_probed_at: k.last_probed_at,
        last_probe_status: k.last_probe_status,
    }
}

#[tauri::command]
pub fn key_api_list() -> Result<Vec<ApiKeySummaryDto>, String> {
    let keys = open_keys_store()?;
    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let rows = keys
        .list_api_keys()
        .map_err(|e| format!("list api keys: {e}"))?;
    Ok(rows.into_iter().map(|k| api_summary(k, &email_map)).collect())
}

#[tauri::command]
pub fn key_oauth_list() -> Result<Vec<OauthTokenSummaryDto>, String> {
    let keys = open_keys_store()?;
    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let rows = keys
        .list_oauth_tokens()
        .map_err(|e| format!("list oauth tokens: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|t| oauth_summary(t, &email_map))
        .collect())
}

/// Add an `ANTHROPIC_API_KEY`. `account_uuid` is required — every key
/// was created under some account, and recording that makes the row
/// findable by account later. The "no account" case isn't a real state
/// we need to model.
#[tauri::command]
pub fn key_api_add(
    label: String,
    token: String,
    account_uuid: String,
) -> Result<ApiKeySummaryDto, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    let token = token.trim();
    if !matches!(classify_token(token), Some(KeyPrefix::ApiKey)) {
        return Err(
            "not an API key — expected a value starting with `sk-ant-api03-`".to_string(),
        );
    }

    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let account_id = parse_account_uuid(&account_uuid, &email_map)?;

    let preview = token_preview(token);
    let keys = open_keys_store()?;
    let row = keys
        .insert_api_key(label, &preview, account_id)
        .map_err(|e| format!("insert: {e}"))?;

    // Secret goes to the keychain. If this fails, tear the row back
    // out so the DB never shows a key whose value we can't recover.
    // If the tear-out also fails, surface it — an orphan row the user
    // can see (preview, no secret) is worse when it's silent.
    if let Err(e) = write_api_secret(row.uuid, token) {
        if let Err(rollback) = keys.remove_api_key(row.uuid) {
            tracing::error!(
                uuid = %row.uuid,
                keychain_error = %e,
                rollback_error = %rollback,
                "api key keychain write failed AND rollback failed — orphan row"
            );
            return Err(format!(
                "keychain write failed: {e}; rollback also failed: {rollback} — remove the orphan row manually"
            ));
        }
        return Err(format!("keychain write failed: {e}"));
    }
    Ok(api_summary(row, &email_map))
}

/// Add a `CLAUDE_CODE_OAUTH_TOKEN`. Account tag is required — the user
/// picks the account they ran `claude setup-token` against.
#[tauri::command]
pub fn key_oauth_add(
    label: String,
    token: String,
    account_uuid: String,
) -> Result<OauthTokenSummaryDto, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    let token = token.trim();
    if !matches!(classify_token(token), Some(KeyPrefix::OauthToken)) {
        return Err(
            "not an OAuth token — expected a value starting with `sk-ant-oat01-`".to_string(),
        );
    }

    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let account_id = parse_account_uuid(&account_uuid, &email_map)?;

    let preview = token_preview(token);
    let keys = open_keys_store()?;
    let row = keys
        .insert_oauth_token(label, &preview, account_id)
        .map_err(|e| format!("insert: {e}"))?;

    if let Err(e) = write_oauth_secret(row.uuid, token) {
        if let Err(rollback) = keys.remove_oauth_token(row.uuid) {
            tracing::error!(
                uuid = %row.uuid,
                keychain_error = %e,
                rollback_error = %rollback,
                "oauth token keychain write failed AND rollback failed — orphan row"
            );
            return Err(format!(
                "keychain write failed: {e}; rollback also failed: {rollback} — remove the orphan row manually"
            ));
        }
        return Err(format!("keychain write failed: {e}"));
    }
    Ok(oauth_summary(row, &email_map))
}

#[tauri::command]
pub fn key_api_remove(uuid: String) -> Result<(), String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    // Keychain first — if the DB row is gone but the secret lingers,
    // we have a stranded keychain item the user can't see or purge
    // without Keychain Access. The reverse orphan (DB row, no secret)
    // is at least visible and manually removable from the Keys list.
    delete_api_secret(id).map_err(|e| format!("keychain delete: {e}"))?;
    let keys = open_keys_store()?;
    keys.remove_api_key(id).map_err(|e| format!("{e}"))
}

#[tauri::command]
pub fn key_oauth_remove(uuid: String) -> Result<(), String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    delete_oauth_secret(id).map_err(|e| format!("keychain delete: {e}"))?;
    let keys = open_keys_store()?;
    keys.remove_oauth_token(id).map_err(|e| format!("{e}"))
}

/// Return the full API-key secret for clipboard copy. Deliberately
/// distinct from `key_api_list` which only returns the preview.
#[tauri::command]
pub fn key_api_copy(uuid: String) -> Result<String, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    read_api_secret(id).map_err(|e| format!("keychain read: {e}"))
}

#[tauri::command]
pub fn key_oauth_copy(uuid: String) -> Result<String, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    read_oauth_secret(id).map_err(|e| format!("keychain read: {e}"))
}

/// Call `/api/oauth/usage` with the stored token to verify it's still
/// valid and update the probe fields. Returns the refreshed summary.
/// A 401 sets status="unauthorized" (authoritative signal that the
/// token has been revoked or has expired ahead of our 365-day proxy).
#[tauri::command]
pub async fn key_oauth_probe(uuid: String) -> Result<OauthTokenSummaryDto, String> {
    use claudepot_core::error::OAuthError;

    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let token = read_oauth_secret(id).map_err(|e| format!("keychain read: {e}"))?;

    let status = match claudepot_core::oauth::usage::fetch(&token).await {
        Ok(_) => "ok".to_string(),
        Err(OAuthError::AuthFailed(_)) => "unauthorized".to_string(),
        Err(OAuthError::RateLimited { retry_after_secs }) => {
            format!("rate_limited:{retry_after_secs}")
        }
        Err(e) => format!("error:{e}"),
    };

    let keys = open_keys_store()?;
    keys.update_oauth_token_probe(id, &status)
        .map_err(|e| format!("update probe: {e}"))?;

    let row = keys
        .find_oauth_token(id)
        .map_err(|e| format!("reload: {e}"))?
        .ok_or_else(|| "token disappeared after probe".to_string())?;
    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    Ok(oauth_summary(row, &email_map))
}

/// Fetch the full usage breakdown for a stored OAuth token. Mirrors
/// `fetch_all_usage` / `refresh_usage_for` for accounts, but keyed on
/// a Keys-section row instead of an Account. Also updates the token's
/// probe status as a side effect so the days-left chip reflects the
/// latest known state without a second round-trip.
#[tauri::command]
pub async fn key_oauth_usage(
    uuid: String,
) -> Result<crate::dto::AccountUsageDto, String> {
    use claudepot_core::error::OAuthError;

    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let token = read_oauth_secret(id).map_err(|e| format!("keychain read: {e}"))?;

    let keys = open_keys_store()?;
    match claudepot_core::oauth::usage::fetch(&token).await {
        Ok(response) => {
            if let Err(dbe) = keys.update_oauth_token_probe(id, "ok") {
                // Non-fatal: the usage fetch succeeded, we just can't
                // persist the "ok" probe marker. Log at warn so the
                // days-left chip going stale isn't silent.
                tracing::warn!(
                    uuid = %id,
                    error = %dbe,
                    "key_oauth_usage: probe persist failed on success path"
                );
            }
            Ok(crate::dto::AccountUsageDto::from_response(&response))
        }
        Err(e) => {
            let status = match &e {
                OAuthError::AuthFailed(_) => "unauthorized".to_string(),
                OAuthError::RateLimited { retry_after_secs } => {
                    format!("rate_limited:{retry_after_secs}")
                }
                other => format!("error:{other}"),
            };
            if let Err(dbe) = keys.update_oauth_token_probe(id, &status) {
                tracing::warn!(
                    uuid = %id,
                    error = %dbe,
                    "key_oauth_usage: probe persist failed on error path"
                );
            }
            Err(format!("usage fetch failed: {e}"))
        }
    }
}

// ─── session_live commands ──────────────────────────────────────────
//
// The live Activity feature. `LiveRuntime` polls ~/.claude/sessions
// and tails each transcript; Tauri commands expose snapshot + subscribe
// semantics. Aggregate updates fire on the `live-all` event channel;
// per-session deltas fire on `live::<sessionId>`.

/// Start the live runtime. Idempotent: repeated calls after a first
/// successful start return `Ok(())` without re-spawning. The poll
/// loop publishes aggregate updates via the `live-all` event channel
/// and per-session deltas via `live::<sessionId>`.
///
/// **Consent gate — trust boundary**: the runtime only starts if the
/// user has explicitly enabled the Activity feature via the consent
/// modal or Settings. A request to start while `activity_enabled ==
/// false` returns `Ok(())` silently (so accidental callers don't
/// break) but the runtime stays off. The frontend check at the
/// consent-modal layer is backed up by this server-side guard so
/// future callers (e.g. a rogue hook or a CLI command) also respect
/// the user's choice.
#[tauri::command]
pub async fn session_live_start(
    state: tauri::State<'_, crate::state::LiveSessionState>,
    prefs: tauri::State<'_, crate::preferences::PreferencesState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Consent check MUST precede the started flag flip, or a user
    // who opted out after opting in would still get a running
    // runtime from a stale `started = true`.
    {
        let p = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
        if !p.activity_enabled {
            return Ok(());
        }
    }
    {
        let mut started = state.started.lock().map_err(|e| e.to_string())?;
        if *started {
            return Ok(());
        }
        *started = true;
    }

    // Spawn a watcher task that forwards aggregate updates to the
    // webview. The runtime publishes on the watch channel; we bridge
    // to Tauri's `emit` here so the webview sees one source of truth.
    //
    // Same task also schedules a debounced tray rebuild on live-set
    // changes so the tray's "Active: N" submenu stays in sync with
    // the sidebar strip. Debounce = 1s: at the 500ms poll cadence
    // this coalesces bursts of "session appeared/disappeared" into
    // a single rebuild, which matters on AppKit where menu rebuilds
    // are synchronous and visible.
    // Apply the user's current excluded-paths preference to the
    // runtime before it starts ticking. Without this the first
    // tick could publish a now-excluded project's live state.
    // Read-and-release the sync prefs lock in a short scope so no
    // std::sync guard lives across the .await below.
    let excluded: Vec<String> = {
        let p = prefs
            .0
            .lock()
            .map_err(|e| format!("prefs lock: {e}"))?;
        p.activity_excluded_paths.clone()
    };
    state.runtime.set_excluded_paths(excluded).await;

    let runtime = std::sync::Arc::clone(&state.runtime);
    let mut rx = runtime.subscribe_aggregate();
    let app_for_bridge = app.clone();
    let aggregate_handle = tokio::spawn(async move {
        use tokio::sync::Mutex as AsyncMutex;
        // Track the last-seen set of session IDs so we only rebuild
        // the tray when membership changes, not on every status
        // transition (which don't affect the top-level menu).
        let mut last_ids: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        // True debounce: a single shared pending flag that flips
        // off only AFTER the delayed rebuild actually runs, so
        // repeat membership changes within the window don't
        // schedule a second rebuild. Earlier impl reset the flag
        // in the same loop iteration that scheduled — no debounce
        // at all.
        let rebuild_pending = std::sync::Arc::new(AsyncMutex::new(false));
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            let list_arc = rx.borrow_and_update().clone();
            let list: Vec<crate::dto::LiveSessionSummaryDto> = list_arc
                .iter()
                .cloned()
                .map(crate::dto::LiveSessionSummaryDto::from)
                .collect();
            let _ = tauri::Emitter::emit(&app_for_bridge, "live-all", list);

            // Tray-rebuild trigger: membership set changed.
            let current_ids: std::collections::BTreeSet<String> = list_arc
                .iter()
                .map(|s| s.session_id.clone())
                .collect();
            if current_ids != last_ids {
                last_ids = current_ids;
                let mut guard = rebuild_pending.lock().await;
                if !*guard {
                    *guard = true;
                    drop(guard);
                    let handle = app_for_bridge.clone();
                    let pending = rebuild_pending.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(
                            std::time::Duration::from_secs(1),
                        )
                        .await;
                        // Clear the pending flag ONLY after the
                        // delay fires so another membership change
                        // inside the window is folded into this
                        // rebuild instead of scheduling a new one.
                        if let Err(e) = crate::tray::rebuild(&handle).await {
                            tracing::warn!(
                                "activity tray rebuild failed: {e}"
                            );
                        }
                        let mut g = pending.lock().await;
                        *g = false;
                    });
                }
            }
        }
    });

    // Stash the aggregate-bridge handle so session_live_stop can
    // abort it — without this, start/stop/start cycles accumulate
    // zombie emitters.
    {
        let mut tasks = state
            .bridge_tasks
            .lock()
            .map_err(|e| format!("bridge lock: {e}"))?;
        tasks.aggregate = Some(aggregate_handle);
    }

    // Start the poll loop.
    let _handle = std::sync::Arc::clone(&state.runtime).start();
    Ok(())
}

/// Stop the live runtime. Idempotent: stopping an already-stopped
/// runtime is a no-op. Aborts every bridge task spawned by
/// `session_live_start` (aggregate → live-all, and each
/// per-session → live::<sid>) so a subsequent start begins from a
/// clean task set instead of accumulating ghost emitters.
#[tauri::command]
pub async fn session_live_stop(
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    state.runtime.stop();
    {
        let mut tasks = state
            .bridge_tasks
            .lock()
            .map_err(|e| format!("bridge lock: {e}"))?;
        tasks.abort_all();
    }
    let mut started = state.started.lock().map_err(|e| e.to_string())?;
    *started = false;
    Ok(())
}

/// Explicit unsubscribe for a per-session detail stream. The Tauri
/// event listener on the JS side has no way to tell the backend
/// "stop forwarding" without this — dropping the listener alone
/// leaves the spawned task alive until the session ends.
#[tauri::command]
pub async fn session_live_unsubscribe(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    // Abort the backend bridge task inside a short scope so the
    // std::sync Mutex guard is dropped before we .await.
    {
        let mut tasks = state
            .bridge_tasks
            .lock()
            .map_err(|e| format!("bridge lock: {e}"))?;
        if let Some(h) = tasks.details.remove(&session_id) {
            h.abort();
        }
    }
    // Drop the backend-side slot in the DetailBus so a future
    // subscribe rebuilds cleanly without an AlreadySubscribed error.
    state.runtime.detail_end_session(&session_id).await;
    Ok(())
}

/// Synchronous snapshot of currently-live sessions. Used by the
/// webview on first mount before the first `live-all` event arrives,
/// and as the resync answer after a gap.
#[tauri::command]
pub fn session_live_snapshot(
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Vec<crate::dto::LiveSessionSummaryDto> {
    state
        .runtime
        .snapshot()
        .iter()
        .cloned()
        .map(crate::dto::LiveSessionSummaryDto::from)
        .collect()
}

/// One-session snapshot for resync after `resync_required`. Returns
/// `None` if the session is not currently live.
#[tauri::command]
pub async fn session_live_session_snapshot(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<Option<crate::dto::LiveSessionSummaryDto>, String> {
    Ok(state
        .runtime
        .session_snapshot(&session_id)
        .await
        .map(crate::dto::LiveSessionSummaryDto::from))
}

/// Query the durable activity metrics store for the time-series
/// Trends view. Returns active-session counts bucketed across the
/// requested window plus a simple error-count total. An unavailable
/// metrics store (first-launch race, permission issue) returns an
/// all-zero series rather than an error — the Trends view is
/// non-critical.
#[tauri::command]
pub fn activity_trends(
    state: tauri::State<'_, crate::state::LiveSessionState>,
    from_ms: i64,
    to_ms: i64,
    bucket_count: u32,
) -> Result<crate::dto::ActivityTrendsDto, String> {
    let buckets = bucket_count as usize;
    let bucket_width = if buckets > 0 && to_ms > from_ms {
        (to_ms - from_ms) / buckets as i64
    } else {
        0
    };
    let Some(store) = state.runtime.metrics() else {
        return Ok(crate::dto::ActivityTrendsDto {
            from_ms,
            to_ms,
            bucket_width_ms: bucket_width,
            active_series: vec![0; buckets],
            error_count: 0,
        });
    };
    let active_series = store
        .active_series(from_ms, to_ms, buckets)
        .map_err(|e| format!("active_series: {e}"))?;
    let error_count = store
        .error_count(from_ms, to_ms)
        .map_err(|e| format!("error_count: {e}"))?;
    Ok(crate::dto::ActivityTrendsDto {
        from_ms,
        to_ms,
        bucket_width_ms: bucket_width,
        active_series,
        error_count,
    })
}

/// Subscribe to per-session deltas. Spawns a task that forwards
/// every received delta as a `live::<sessionId>` event. Single-
/// subscriber per session — concurrent calls with the same id
/// return `BusError::AlreadySubscribed`. The JS side should call
/// `session_live_unsubscribe` before dropping its listener so the
/// backend-side task doesn't outlive the frontend.
#[tauri::command]
pub async fn session_live_subscribe(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut rx = state
        .runtime
        .subscribe_detail(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    let channel = format!("live::{session_id}");
    let handle = tokio::spawn(async move {
        while let Some(delta) = rx.recv().await {
            let dto = crate::dto::LiveDeltaDto::from(delta);
            let _ = tauri::Emitter::emit(&app, &channel, dto);
        }
    });
    // Track the handle so session_live_unsubscribe (or _stop) can
    // abort it. Dropping the frontend listener alone keeps this
    // task alive until the session itself ends; with the explicit
    // unsubscribe path it goes away immediately.
    let mut tasks = state
        .bridge_tasks
        .lock()
        .map_err(|e| format!("bridge lock: {e}"))?;
    if let Some(prev) = tasks.details.insert(session_id, handle) {
        prev.abort();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Session prune / slim / trash (Tauri surface)
// ---------------------------------------------------------------------------

fn filter_from_dto(
    dto: crate::dto::PruneFilterDto,
) -> claudepot_core::session_prune::PruneFilter {
    claudepot_core::session_prune::PruneFilter {
        older_than: dto.older_than_secs.map(std::time::Duration::from_secs),
        larger_than: dto.larger_than_bytes,
        project: dto
            .project
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect(),
        has_error: dto.has_error,
        is_sidechain: dto.is_sidechain,
    }
}

#[tauri::command]
pub fn session_prune_plan(
    filter: crate::dto::PruneFilterDto,
) -> Result<crate::dto::PrunePlanDto, String> {
    let f = filter_from_dto(filter);
    let plan = claudepot_core::session_prune::plan_prune(&paths::claude_config_dir(), &f)
        .map_err(|e| format!("plan_prune: {e}"))?;
    Ok((&plan).into())
}

#[tauri::command]
pub fn session_prune_start(
    filter: crate::dto::PruneFilterDto,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let f = filter_from_dto(filter);
    let plan = claudepot_core::session_prune::plan_prune(&paths::claude_config_dir(), &f)
        .map_err(|e| format!("plan_prune: {e}"))?;
    let op_id = new_op_id();
    ops.insert(RunningOpInfo {
        op_id: op_id.clone(),
        kind: OpKind::SessionPrune,
        old_path: String::new(),
        new_path: String::new(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    });
    let app_c = app.clone();
    let ops_c = ops.inner().clone();
    let op_id_c = op_id.clone();
    std::thread::spawn(move || {
        let sink = TauriProgressSink {
            app: app_c.clone(),
            op_id: op_id_c.clone(),
            ops: ops_c.clone(),
        };
        let data_dir = paths::claudepot_data_dir();
        let res = claudepot_core::session_prune::execute_prune(&data_dir, &plan, &sink);
        match res {
            Ok(_report) => emit_terminal(&app_c, &ops_c, &op_id_c, None),
            Err(e) => emit_terminal(&app_c, &ops_c, &op_id_c, Some(e.to_string())),
        }
    });
    Ok(op_id)
}

#[tauri::command]
pub fn session_slim_plan(
    path: String,
    opts: crate::dto::SlimOptsDto,
) -> Result<crate::dto::SlimPlanDto, String> {
    let opts = claudepot_core::session_slim::SlimOpts {
        drop_tool_results_over_bytes: opts.drop_tool_results_over_bytes,
        exclude_tools: opts.exclude_tools,
        strip_images: opts.strip_images,
        strip_documents: opts.strip_documents,
    };
    let plan = claudepot_core::session_slim::plan_slim(std::path::Path::new(&path), &opts)
        .map_err(|e| format!("plan_slim: {e}"))?;
    Ok((&plan).into())
}

#[tauri::command]
pub fn session_slim_start(
    path: String,
    opts: crate::dto::SlimOptsDto,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let opts = claudepot_core::session_slim::SlimOpts {
        drop_tool_results_over_bytes: opts.drop_tool_results_over_bytes,
        exclude_tools: opts.exclude_tools,
        strip_images: opts.strip_images,
        strip_documents: opts.strip_documents,
    };
    let path_buf = std::path::PathBuf::from(&path);
    let op_id = new_op_id();
    ops.insert(RunningOpInfo {
        op_id: op_id.clone(),
        kind: OpKind::SessionSlim,
        old_path: path.clone(),
        new_path: path.clone(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    });
    let app_c = app.clone();
    let ops_c = ops.inner().clone();
    let op_id_c = op_id.clone();
    std::thread::spawn(move || {
        let sink = TauriProgressSink {
            app: app_c.clone(),
            op_id: op_id_c.clone(),
            ops: ops_c.clone(),
        };
        let data_dir = paths::claudepot_data_dir();
        let res = claudepot_core::session_slim::execute_slim(
            &data_dir,
            &path_buf,
            &opts,
            &sink,
        );
        match res {
            Ok(_report) => emit_terminal(&app_c, &ops_c, &op_id_c, None),
            Err(e) => emit_terminal(&app_c, &ops_c, &op_id_c, Some(e.to_string())),
        }
    });
    Ok(op_id)
}

#[tauri::command]
pub fn session_slim_plan_all(
    filter: crate::dto::PruneFilterDto,
    opts: crate::dto::SlimOptsDto,
) -> Result<crate::dto::BulkSlimPlanDto, String> {
    let filter = claudepot_core::session_prune::PruneFilter {
        older_than: filter.older_than_secs.map(std::time::Duration::from_secs),
        larger_than: filter.larger_than_bytes,
        project: filter.project.into_iter().map(std::path::PathBuf::from).collect(),
        has_error: filter.has_error,
        is_sidechain: filter.is_sidechain,
    };
    let opts = claudepot_core::session_slim::SlimOpts {
        drop_tool_results_over_bytes: opts.drop_tool_results_over_bytes,
        exclude_tools: opts.exclude_tools,
        strip_images: opts.strip_images,
        strip_documents: opts.strip_documents,
    };
    let config_dir = paths::claude_config_dir();
    let plan = claudepot_core::session_slim::plan_slim_all(&config_dir, &filter, &opts)
        .map_err(|e| format!("plan_slim_all: {e}"))?;
    Ok((&plan).into())
}

#[tauri::command]
pub fn session_slim_start_all(
    filter: crate::dto::PruneFilterDto,
    opts: crate::dto::SlimOptsDto,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let filter = claudepot_core::session_prune::PruneFilter {
        older_than: filter.older_than_secs.map(std::time::Duration::from_secs),
        larger_than: filter.larger_than_bytes,
        project: filter.project.into_iter().map(std::path::PathBuf::from).collect(),
        has_error: filter.has_error,
        is_sidechain: filter.is_sidechain,
    };
    let opts = claudepot_core::session_slim::SlimOpts {
        drop_tool_results_over_bytes: opts.drop_tool_results_over_bytes,
        exclude_tools: opts.exclude_tools,
        strip_images: opts.strip_images,
        strip_documents: opts.strip_documents,
    };
    let op_id = new_op_id();
    ops.insert(RunningOpInfo {
        op_id: op_id.clone(),
        kind: OpKind::SessionSlim,
        old_path: "--all".to_string(),
        new_path: "--all".to_string(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    });
    let app_c = app.clone();
    let ops_c = ops.inner().clone();
    let op_id_c = op_id.clone();
    std::thread::spawn(move || {
        let sink = TauriProgressSink {
            app: app_c.clone(),
            op_id: op_id_c.clone(),
            ops: ops_c.clone(),
        };
        let config_dir = paths::claude_config_dir();
        let data_dir = paths::claudepot_data_dir();
        let plan = match claudepot_core::session_slim::plan_slim_all(&config_dir, &filter, &opts) {
            Ok(p) => p,
            Err(e) => {
                emit_terminal(&app_c, &ops_c, &op_id_c, Some(e.to_string()));
                return;
            }
        };
        let report =
            claudepot_core::session_slim::execute_slim_all(&data_dir, &plan, &opts, &sink);
        // Propagate per-file failures to the UI. Planning errors
        // (rows that couldn't be scanned) and execute failures are
        // both reported — a partial success with any failures is not
        // a clean `complete`.
        let mut problems: Vec<String> = Vec::new();
        for (p, e) in &plan.failed_to_plan {
            problems.push(format!("plan {}: {e}", p.display()));
        }
        for (p, e) in &report.failed {
            problems.push(format!("exec {}: {e}", p.display()));
        }
        let err_msg = if problems.is_empty() {
            None
        } else {
            Some(format!(
                "bulk slim finished with {} success, {} skipped, {} failed:\n{}",
                report.succeeded.len(),
                report.skipped_live.len(),
                problems.len(),
                problems.join("\n"),
            ))
        };
        emit_terminal(&app_c, &ops_c, &op_id_c, err_msg);
    });
    Ok(op_id)
}

#[tauri::command]
pub fn session_trash_list(
    older_than_secs: Option<u64>,
) -> Result<crate::dto::TrashListingDto, String> {
    let filter = claudepot_core::trash::TrashFilter {
        older_than: older_than_secs.map(std::time::Duration::from_secs),
        kind: None,
    };
    let listing = claudepot_core::trash::list(&paths::claudepot_data_dir(), filter)
        .map_err(|e| format!("trash list: {e}"))?;
    Ok((&listing).into())
}

#[tauri::command]
pub fn session_trash_restore(
    entry_id: String,
    override_cwd: Option<String>,
) -> Result<String, String> {
    let cwd = override_cwd.as_deref().map(std::path::Path::new);
    let restored =
        claudepot_core::trash::restore(&paths::claudepot_data_dir(), &entry_id, cwd)
            .map_err(|e| format!("trash restore: {e}"))?;
    Ok(restored.to_string_lossy().to_string())
}

#[tauri::command]
pub fn session_trash_empty(older_than_secs: Option<u64>) -> Result<u64, String> {
    let filter = claudepot_core::trash::TrashFilter {
        older_than: older_than_secs.map(std::time::Duration::from_secs),
        kind: None,
    };
    claudepot_core::trash::empty(&paths::claudepot_data_dir(), filter)
        .map_err(|e| format!("trash empty: {e}"))
}

// ---------------------------------------------------------------------------
// Session export (preview, clipboard, gist) + GitHub token management
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct RedactionPolicyDto {
    #[serde(default = "default_true")]
    pub anthropic_keys: bool,
    #[serde(default)]
    pub paths: Option<PathStrategyDto>,
    #[serde(default)]
    pub emails: bool,
    #[serde(default)]
    pub env_assignments: bool,
    #[serde(default)]
    pub custom_regex: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PathStrategyDto {
    Off,
    Relative { root: String },
    Hash,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportFormatDto {
    Markdown,
    MarkdownSlim,
    Json,
    Html {
        #[serde(default)]
        no_js: bool,
    },
}

fn policy_from_dto(p: Option<RedactionPolicyDto>) -> claudepot_core::redaction::RedactionPolicy {
    use claudepot_core::redaction::{PathStrategy, RedactionPolicy};
    match p {
        None => RedactionPolicy::default(),
        Some(dto) => RedactionPolicy {
            anthropic_keys: dto.anthropic_keys,
            paths: match dto.paths {
                Some(PathStrategyDto::Off) | None => PathStrategy::Off,
                Some(PathStrategyDto::Relative { root }) => PathStrategy::Relative {
                    root: std::path::PathBuf::from(root),
                },
                Some(PathStrategyDto::Hash) => PathStrategy::Hash,
            },
            emails: dto.emails,
            env_assignments: dto.env_assignments,
            custom_regex: dto.custom_regex,
        },
    }
}

fn format_from_dto(f: ExportFormatDto) -> claudepot_core::session_export::ExportFormat {
    use claudepot_core::session_export::ExportFormat;
    match f {
        ExportFormatDto::Markdown => ExportFormat::Markdown,
        ExportFormatDto::MarkdownSlim => ExportFormat::MarkdownSlim,
        ExportFormatDto::Json => ExportFormat::Json,
        ExportFormatDto::Html { no_js } => ExportFormat::Html { no_js },
    }
}

/// File extension matching the requested export format. Used by gist
/// uploads so the uploaded file is named with the right suffix.
fn export_extension(f: &ExportFormatDto) -> &'static str {
    match f {
        ExportFormatDto::Markdown | ExportFormatDto::MarkdownSlim => "md",
        ExportFormatDto::Json => "json",
        ExportFormatDto::Html { .. } => "html",
    }
}

fn resolve_session_detail(
    target: &str,
) -> Result<claudepot_core::session::SessionDetail, String> {
    let cfg = paths::claude_config_dir();
    if target.ends_with(".jsonl") {
        let p = std::path::PathBuf::from(target);
        return claudepot_core::session::read_session_detail_at_path(&cfg, &p)
            .map_err(|e| format!("read session: {e}"));
    }
    claudepot_core::session::read_session_detail(&cfg, target)
        .map_err(|e| format!("read session: {e}"))
}

#[tauri::command]
pub fn session_export_preview(
    target: String,
    format: ExportFormatDto,
    policy: Option<RedactionPolicyDto>,
) -> Result<String, String> {
    let detail = resolve_session_detail(&target)?;
    let fmt = format_from_dto(format);
    let pol = policy_from_dto(policy);
    Ok(claudepot_core::session_export::export_preview(&detail, fmt, &pol))
}

#[tauri::command]
pub fn session_share_gist_start(
    target: String,
    format: ExportFormatDto,
    policy: Option<RedactionPolicyDto>,
    public: bool,
    app: AppHandle,
    ops: State<RunningOps>,
) -> Result<String, String> {
    let detail = resolve_session_detail(&target)?;
    let ext = export_extension(&format);
    let fmt = format_from_dto(format);
    let pol = policy_from_dto(policy);
    let body = claudepot_core::session_export::export_with(&detail, fmt, &pol);
    let filename = format!("session-{}.{}", detail.row.session_id, ext);
    let description = format!("Claudepot session export: {}", detail.row.session_id);
    let token = github_token_for_upload()?;
    let op_id = new_op_id();
    ops.insert(RunningOpInfo {
        op_id: op_id.clone(),
        kind: OpKind::SessionShare,
        old_path: detail.row.session_id.clone(),
        new_path: String::new(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
    });
    let app_c = app.clone();
    let ops_c = ops.inner().clone();
    let op_id_c = op_id.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                emit_terminal(&app_c, &ops_c, &op_id_c, Some(e.to_string()));
                return;
            }
        };
        let sink = TauriProgressSink {
            app: app_c.clone(),
            op_id: op_id_c.clone(),
            ops: ops_c.clone(),
        };
        let res = rt.block_on(claudepot_core::session_share::share_gist(
            &body,
            &filename,
            &description,
            public,
            &token,
            &sink,
        ));
        match res {
            Ok(_) => emit_terminal(&app_c, &ops_c, &op_id_c, None),
            // `ShareError::Display` is already token-scrubbed.
            Err(e) => emit_terminal(&app_c, &ops_c, &op_id_c, Some(e.to_string())),
        }
    });
    Ok(op_id)
}

const GH_TOKEN_SERVICE: &str = "claudepot";
const GH_TOKEN_ENTRY: &str = "github-token";

/// Token used by gist uploads: env var wins over keychain, same as
/// the CLI. Kept private — the settings UI never sees this directly;
/// it operates on the keychain slot only, so Save/Clear aren't
/// silent no-ops when the env var is also set.
fn github_token_for_upload() -> Result<String, String> {
    if let Ok(v) = std::env::var("GITHUB_TOKEN") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    entry
        .get_password()
        .map_err(|_| "no GitHub token stored".to_string())
}

/// Read only the keychain-backed token. Returns `None` when absent.
fn github_token_keychain_read() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(_) => Ok(None),
    }
}

#[derive(serde::Serialize)]
pub struct GithubTokenStatus {
    /// True iff a value lives in the Claudepot keychain slot.
    pub present: bool,
    /// Last four chars of the keychain value, if present.
    pub last4: Option<String>,
    /// True when `GITHUB_TOKEN` env var is set — the CLI and the
    /// gist uploader both prefer it over the keychain value. The UI
    /// can surface this so users understand why "Clear" didn't take
    /// effect for an upload.
    pub env_override: bool,
}

fn last4_of(s: &str) -> Option<String> {
    if s.len() >= 4 {
        Some(s[s.len() - 4..].to_string())
    } else if !s.is_empty() {
        Some(s.to_string())
    } else {
        None
    }
}

#[tauri::command]
pub fn settings_github_token_get() -> Result<GithubTokenStatus, String> {
    let env_override = std::env::var("GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    match github_token_keychain_read()? {
        Some(t) => Ok(GithubTokenStatus {
            present: true,
            last4: last4_of(&t),
            env_override,
        }),
        None => Ok(GithubTokenStatus {
            present: false,
            last4: None,
            env_override,
        }),
    }
}

#[tauri::command]
pub fn settings_github_token_set(value: String) -> Result<GithubTokenStatus, String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    entry
        .set_password(&value)
        .map_err(|e| format!("keychain set: {e}"))?;
    let env_override = std::env::var("GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    Ok(GithubTokenStatus {
        present: true,
        last4: last4_of(&value),
        env_override,
    })
}

#[tauri::command]
pub fn settings_github_token_clear() -> Result<(), String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    // Delete is a best-effort; not-found is fine.
    let _ = entry.delete_credential();
    Ok(())
}
