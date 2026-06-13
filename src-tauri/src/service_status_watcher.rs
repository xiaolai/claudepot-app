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

use claudepot_core::notification_log::NotificationKind;
use claudepot_core::notifications::{Category, Priority, Surface};
use claudepot_core::service_status as core;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::notification::NotificationLogState;
use crate::commands::service_status::ServiceStatusState;
use crate::preferences::PreferencesState;

/// Initial delay before the first tick. Long enough for the webview
/// to mount + listen on `service-status::updated`; short enough that
/// fresh data lands within seconds of opening the app.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(8);

/// Used when polling is off OR the preferences mutex is poisoned.
/// Keeps the loop alive without hammering the network.
const FALLBACK_INTERVAL: Duration = Duration::from_secs(60 * 5);

pub fn spawn(app: AppHandle) {
    crate::poller::spawn_poller(
        app,
        "service_status_watcher",
        FIRST_TICK_DELAY,
        |app| async move { tick(&app).await },
    );
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
                if let Err(e) = record_transition(app, prev_tier, new_tier, &summary, &settings) {
                    tracing::warn!(error = %e, "service_status_watcher: record_transition failed");
                }
            }

            if let Err(e) = app.emit(crate::events::SERVICE_STATUS_UPDATED, ()) {
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
            // Emit on failure too so the renderer can re-read and
            // render "last poll failed: <message>" without waiting for
            // the next successful tick. The renderer reads cached
            // state via `service_status_summary_get`; the event is
            // just a refresh ping, which is why we use the same
            // channel as success.
            if let Err(e) = app.emit(crate::events::SERVICE_STATUS_UPDATED, ()) {
                tracing::warn!(error = %e, "service_status_watcher: emit (failure path) failed");
            }
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

    // Phase 2 fix for audit issue #2: this watcher used to write
    // `source: Toast` even though no toast ever rendered.
    //
    // Audit-fix High #6: surfaces_requested is now derived from the
    // CategoryPrefs map (via `effective_os_surface`) rather than
    // the legacy `os_notify_on_status_change` scalar alone, so the
    // routing matches what the renderer's emit() would compute for
    // the same category. The settings.os_notify_on_change scalar
    // is the "wants_os" priority default; `os_override` flips it.
    // P3 categories normally don't want OS, but
    // ServiceStatusChanged opts in via the scalar.
    // Mutex-poison recovery — see usage_watcher's equivalent block
    // for the rationale. Long-lived watcher tasks must not panic
    // on a poisoned lock; notification routing is advisory.
    let surfaces_requested: Vec<Surface> = {
        let prefs_state = app.state::<crate::preferences::PreferencesState>();
        let guard = match prefs_state.0.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!(
                    "service_status_watcher: preferences mutex was poisoned by an earlier panic; \
                     recovering with under-lock data"
                );
                poisoned.into_inner()
            }
        };
        crate::preferences::effective_os_surface(
            &guard,
            Category::ServiceStatusChanged,
            settings.os_notify_on_change,
        )
    };
    let should_dispatch_os = surfaces_requested.contains(&Surface::OsBanner);

    // Attempt the OS dispatch first when requested, so we can record
    // the delivered outcome in the same append.
    let mut surfaces_delivered: Vec<Surface> = Vec::new();
    if should_dispatch_os {
        use tauri_plugin_notification::NotificationExt;
        match app
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            Ok(_) => surfaces_delivered.push(Surface::OsBanner),
            Err(e) => {
                tracing::warn!(error = %e, "service_status_watcher: OS notification failed");
            }
        }
    }

    // Audit-fix Medium #11: propagate persistence failures rather
    // than discarding them with `let _ =`. Caller already handles
    // `Result<(), String>` and the top-level tick logs any error.
    if let Some(log) = app.try_state::<NotificationLogState>() {
        if let Err(e) = log.log.append_routed(
            Category::ServiceStatusChanged,
            Priority::P3Ambient,
            NotificationKind::Notice,
            title.clone(),
            body.clone(),
            // No click target — clicking the entry doesn't navigate.
            // Wiring this to "Open Settings → Network" is a future polish.
            serde_json::Value::Null,
            surfaces_requested,
            surfaces_delivered,
        ) {
            tracing::warn!(error = %e, "service_status_watcher: log append failed");
            return Err(format!("notification_log append failed: {e}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(description: &str, components: Vec<core::Component>) -> core::StatusSummary {
        core::StatusSummary {
            page: core::PageInfo {
                id: "pg".to_string(),
                name: "Claude".to_string(),
                url: "https://status.claude.com".to_string(),
                updated_at: None,
            },
            status: core::StatusIndicator {
                indicator: "none".to_string(),
                description: description.to_string(),
            },
            components,
            incidents: Vec::new(),
            scheduled_maintenances: Vec::new(),
        }
    }

    fn component(name: &str, status: &str) -> core::Component {
        core::Component {
            id: format!("c-{name}"),
            name: name.to_string(),
            status: status.to_string(),
            description: None,
        }
    }

    #[test]
    fn test_transition_message_recovery_names_previous_tier() {
        let s = summary("All Systems Operational", vec![]);
        let (title, body) =
            transition_message(core::StatusTier::Degraded, core::StatusTier::Ok, &s);
        assert_eq!(title, "Claude services back to normal");
        assert_eq!(
            body,
            "Recovered from degraded. Status: All Systems Operational."
        );
    }

    #[test]
    fn test_transition_message_degraded_lists_affected_components() {
        let s = summary(
            "Partial outage",
            vec![
                component("API", "partial_outage"),
                component("Console", "operational"),
                component("claude.ai", "degraded_performance"),
            ],
        );
        let (title, body) =
            transition_message(core::StatusTier::Ok, core::StatusTier::Degraded, &s);
        assert_eq!(title, "Claude services degraded");
        // Only the non-Ok components appear, in summary order.
        assert_eq!(body, "Affected: API, claude.ai. Partial outage");
    }

    #[test]
    fn test_transition_message_truncates_affected_to_three() {
        let s = summary(
            "Major outage",
            vec![
                component("A", "major_outage"),
                component("B", "major_outage"),
                component("C", "major_outage"),
                component("D", "major_outage"),
                component("E", "major_outage"),
            ],
        );
        let (title, body) = transition_message(core::StatusTier::Ok, core::StatusTier::Down, &s);
        assert_eq!(title, "Claude services down");
        // take(3) truncation — D and E are dropped.
        assert_eq!(body, "Affected: A, B, C. Major outage");
    }

    #[test]
    fn test_transition_message_empty_components_falls_back_to_description() {
        let s = summary("Investigating elevated errors", vec![]);
        let (_, body) = transition_message(core::StatusTier::Ok, core::StatusTier::Degraded, &s);
        assert_eq!(body, "Investigating elevated errors");
    }

    #[test]
    fn test_transition_message_all_ok_components_falls_back_to_description() {
        // Components exist but none are degraded (page-level indicator
        // drove the tier) — same fallback as the empty list.
        let s = summary(
            "Elevated error rates",
            vec![component("API", "operational")],
        );
        let (_, body) = transition_message(core::StatusTier::Ok, core::StatusTier::Degraded, &s);
        assert_eq!(body, "Elevated error rates");
    }
}
