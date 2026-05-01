//! Background poller for CC CLI + Claude Desktop updates.
//!
//! Spawned once from `setup()` via [`spawn`]. The task lives for the
//! app's lifetime; tokio drops it on runtime shutdown.
//!
//! Each cycle:
//!   1. Single-flight gate: if a prior cycle is still in flight (a
//!      long auto-install spanning a tick boundary), this tick skips
//!      and waits for the next one.
//!   2. Calls `claudepot_core::updates::poller::run_one_check_cycle`.
//!      That function probes upstream, persists the cache, and runs
//!      the auto-install pass when toggles allow.
//!   3. Updates the tray badge via `UpdatesAlertState` + a
//!      `tray::refresh_alert_chrome` call. Counter scheme: 1 per
//!      surface that has an available update AND `notify_on_available`.
//!   4. Emits `updates::cycle-complete` to the webview so the
//!      Updates panel can refresh without polling.
//!   5. Sleeps for the configured `poll_interval_minutes`
//!      (default 240 = 4 h, clamped to [30, 1440]).
//!
//! All logic for "does an auto-install fire" lives in core; this
//! module is pure orchestration (timing + side effects).

use claudepot_core::updates::poller::{
    run_one_check_cycle, save_state, CheckCycleOutcome, PollerGate,
};
use claudepot_core::updates::UpdateStateMutex;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::UpdatesAlertState;

/// Initial delay before the first tick. Long enough for the webview
/// to finish mounting + listening on `updates::cycle-complete` so a
/// quick "update available" emit isn't lost. Short enough that the
/// user gets fresh data within seconds of opening the app.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(15);

/// Hard cap on how long we'll sleep between ticks if reading the
/// settings cadence fails (e.g., mutex poisoned, state un-managed).
/// Defaults to 1 h so we recover within a reasonable window.
const FALLBACK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Frontend event payload emitted after each cycle. The Updates
/// panel listens on `updates::cycle-complete` and refreshes its
/// status from `updates_status_get` without doing its own polling.
#[derive(Debug, Clone, Serialize)]
pub struct UpdatesCycleEvent {
    pub cli_latest: Option<String>,
    pub desktop_latest: Option<String>,
    pub cli_update_available: bool,
    pub desktop_update_available: bool,
    pub cli_auto: claudepot_core::updates::AutoInstallOutcome,
    pub desktop_auto: claudepot_core::updates::AutoInstallOutcome,
}

impl From<&CheckCycleOutcome> for UpdatesCycleEvent {
    fn from(o: &CheckCycleOutcome) -> Self {
        Self {
            cli_latest: o.cli_latest.clone(),
            desktop_latest: o.desktop_latest.clone(),
            cli_update_available: o.cli_update_available,
            desktop_update_available: o.desktop_update_available,
            cli_auto: o.cli_auto.clone(),
            desktop_auto: o.desktop_auto.clone(),
        }
    }
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK_DELAY).await;

        loop {
            tick(&app).await;
            let interval = read_interval(&app);
            tokio::time::sleep(interval).await;
        }
    });
}

async fn tick(app: &AppHandle) {
    // Pull the shared gate from app state so the watcher serializes
    // against manual `updates_check_now` / `updates_cli_install` /
    // `updates_desktop_install` IPC calls. See lib.rs `.manage(Arc<PollerGate>)`.
    let gate: Arc<PollerGate> = match app.try_state::<Arc<PollerGate>>() {
        Some(g) => (*g).clone(),
        None => {
            tracing::warn!("updates_watcher: PollerGate not managed; skipping tick");
            return;
        }
    };
    let lease = match gate.try_acquire() {
        Some(l) => l,
        None => {
            tracing::info!(
                "updates_watcher: previous cycle still in flight; skipping tick"
            );
            return;
        }
    };

    let state = match app.try_state::<UpdateStateMutex>() {
        Some(s) => s,
        None => {
            tracing::warn!("updates_watcher: UpdateStateMutex not managed; skipping tick");
            drop(lease);
            return;
        }
    };
    let _ = &gate; // suppress unused-import warning when no other ref

    let outcome = run_one_check_cycle(&state).await;

    // Read settings + decide which OS notifications to fire under a
    // single lock. The notify decision needs a comparison against the
    // cached `last_notified_version`; making it inside the lock keeps
    // dedupe atomic across overlapping ticks (the gate prevents
    // overlap, but better cheap-and-correct than fast-and-racy).
    let signals = compute_signals(app, &outcome);

    // Consolidated save AFTER both mutations land (cycle cache write
    // + signal-dedupe write). Saving once per tick — instead of two
    // detached spawn_blocking calls — eliminates the "stale snapshot
    // wins last-write race" failure mode where a faster save of the
    // older snapshot overwrote the dedupe cursor written by the
    // slower second save.
    save_state(&state).await;
    drop(lease); // release after I/O so a long save still single-flights

    // Tray badge — only surfaces with notify_on_available + delta count.
    if let Some(alert) = app.try_state::<UpdatesAlertState>() {
        alert.set(signals.badge_count);
    }
    crate::tray::refresh_alert_chrome(app);

    // OS notifications — fire only when the user opted in AND we
    // haven't already toasted for this exact version. Best-effort:
    // a failed `show()` (Catalina permission revoked, missing
    // notification daemon on Linux, etc.) is logged at warn level
    // and the cycle continues.
    if let Some(n) = signals.cli_notification.as_ref() {
        if let Err(e) = app
            .notification()
            .builder()
            .title(&n.title)
            .body(&n.body)
            .show()
        {
            tracing::warn!(error = %e, "updates_watcher: CLI notification show failed");
        }
    }
    if let Some(n) = signals.desktop_notification.as_ref() {
        if let Err(e) = app
            .notification()
            .builder()
            .title(&n.title)
            .body(&n.body)
            .show()
        {
            tracing::warn!(error = %e, "updates_watcher: Desktop notification show failed");
        }
    }

    // Emit so the panel refreshes without polling.
    if let Err(e) = app.emit(
        "updates::cycle-complete",
        UpdatesCycleEvent::from(&outcome),
    ) {
        tracing::warn!(error = %e, "updates_watcher: emit failed");
    }
}

#[derive(Debug)]
struct CycleSignals {
    badge_count: u32,
    cli_notification: Option<NotificationPayload>,
    desktop_notification: Option<NotificationPayload>,
}

#[derive(Debug)]
struct NotificationPayload {
    title: String,
    body: String,
}

/// Compute badge count + which OS notifications to fire. Mutates
/// the cache's `last_notified_version` fields in-place when a
/// notification will fire, so the next cycle dedupes correctly.
/// Persists the cache on a blocking task — fire-and-forget.
fn compute_signals(app: &AppHandle, outcome: &CheckCycleOutcome) -> CycleSignals {
    let state = match app.try_state::<UpdateStateMutex>() {
        Some(s) => s,
        None => {
            return CycleSignals {
                badge_count: 0,
                cli_notification: None,
                desktop_notification: None,
            }
        }
    };

    // Mutate the dedupe cursor under a single lock so the
    // notification decision and the cursor-write are atomic. The
    // caller (tick) does the save after this returns so all cycle
    // mutations land in one file write.
    let cli_notify;
    let desktop_notify;
    let cli_os_notify;
    let desktop_os_notify;
    let cli_already_notified;
    let desktop_already_notified;
    {
        let mut g = match state.0.lock() {
            Ok(x) => x,
            Err(p) => p.into_inner(),
        };
        cli_notify = g.settings.cli.notify_on_available;
        desktop_notify = g.settings.desktop.notify_on_available;
        cli_os_notify = g.settings.cli.notify_os_on_available;
        desktop_os_notify = g.settings.desktop.notify_os_on_available;
        cli_already_notified =
            g.cache.cli.last_notified_version.as_deref() == outcome.cli_latest.as_deref();
        desktop_already_notified = g.cache.desktop.last_notified_version.as_deref()
            == outcome.desktop_latest.as_deref();

        if outcome.cli_update_available && cli_os_notify && !cli_already_notified {
            g.cache.cli.last_notified_version = outcome.cli_latest.clone();
        }
        if outcome.desktop_update_available && desktop_os_notify && !desktop_already_notified {
            g.cache.desktop.last_notified_version = outcome.desktop_latest.clone();
        }
    }

    let mut badge_count: u32 = 0;
    if outcome.cli_update_available && cli_notify {
        badge_count += 1;
    }
    if outcome.desktop_update_available && desktop_notify {
        badge_count += 1;
    }

    let cli_notification = if outcome.cli_update_available
        && cli_os_notify
        && !cli_already_notified
    {
        Some(NotificationPayload {
            title: "Claude Code update available".into(),
            body: match outcome.cli_latest.as_deref() {
                Some(v) => format!("Version {v} is available. Open Claudepot to install."),
                None => "A new version is available.".into(),
            },
        })
    } else {
        None
    };
    let desktop_notification = if outcome.desktop_update_available
        && desktop_os_notify
        && !desktop_already_notified
    {
        Some(NotificationPayload {
            title: "Claude Desktop update available".into(),
            body: match outcome.desktop_latest.as_deref() {
                Some(v) => format!("Version {v} is available. Quit Desktop to auto-install."),
                None => "A new version is available.".into(),
            },
        })
    } else {
        None
    };

    CycleSignals {
        badge_count,
        cli_notification,
        desktop_notification,
    }
}

fn read_interval(app: &AppHandle) -> Duration {
    let minutes = match app.try_state::<UpdateStateMutex>() {
        Some(s) => match s.0.lock() {
            Ok(g) => g.poll_interval_minutes(),
            Err(p) => p.into_inner().poll_interval_minutes(),
        },
        None => return FALLBACK_INTERVAL,
    };
    Duration::from_secs(u64::from(minutes) * 60)
}
