//! Periodic writer for `~/.claudepot/usage-snapshot.json`.
//!
//! Every `POLL_INTERVAL` it lists all accounts, calls
//! `UsageCache::fetch_batch_detailed_verified` for the ones with
//! credentials, and atomically writes the resulting snapshot via
//! `claudepot_core::services::usage_snapshot`.
//!
//! Why a separate task from `usage_watcher`: that watcher polls only
//! the *active CLI* account because its purpose is per-account
//! threshold alerts. The snapshot needs *all* accounts so headless
//! consumers (cron, CC bash subprocess) can pick the least-loaded
//! one. Sharing the call would couple two unrelated concerns.
//!
//! Cadence: 5 min, same as `usage_watcher`. Both tasks share the
//! `UsageCache`, so on each tick the active CLI account is served
//! cache-warm to whichever ran second; the snapshot writer pays one
//! live fetch per *other* account. Anthropic's `/usage` endpoint
//! costs no tokens, so the call budget is well within reason for
//! typical 1–5 account households.

use std::time::Duration;

use claudepot_core::services::usage_cache::UsageCache;
use claudepot_core::services::usage_snapshot;
use tauri::{AppHandle, Manager};

/// Match `usage_watcher`'s 5-minute cadence. Different cadence here
/// would only desynchronize the two writers' snapshot/threshold
/// observations of the same shared cache; keep them aligned.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Stagger the first tick by 10s so we don't compete with setup-time
/// I/O for the OS file cache. The previous run's snapshot (if any)
/// remains readable until the first tick overwrites it; a brief
/// startup gap is invisible to consumers per the
/// `usage_snapshot::UsageSnapshot` "older than 5 min = GUI not
/// running" contract.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(10);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK_DELAY).await;
        loop {
            run_tick(&app).await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

async fn run_tick(app: &AppHandle) {
    // Open store + list accounts under one blocking scope so the
    // SQLite open and the .list() call aren't ping-ponging into
    // the async runtime.
    let setup = tauri::async_runtime::spawn_blocking(|| -> Result<_, String> {
        let store = crate::commands::open_store()?;
        let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
        Ok((store, accounts))
    })
    .await;

    let (store, accounts) = match setup {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "usage_snapshot: setup failed");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "usage_snapshot: spawn_blocking join failed");
            return;
        }
    };

    if accounts.is_empty() {
        // No registered accounts → leave any prior snapshot alone.
        // An empty file would mislead consumers into "the GUI is
        // running but has no accounts," which is a different state
        // from "you haven't installed Claudepot."
        return;
    }

    let uuids: Vec<uuid::Uuid> = accounts
        .iter()
        .filter(|a| a.has_cli_credentials)
        .map(|a| a.uuid)
        .collect();
    if uuids.is_empty() {
        return;
    }

    let outcomes = {
        let cache_state = app.state::<UsageCache>();
        let cache: &UsageCache = &cache_state;
        cache.fetch_batch_detailed_verified(&store, &uuids).await
    };

    let snapshot = usage_snapshot::build(&accounts, &outcomes);
    let path = usage_snapshot::snapshot_path();
    if let Err(e) = usage_snapshot::write(&path, &snapshot) {
        tracing::warn!(
            error = %e,
            path = %path.display(),
            "usage_snapshot: write failed"
        );
    } else {
        tracing::debug!(
            path = %path.display(),
            accounts = snapshot.accounts.len(),
            "usage_snapshot: wrote"
        );
    }
}
