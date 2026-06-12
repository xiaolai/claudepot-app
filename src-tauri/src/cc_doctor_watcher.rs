//! Background poller for `claude doctor`.
//!
//! Spawned once from `setup()`; lives for the app's lifetime. Each
//! cycle runs a fresh `scrape_with_probes`, pushes the verdict to
//! [`crate::state::TrayHealthState`], and rebuilds the tray menu so
//! the Health row stays current even when the window is closed.
//!
//! **The watcher feeds the tray only.** The IPC command's 60 s
//! snapshot cache (`commands::cc_doctor::CcDoctorState`) is
//! renderer-owned: only `cc_doctor_snapshot` writes it, and the
//! renderer's 60 s poll keeps it fresh whenever the window is open
//! — the only scenario where IPC staleness matters. The watcher's
//! scrape deliberately does not touch it; exposing a cache setter
//! would widen the command's surface to save at most one scrape per
//! open-window tick.
//!
//! Cadence (5 min) is intentionally slow:
//!
//! - Each scrape is 6–10 s of blocking work — the pty must wait for
//!   CC's npm dist-tag fetch in the Updates section. A tighter
//!   cadence would burn CPU for no real change in health status.
//! - The first tick is delayed [`FIRST_TICK_DELAY`] so the renderer's
//!   own first scrape lands first and the user doesn't see a
//!   double-scrape race at boot.
//!
//! No single-flight gate: a tick that overlaps with the next tick
//! is harmless — the second scrape overwrites TrayHealthState with
//! the same-or-newer verdict.

use std::time::Duration;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::cc_doctor::push_to_tray_health;
use crate::state::TrayHealthState;

const FIRST_TICK_DELAY: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub fn spawn(app: AppHandle) {
    crate::poller::spawn_poller(
        app,
        "cc_doctor_watcher",
        FIRST_TICK_DELAY,
        |app| async move {
            tick(&app).await;
            POLL_INTERVAL
        },
    );
}

async fn tick(app: &AppHandle) {
    // `scrape_with_probes` over bare `scrape_doctor`: the watcher's
    // verdict feeds the tray menu copy, which is closed-window
    // users' only health signal. If the TUI parser breaks (and the
    // renderer's pane isn't open to surface the failure), the
    // probes still give us cc_version + install identity so the
    // tray label reads "Health: ok" instead of "Health: 1 issue"
    // (the old aggregate_severity's forced-Warning behavior).
    let snapshot =
        match tokio::task::spawn_blocking(claudepot_core::cc_doctor::scrape_with_probes).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("cc_doctor_watcher: blocking task join failed: {e}");
                return;
            }
        };

    // Mirror to tray state, then ask for a tray rebuild. The IPC
    // command's snapshot cache is deliberately NOT written here —
    // see the module doc ("the watcher feeds the tray only").
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
