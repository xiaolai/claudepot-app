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

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use claudepot_core::services::usage_cache::UsageCache;
use claudepot_core::services::usage_snapshot;
use futures_util::FutureExt;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::rotation_orchestrator::RotationOrchestrator;

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
    // grill X6: each sub-orchestrator call is wrapped in a
    // `catch_unwind` so a single panic anywhere in the pipeline
    // cannot silently kill *every* orchestrator for the GUI's
    // lifetime. The previous shape — `loop { run_tick().await;
    // sleep().await; }` with zero panic isolation — meant a malformed
    // `result.json`, an unmanaged `app.state::<…>()` call, or any
    // subprocess panic stopped permission revert + PR refresh + the
    // event orchestrator + snapshot writer + rotation all at once.
    // Note `panic = "abort"` in the release profile defeats this in
    // production binaries, but the guard still pays off in dev,
    // tests, and any future profile change.

    // Revert any expired permission grants first — this is independent
    // of account state, so it must run before the early returns below
    // (no accounts / no CLI credentials). Returns after one cheap file
    // read when no grants exist.
    guarded("permission_orchestrator", crate::permission_orchestrator::tick(app))
        .await;

    // Refresh per-project PR detection. Independent of accounts (a
    // user can have projects without an Anthropic account); runs
    // before the account-state early returns. The orchestrator
    // dispatches each detect via `spawn_blocking` internally, so
    // this awaits a single task that walks all projects sequentially.
    guarded("pr_orchestrator", pr_tick(app)).await;

    // Fire session-settled event agents — also independent of the
    // active Anthropic account (an event agent can run as any
    // configured route / binary). Returns immediately when no
    // event-triggered agents are installed; otherwise loads the
    // ledger, evaluates, and dispatches `claude -p`. The "record
    // ledger first, dispatch second" invariant lives inside the
    // orchestrator — see `agent_event_orchestrator::tick`'s
    // doc-comment, and the F1/F14/F17 constraints in
    // `claudepot_core::agent::events`'s module-doc.
    let config_dir = claudepot_core::paths::claude_config_dir();
    guarded(
        "agent_event_orchestrator",
        crate::agent_event_orchestrator::tick(app, config_dir),
    )
    .await;

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

    // Scrape `claude daemon status` once per snapshot write so
    // downstream consumers (rotation audit, Activities tile) see a
    // consistent bg-worker count alongside the utilization numbers.
    // Spawned blocking — the scrape is sync (process spawn + line
    // parse, ~50ms in the idle case).
    let bg_workers = tauri::async_runtime::spawn_blocking(|| {
        let s = claudepot_core::cc_daemon::scrape_daemon_status();
        s.bg_workers
    })
    .await
    .unwrap_or(None);

    let snapshot = usage_snapshot::build_with_bg_workers(&accounts, &outcomes, bg_workers);
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
            bg_workers = ?snapshot.bg_workers,
            "usage_snapshot: wrote"
        );
    }

    // Run rotation evaluation against the same snapshot we just
    // produced. The orchestrator loads its rules each tick (cheap;
    // small file), evaluates per-rule, and dispatches swaps via
    // mode (auto / confirm). When no rules exist, the orchestrator
    // returns immediately — zero overhead for users who don't opt in.
    if let Some(active_uuid) = active_cli_uuid(&accounts) {
        let orchestrator = app.state::<Arc<RotationOrchestrator>>();
        let orchestrator: Arc<RotationOrchestrator> = Arc::clone(&orchestrator);
        guarded(
            "rotation_orchestrator",
            orchestrator.tick(app, &snapshot, active_uuid),
        )
        .await;
    }
}

/// Run a sub-orchestrator future inside a `catch_unwind` so a panic
/// in one tick cannot kill every orchestrator for the GUI's lifetime
/// (grill X6). The future is wrapped in `AssertUnwindSafe` because
/// the orchestrators' shared state (Tauri `AppHandle`, store locks,
/// `Arc<…>` handles) is not auto-`UnwindSafe`; a panic here is
/// surfaced via `tracing::error!` and the loop continues.
///
/// ## Release-profile caveat — the guard is a no-op in production
///
/// Audit follow-up A8a: the workspace's `[profile.release]` sets
/// `panic = "abort"`. Under that profile a panic inside `fut`
/// immediately aborts the entire process; `AssertUnwindSafe(fut)
/// .catch_unwind()` never observes a payload because there is no
/// unwinding to catch. The protection this function provides is
/// therefore **limited to debug builds, tests, and any future
/// profile that opts back into `panic = "unwind"`**. Production
/// (the `--release` Tauri bundle the user installs) does NOT enjoy
/// the X6 isolation — a panic in any sub-orchestrator takes the
/// whole GUI down.
///
/// This is a deliberate cost/benefit: switching the release profile
/// to `panic = "unwind"` would grow binary size and incur a small
/// runtime cost on every fallible path. The guard still earns its
/// place by catching panic-shaped bugs in CI before they reach a
/// release; for true production resilience, individual orchestrators
/// must use `Result`-shaped error handling rather than relying on
/// this wrapper.
async fn guarded<F>(name: &'static str, fut: F)
where
    F: std::future::Future<Output = ()>,
{
    if let Err(payload) = AssertUnwindSafe(fut).catch_unwind().await {
        let msg = panic_payload_msg(&payload);
        tracing::error!(
            orchestrator = name,
            panic_msg = %msg,
            "usage_snapshot::run_tick: sub-orchestrator panicked; other \
             orchestrators continue running on the next tick"
        );
    }
}

/// Best-effort extraction of a printable message from a panic
/// payload. The std panic hook accepts `&str` and `String`; anything
/// else surfaces as "<non-string panic payload>".
fn panic_payload_msg(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}

/// Pick the active-CLI account's uuid from the list. Returns `None`
/// when no account is marked active — rotation has nothing to do in
/// that state.
fn active_cli_uuid(accounts: &[claudepot_core::account::Account]) -> Option<Uuid> {
    accounts.iter().find(|a| a.is_cli_active).map(|a| a.uuid)
}

/// Walk every project and refresh its cached PR detection. Skipped
/// silently when project listing fails (treated like permissions
/// errors elsewhere — never fatal to the tick).
async fn pr_tick(app: &AppHandle) {
    use crate::pr_orchestrator::PrOrchestrator;
    use std::path::PathBuf;

    let orch_state = app.state::<Arc<PrOrchestrator>>();
    let orch: Arc<PrOrchestrator> = Arc::clone(&orch_state);

    // Discovering the project list is sync I/O — keep it on the
    // blocking pool. The orchestrator's own `tick_all` is fully
    // async (tokio::process under a Semaphore) so detection runs
    // on the tokio reactor, not the blocking pool.
    let roots = tauri::async_runtime::spawn_blocking(move || {
        let cfg = claudepot_core::paths::claude_config_dir();
        let projects = claudepot_core::project::list_projects(&cfg)
            .map_err(|e| format!("pr_tick: list projects: {e}"))?;
        Ok::<Vec<PathBuf>, String>(
            projects
                .iter()
                .filter(|p| p.is_reachable)
                .map(|p| PathBuf::from(&p.original_path))
                .collect(),
        )
    })
    .await;

    let roots = match roots {
        Ok(Ok(r)) if !r.is_empty() => r,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "pr_tick failed");
            return;
        }
        _ => return,
    };

    orch.tick_all(roots).await;
}

#[cfg(test)]
mod tests {
    //! grill X6: the panic guard is the load-bearing claim, so the
    //! `guarded` helper has its own test. The full `run_tick`
    //! requires a Tauri `AppHandle` and is exercised by the
    //! integration shape; here we verify the *isolation* contract.

    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn guarded_swallows_panic_and_logs() {
        // A panicking future inside `guarded` must not propagate the
        // panic. Two `guarded` calls in sequence: the first panics,
        // the second runs to completion — proving the loop survives.
        guarded("panicker", async {
            panic!("synthetic panic in a sub-orchestrator");
        })
        .await;

        let ran = AtomicBool::new(false);
        // Re-borrow into the async block (closure can't move
        // `&AtomicBool` non-'static — but the future does not
        // outlive `ran`).
        guarded("after-panic", async {
            ran.store(true, Ordering::SeqCst);
        })
        .await;
        assert!(
            ran.load(Ordering::SeqCst),
            "guarded must allow subsequent ticks to run after a panic"
        );
    }

    #[tokio::test]
    async fn guarded_returns_normally_when_future_completes() {
        // Sanity: the happy path is just an `await`.
        let ran = AtomicBool::new(false);
        guarded("happy", async {
            ran.store(true, Ordering::SeqCst);
        })
        .await;
        assert!(ran.load(Ordering::SeqCst));
    }
}
