//! Tauri command layer — thin async wrappers over `claudepot-core`.
//!
//! Per `.claude/rules/architecture.md`, NO business logic lives here. Each
//! command opens the store, calls a core function, and serializes the result.
//! Errors become user-facing strings at this boundary.

use crate::dto::{
    AccountSummary, AccountUsageDto, AppStatus, CcIdentity, DryRunPlanDto, JournalEntryDto,
    MoveArgsDto, ProjectDetailDto, ProjectInfoDto, RegisterOutcome, RemoveOutcome,
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
            None => tracing::warn!(account = %uuid, "usage returned None (no creds / token expired / fetch failed — run verify_all_accounts to reconcile)"),
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

#[tauri::command]
pub fn project_move_dry_run(args: MoveArgsDto) -> Result<DryRunPlanDto, String> {
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
    Ok(DryRunPlanDto::from(&plan))
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
        failed_journal_id: None,
    };
    ops.insert(info);

    let app_for_task = app.clone();
    let ops_for_task = ops.clone();
    let op_id_for_task = op_id.clone();
    let old_path_for_task = entry.journal.old_path.clone();
    tokio::task::spawn_blocking(move || {
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
            OpKind::MoveProject => unreachable!("wrong spawn path"),
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
        failed_journal_id: None,
    };
    ops.insert(info);

    let app_for_task = app.clone();
    let ops_for_task = ops.inner().clone();
    let op_id_for_task = op_id.clone();
    let old_path_for_task = args.old_path.clone();
    tokio::task::spawn_blocking(move || {
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
