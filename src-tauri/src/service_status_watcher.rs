//! Background poller for `status.claude.com`.
//!
//! Spawned once from `setup()` via [`spawn`]. The task lives for the
//! app's lifetime; tokio drops it on runtime shutdown.
//!
//! Each cycle:
//!   1. Read the user's poll setting + interval. If polling is off,
//!      sleep one fallback interval and re-check (lets the user
//!      enable polling without restarting the app).
//!   2. Call `claudepot_core::service_status::fetch_summary()`.
//!   3. If the tier transitioned (OK ↔ Degraded ↔ Down), append a
//!      `Notice`-kind entry to `notification_log`. Optionally fire an
//!      OS notification (off by default — see plan doc).
//!   4. Emit `service-status::updated` so the renderer can refresh.
//!   5. Sleep `poll_interval_minutes`.
//!
//! Latency probing is **not** done here — see
//! `dev-docs/network-status.md`. The renderer triggers probes
//! on-demand via `service_status_probe_now`.

use std::time::Duration;

use claudepot_core::notification_log::{NotificationKind, NotificationSource};
use claudepot_core::service_status as core;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands_notification::NotificationLogState;
use crate::commands_service_status::ServiceStatusState;
use crate::preferences::PreferencesState;

/// Initial delay before the first tick. Long enough for the webview
/// to mount + listen on `service-status::updated`; short enough that
/// fresh data lands within seconds of opening the app.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(8);

/// Used when polling is off OR the preferences mutex is poisoned.
/// Keeps the loop alive without hammering the network.
const FALLBACK_INTERVAL: Duration = Duration::from_secs(60 * 5);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK_DELAY).await;

        loop {
            let interval = tick(&app).await;
            tokio::time::sleep(interval).await;
        }
    });
}

/// Run one cycle. Returns the duration to sleep before the next cycle.
async fn tick(app: &AppHandle) -> Duration {
    let settings = read_settings(app);
    if !settings.poll_enabled {
        return FALLBACK_INTERVAL;
    }

    let state = match app.try_state::<ServiceStatusState>() {
        Some(s) => s,
        None => {
            tracing::warn!("service_status_watcher: state not managed; aborting cycle");
            return FALLBACK_INTERVAL;
        }
    };

    match core::fetch_summary().await {
        Ok(summary) => {
            let new_tier = core::summary_tier(&summary);
            let prev_tier = state.store_summary(summary.clone());

            // Skip notification on the first successful poll (prev =
            // Unknown) — we don't want a "Claude services back to
            // normal" toast every cold start.
            if prev_tier != new_tier && prev_tier != core::StatusTier::Unknown {
                if let Err(e) =
                    record_transition(app, prev_tier, new_tier, &summary, &settings)
                {
                    tracing::warn!(error = %e, "service_status_watcher: record_transition failed");
                }
            }

            if let Err(e) = app.emit("service-status::updated", ()) {
                tracing::warn!(error = %e, "service_status_watcher: emit failed");
            }
        }
        Err(e) => {
            // Don't log at warn for routine fetch failures — the user
            // closes the laptop, the wifi blips, etc. Surface the
            // error string in the cached state so the renderer can
            // explain "last poll failed" without us paging an operator.
            tracing::debug!(error = %e, "service_status_watcher: fetch failed");
            state.store_fetch_error(e.to_string());
        }
    }

    Duration::from_secs(u64::from(settings.poll_interval_minutes) * 60)
}

#[derive(Debug, Clone, Copy)]
struct WatcherSettings {
    poll_enabled: bool,
    poll_interval_minutes: u32,
    os_notify_on_change: bool,
}

fn read_settings(app: &AppHandle) -> WatcherSettings {
    let prefs = match app.try_state::<PreferencesState>() {
        Some(p) => p,
        None => {
            return WatcherSettings {
                poll_enabled: false,
                poll_interval_minutes: 5,
                os_notify_on_change: false,
            }
        }
    };
    let g = match prefs.0.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    WatcherSettings {
        poll_enabled: g.service_status.poll_status_page,
        poll_interval_minutes: g.service_status.poll_interval_minutes.clamp(2, 60),
        os_notify_on_change: g.service_status.os_notify_on_status_change,
    }
}

fn record_transition(
    app: &AppHandle,
    prev: core::StatusTier,
    new: core::StatusTier,
    summary: &core::StatusSummary,
    settings: &WatcherSettings,
) -> Result<(), String> {
    let (title, body) = transition_message(prev, new, summary);

    // Append to the in-app notification log unconditionally — the bell
    // popover is the persistent record. OS banner is gated.
    if let Some(log) = app.try_state::<NotificationLogState>() {
        let _ = log.log.append(
            NotificationSource::Toast,
            NotificationKind::Notice,
            title.clone(),
            body.clone(),
            // No click target — clicking the entry doesn't navigate.
            // Wiring this to "Open Settings → Network" is a future polish.
            serde_json::Value::Null,
        );
    }

    if settings.os_notify_on_change {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            tracing::warn!(error = %e, "service_status_watcher: OS notification failed");
        }
    }

    Ok(())
}

fn transition_message(
    prev: core::StatusTier,
    new: core::StatusTier,
    summary: &core::StatusSummary,
) -> (String, String) {
    let title = match new {
        core::StatusTier::Ok => "Claude services back to normal".to_string(),
        core::StatusTier::Degraded => "Claude services degraded".to_string(),
        core::StatusTier::Down => "Claude services down".to_string(),
        core::StatusTier::Unknown => "Claude service status unknown".to_string(),
    };

    let body = if matches!(new, core::StatusTier::Ok) {
        format!(
            "Recovered from {}. Status: {}.",
            tier_human(prev),
            summary.status.description,
        )
    } else {
        let affected: Vec<&str> = summary
            .components
            .iter()
            .filter(|c| {
                !matches!(
                    core::StatusTier::from_component_status(&c.status),
                    core::StatusTier::Ok
                )
            })
            .map(|c| c.name.as_str())
            .take(3)
            .collect();
        if affected.is_empty() {
            summary.status.description.clone()
        } else {
            format!(
                "Affected: {}. {}",
                affected.join(", "),
                summary.status.description
            )
        }
    };

    (title, body)
}

fn tier_human(t: core::StatusTier) -> &'static str {
    match t {
        core::StatusTier::Ok => "operational",
        core::StatusTier::Degraded => "degraded",
        core::StatusTier::Down => "down",
        core::StatusTier::Unknown => "unknown",
    }
}
