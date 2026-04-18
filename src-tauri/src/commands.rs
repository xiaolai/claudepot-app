//! Tauri command layer — thin async wrappers over `claudepot-core`.
//!
//! Per `.claude/rules/architecture.md`, NO business logic lives here. Each
//! command opens the store, calls a core function, and serializes the result.
//! Errors become user-facing strings at this boundary.

use crate::dto::{
    AccountSummary, AppStatus, CcIdentity, CleanPreviewDto, DryRunPlanDto,
    JournalEntryDto, MoveArgsDto, ProjectDetailDto, ProjectInfoDto,
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
    let cfg = paths::claude_config_dir();
    let base = cfg.join("claudepot");
    (base.join("journals"), base.join("locks"), base.join("snapshots"))
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
    let snapshots_dir = Some(cfg.join("claudepot").join("snapshots"));
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
    Ok(CleanPreviewDto {
        orphans: orphans.iter().map(ProjectInfoDto::from).collect(),
        orphans_found: result.orphans_found,
        unreachable_skipped: result.unreachable_skipped,
        total_bytes,
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
        let result = project::clean_orphans_with_progress(
            &cfg,
            claude_json.as_deref(),
            Some(snaps_for_task.as_path()),
            Some(locks_for_task.as_path()),
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
        let snaps = Some(cfg.join("claudepot").join("snapshots"));
        let result = match kind {
            OpKind::RepairResume => {
                project_repair::resume(&entry, cfg, claude_json, snaps, &sink)
            }
            OpKind::RepairRollback => {
                project_repair::rollback(&entry, cfg, claude_json, snaps, &sink)
            }
            OpKind::MoveProject | OpKind::CleanProjects => {
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
    let snaps = Some(cfg.join("claudepot").join("snapshots"));
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
