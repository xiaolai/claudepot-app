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

use crate::dto::{AccountSummary, AccountSummaryBasic, AppStatus, ReconcileReportDto};
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
///
/// The Keychain reads themselves are routed through
/// `tauri::async_runtime::spawn_blocking` so they run on Tokio's
/// blocking pool instead of an async worker — same threading guard
/// as the `commands_config` / `commands_keys` handlers. Sequential
/// order is preserved inside `list_summaries` so macOS unlock
/// dialogs don't stack.
///
/// **Pure read.** Any DB ↔ truth-on-disk drift is reconciled by
/// [`accounts_reconcile`] (called once at startup in `lib.rs::run`
/// and on user request). `account_list` never writes to the store.
#[tauri::command]
pub async fn account_list() -> Result<Vec<AccountSummary>, String> {
    let views = tauri::async_runtime::spawn_blocking(move || {
        let store = open_store()?;
        let views = services::account_summary::list_summaries(&store)
            .map_err(|e| format!("list failed: {e}"))?;
        Ok::<_, String>(views)
    })
    .await
    .map_err(|e| format!("account_list join: {e}"))??;

    Ok(views.iter().map(AccountSummary::from).collect())
}

/// Run both reconcile passes (CLI flag drift + Desktop flag drift +
/// orphan `state.active_desktop` clear) and return counts to the
/// webview. The full per-row detail stays in the Rust
/// [`claudepot_core::services::account_service::ReconcileReport`];
/// the JS side renders a one-line summary and refetches
/// `account_list` to pick up the post-reconcile DB state.
///
/// Routed through `spawn_blocking` for the same reason as
/// [`account_list`] — the inner `token_health` calls do macOS
/// Keychain syscalls and must stay off async workers.
#[tauri::command]
pub async fn accounts_reconcile() -> Result<ReconcileReportDto, String> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        let store = open_store()?;
        services::account_service::reconcile_all(&store)
            .map_err(|e| format!("reconcile failed: {e}"))
    })
    .await
    .map_err(|e| format!("accounts_reconcile join: {e}"))??;

    Ok(ReconcileReportDto::from(&report))
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

/// Whether the current install can be updated in-place by
/// `tauri-plugin-updater`.
///
/// Returns `false` immediately if the embedded updater public key is
/// empty (placeholder state — keypair hasn't been generated yet, or
/// the `tauri.conf.json` was checked in without one). Without a
/// pubkey, signature verification has no anchor; surfacing the UI
/// would just produce confusing "verification failed" errors when
/// the user clicks Download. The release workflow's preflight job
/// also fails closed on this state, so this is defense in depth.
///
/// **macOS** — always supported. Tauri ships `.app.tar.gz` updater
/// bundles for both architectures.
///
/// **Linux** — supported only when the binary is running from an
/// AppImage. The AppImage runtime sets the `APPIMAGE` env var to the
/// absolute path of the `.AppImage` file; that's the canonical
/// detection signal recommended by the AppImageKit project. Without
/// it (system install, `cargo run`, `.deb`), the in-app updater
/// would download an `.AppImage.tar.gz` and try to extract it over
/// files apt manages — which would either fail, race with apt, or
/// corrupt the package state. We hide the UI instead.
///
/// **Windows** — supported only for NSIS installs. Tauri 2's
/// `latest.json` keys platforms by `{os}-{arch}` only, with no
/// per-bundle-format distinction; the canonical `windows-x86_64`
/// entry points at `.nsis.zip`. An MSI install that hits the
/// updater would download NSIS and replace files under the MSI's
/// registration, leaving Windows Add/Remove Programs out of sync.
/// We disambiguate at runtime by checking the binary's install
/// directory: NSIS defaults to a per-user path under
/// `%LOCALAPPDATA%\Programs\…`, while MSI installs land under
/// `Program Files`. Misclassifying a side-loaded build is harmless
/// (the updater check runs against the manifest and returns "no
/// update available" for the dev version anyway).
#[tauri::command]
pub async fn updater_supported(app: tauri::AppHandle) -> bool {
    if !updater_pubkey_configured(&app) {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("APPIMAGE").is_some()
    }
    #[cfg(target_os = "windows")]
    {
        windows_is_nsis_install()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

/// Read `plugins.updater.pubkey` from the live Tauri config and
/// return true iff it's non-empty. The config is parsed once at
/// app start, so this is just an Arc deref — cheap to call from a
/// hot command path.
fn updater_pubkey_configured(app: &tauri::AppHandle) -> bool {
    let config = app.config();
    let Some(updater) = config.plugins.0.get("updater") else {
        return false;
    };
    updater
        .get("pubkey")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn windows_is_nsis_install() -> bool {
    // True when the running binary lives in a directory that looks
    // like a Tauri NSIS install (per-user). Returns true for any
    // path containing the `\Programs\` segment under a user profile,
    // which is where Tauri's NSIS template installs by default.
    // Returns false for `Program Files` (MSI / system install) and
    // for `target\debug` / `target\release` (dev runs — keeping the
    // updater UI out of dev where there's no signed bundle anyway).
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let path = exe.to_string_lossy().to_lowercase();
    // Dev-build paths — not an installed location.
    if path.contains(r"\target\debug\") || path.contains(r"\target\release\") {
        return false;
    }
    // MSI / system install. Tauri's MSI bundler defaults to
    // `%ProgramFiles%\<publisher>\<product>\…` for per-machine
    // installs, which is the only path users typically reach via
    // `msiexec`.
    if path.contains(r"\program files\") || path.contains(r"\program files (x86)\") {
        return false;
    }
    // NSIS default: `%LOCALAPPDATA%\Programs\<product>\…`. Match the
    // signature segment rather than the full LOCALAPPDATA prefix so
    // we tolerate roaming profiles and redirected folders.
    path.contains(r"\programs\")
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

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::cli_backend::swap;
    use std::sync::Mutex;

    /// Tests in this module mutate process-global env vars
    /// (`CLAUDEPOT_DATA_DIR`, `CLAUDEPOT_CREDENTIAL_BACKEND`). Cargo
    /// runs unit tests within a single binary in parallel by default,
    /// so we serialize through this lock to keep the env from racing.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn setup_data_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());
        std::env::set_var("CLAUDEPOT_CREDENTIAL_BACKEND", "file");
        dir
    }

    fn make_account(email: &str) -> claudepot_core::account::Account {
        claudepot_core::account::Account {
            uuid: uuid::Uuid::new_v4(),
            email: email.to_string(),
            org_uuid: Some("org-test".to_string()),
            org_name: Some("Test Org".to_string()),
            subscription_type: Some("pro".to_string()),
            rate_limit_tier: None,
            created_at: chrono::Utc::now(),
            last_cli_switch: None,
            last_desktop_switch: None,
            has_cli_credentials: true,
            has_desktop_profile: false,
            is_cli_active: false,
            is_desktop_active: false,
            verified_email: None,
            verified_at: None,
            verify_status: "never".to_string(),
        }
    }

    /// `account_list` must be a pure read after B-2: even when the DB
    /// flag for `has_cli_credentials` disagrees with keychain truth,
    /// the list call leaves the row untouched. Reconciliation is the
    /// dedicated `accounts_reconcile` command's job.
    #[tokio::test]
    async fn test_account_list_is_pure_read() {
        let _g = lock();
        let _env = setup_data_dir();

        // Open the production-style store at `<data_dir>/accounts.db`,
        // then close it before invoking `account_list` — the command
        // re-opens its own connection.
        let db_path = paths::claudepot_data_dir().join("accounts.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = AccountStore::open(&db_path).unwrap();

        // Drift case 1: DB says has_cli_credentials=true, keychain empty.
        let mut a = make_account("drift@example.com");
        a.has_cli_credentials = true;
        store.insert(&a).unwrap();
        let before = store.find_by_uuid(a.uuid).unwrap().unwrap();
        assert!(before.has_cli_credentials, "precondition: flag was true");
        // Drop the store handle so the command's open succeeds.
        drop(store);

        // The list call. Resolve the inner future via the macro shim:
        // `#[tauri::command]` keeps the function callable as a normal
        // async fn from Rust.
        let _ = account_list().await.unwrap();

        // Re-open and verify the flag is *unchanged*. (The pre-B-2
        // implementation would have flipped this to false.)
        let after_store = AccountStore::open(&db_path).unwrap();
        let after = after_store.find_by_uuid(a.uuid).unwrap().unwrap();
        assert!(
            after.has_cli_credentials,
            "account_list must not write to the store; flag flipped from {} to {}",
            before.has_cli_credentials, after.has_cli_credentials
        );

        // Sanity: `accounts_reconcile` *does* flip the same drift.
        let report = accounts_reconcile().await.unwrap();
        assert_eq!(report.cli_flipped, 1);
        let after_reconcile = after_store.find_by_uuid(a.uuid).unwrap().unwrap();
        assert!(!after_reconcile.has_cli_credentials);

        // Cleanup keychain side: nothing was saved here, but if a
        // previous run left a blob on this uuid, drop it.
        let _ = swap::delete_private(a.uuid);
    }
}
