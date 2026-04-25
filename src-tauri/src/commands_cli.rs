//! Tauri commands for the CC (CLI) surface: switch, running-check,
//! startup sync, ground-truth identity probe.

use crate::commands::{active_id, open_store, resolve_target};
use crate::dto::CcIdentity;
use claudepot_core::account::AccountStore;
use claudepot_core::cli_backend;
use claudepot_core::services;

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
    // Return the bare SwapError Display — the frontend (toast or
    // tray-switch handler) is responsible for the surface-appropriate
    // prefix. The previous `"cli switch failed: {e}"` wrapper produced
    // a doubly-prefixed toast ("Tray switch failed: cli switch failed:
    // …") and leaked CLI-binary phrasing into a GUI surface.
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
    .map_err(|e| e.to_string())
}

/// Idempotent startup sync: if CC is currently signed in as one of the
/// registered Claudepot accounts, make sure Claudepot's stored blob
/// and active_cli match. Lets users who ran `claude auth login`
/// externally see healthy credentials the moment the GUI opens,
/// without clicking anything.
///
/// Returns the email that was synced, empty string if nothing matched,
/// or a prefixed error message for user-facing conditions the UI should
/// surface prominently:
///   * `keychain is locked` — macOS login keychain needs unlocking.
///   * `auth rejected: …` — CC's stored token is terminally dead
///     (refresh failed). UI routes to a "Sign in again" banner.
#[tauri::command]
pub async fn sync_from_current_cc() -> Result<String, String> {
    use claudepot_core::services::account_service::RegisterError;
    let store = open_store()?;
    match services::account_service::sync_from_current_cc(&store).await {
        Ok(Some(uuid)) => Ok(store
            .find_by_uuid(uuid)
            .ok()
            .flatten()
            .map(|a| a.email)
            .unwrap_or_default()),
        Ok(None) => Ok(String::new()),
        Err(RegisterError::AuthRejected) => {
            // Keep the `auth rejected:` prefix stable — the frontend
            // pattern-matches on it to distinguish "log in again" from
            // the generic sync-warning banner.
            Err(format!("auth rejected: {}", RegisterError::AuthRejected))
        }
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
