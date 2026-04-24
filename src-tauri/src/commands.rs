//! Tauri command layer — thin async wrappers over `claudepot-core`.
//!
//! Per `.claude/rules/architecture.md`, NO business logic lives here. Each
//! command opens the store, calls a core function, and serializes the result.
//! Errors become user-facing strings at this boundary.
//!
//! # Threading policy
//!
//! **Every `#[tauri::command]` handler in this file is declared `async fn`.**
//! Tauri 2 dispatches sync (`pub fn`) handlers on the main thread — the same
//! thread that drives the OS event loop and serves the webview. Any blocking
//! I/O in a sync handler (SQLite open/read/write, macOS Keychain lookup,
//! filesystem stat, JSONL scan, HTTP call) therefore freezes the entire
//! window for the duration of the call.
//!
//! Declaring the handler `async fn` tells Tauri to dispatch it on a Tokio
//! worker. The body's sync I/O then blocks that worker, not the UI thread,
//! and the webview keeps painting. Bodies stay otherwise unchanged — no
//! `.await` is required just to reap the threading benefit.
//!
//! Precedents / history: commit `4ad707e` (sessions async fix), followed by
//! the Keys + `account_list` conversion (commit after Keys freeze report).
//! Apply the same discipline to every new handler added here.

use crate::dto::{AccountSummary, AccountSummaryBasic, AppStatus};
use claudepot_core::account::{Account, AccountStore};
use claudepot_core::desktop_backend;
use claudepot_core::paths;
use claudepot_core::services;
use uuid::Uuid;

pub(crate) fn resolve_target(store: &AccountStore, email: &str) -> Result<Account, String> {
    let target_email = claudepot_core::resolve::resolve_email(store, email)
        .map_err(|e| format!("resolve failed: {e}"))?;
    store
        .find_by_email(&target_email)
        .map_err(|e| format!("lookup failed: {e}"))?
        .ok_or_else(|| format!("account not found: {target_email}"))
}

pub(crate) fn active_id<E>(
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

/// `async fn` is load-bearing: Tauri 2 dispatches sync `#[command] fn`
/// handlers on the main thread (the same thread that serves the
/// webview and runs the OS event loop). This handler does N
/// synchronous Keychain lookups per account via `token_health` →
/// `swap::load_private`, so a sync dispatch would freeze the window
/// for the sum of those round-trips. `async fn` moves the body to a
/// Tokio worker; the sync I/O blocks that worker instead of the UI
/// thread. Same rationale / pattern as the `session_*` commands
/// (commit 4ad707e).
#[tauri::command]
pub async fn account_list() -> Result<Vec<AccountSummary>, String> {
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

/// Lean sibling of [`account_list`] — returns just the sqlite-backed
/// fields. Does NOT touch the macOS Keychain (no `token_health`
/// calls) and does NOT run `reconcile_flags`, so it resolves in
/// single-digit milliseconds regardless of account count.
///
/// Callers that only need identity resolution (uuid ↔ email) should
/// use this. KeysSection — which labels each API key / OAuth token
/// with its owner account — is the primary consumer; the Accounts
/// tab / status bar / tray continue to use the full `account_list`
/// since they actually render token health.
#[tauri::command]
pub async fn account_list_basic() -> Result<Vec<AccountSummaryBasic>, String> {
    let store = open_store()?;
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    Ok(accounts.iter().map(AccountSummaryBasic::from).collect())
}

#[tauri::command]
pub async fn app_status() -> Result<AppStatus, String> {
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

// Every topic-specific command surface lives in its own sibling
// `commands_<topic>.rs`; see:
//   - Projects read surface + clean preview/start: `commands_project.rs`
//   - Project move + repair journal ops:          `commands_repair.rs`
//   - Session move / orphan adopt / discard:      `commands_session_move.rs`
//   - Sessions tab + session debugger:            `commands_session_index.rs`
//   - Session prune / slim / trash:               `commands_session_prune.rs`
//   - Session export / share / github token:      `commands_session_share.rs`
//   - Live activity + trends:                     `commands_activity.rs`
//   - Keys (API key + OAuth token):               `commands_keys.rs`
//   - Protected paths:                            `commands_protected.rs`
//   - Preferences:                                `commands_preferences.rs`
//   - Config tree / watcher:                      `commands_config.rs`
//   - Pricing:                                    `commands_pricing.rs`
