//! Background poller for `claude doctor`.
//!
//! Spawned once from `setup()`; lives for the app's lifetime. Each
//! cycle re-scrapes (force=true → bypass the command's 60 s cache),
//! pushes the verdict to [`crate::state::TrayHealthState`], and
//! rebuilds the tray menu so the Health row stays current even when
//! the window is closed.
//!
//! Cadence (5 min) is intentionally slow:
//!
//! - Each scrape is 6–10 s of blocking work — the pty must wait for
//!   CC's npm dist-tag fetch in the Updates section. A tighter
//!   cadence would either trample the cache or burn CPU for no real
//!   change in health status.
//! - The renderer's pill polls every 60 s while the window is
//!   visible. When the window is open, this watcher is mostly a
//!   no-op (the command's cache returns the existing snapshot when
//!   it's <60 s old, AND we force-refresh, so a tick still runs a
//!   fresh scrape but produces the same data).
//! - The first tick is delayed [`FIRST_TICK_DELAY`] so the renderer's
//!   own first scrape lands first and the user doesn't see a
//!   double-scrape race at boot.
//!
//! No single-flight gate: a tick that overlaps with the next tick
//! is harmless — the second scrape overwrites the cache and
//! TrayHealthState with the same-or-newer verdict.

use std::time::Duration;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::cc_doctor::{push_to_tray_health, CcDoctorState};
use crate::state::TrayHealthState;

const FIRST_TICK_DELAY: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK_DELAY).await;
        loop {
            tick(&app).await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

async fn tick(app: &AppHandle) {
    let snapshot = match tokio::task::spawn_blocking(claudepot_core::cc_doctor::scrape_doctor).await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cc_doctor_watcher: blocking task join failed: {e}");
            return;
        }
    };

    // Mirror what the IPC command does, in the same order: update
    // the cache so the next IPC call doesn't repeat the work, then
    // mirror to tray state, then ask for a tray rebuild.
    if let Some(cache_state) = app.try_state::<CcDoctorState>() {
        // The command's cache is private (Mutex<Option<Cached>>);
        // exposing it via a setter would be wider surface than the
        // watcher needs. Instead the watcher takes the cheaper
        // path: it doesn't write to the cache directly. The next
        // IPC caller within the TTL will hit the existing cached
        // snapshot (potentially stale by up to one watcher tick),
        // and the call after will re-scrape. Acceptable: the
        // renderer's 60 s poll keeps the cache fresh whenever the
        // window is open, which is the only scenario where IPC
        // staleness matters.
        let _ = cache_state;
    }
    if let Some(tray_state) = app.try_state::<TrayHealthState>() {
        push_to_tray_health(&tray_state, &snapshot);
    } else {
        tracing::warn!("cc_doctor_watcher: TrayHealthState not managed; tick wasted");
        return;
    }
    if let Err(e) = crate::tray::rebuild(app).await {
        tracing::warn!("cc_doctor_watcher: tray rebuild failed: {e}");
    }
}
