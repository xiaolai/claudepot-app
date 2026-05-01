//! Periodic poller that fires OS notifications when the CLI-active
//! account's Anthropic usage utilization crosses a configured
//! threshold (e.g. 80 %, 90 %).
//!
//! Runs as a single tokio task spawned in `setup()`. The task owns
//! the `UsageAlertState` (loaded once at start, persisted after every
//! fired-set mutation). On each tick it:
//!
//!   1. Reads `notify_on_usage_thresholds` and `activity_enabled`
//!      from the live `Preferences` snapshot. Empty list or activity
//!      disabled → tick is a no-op.
//!   2. Resolves the CLI-active account uuid via `AccountStore`.
//!   3. Calls `UsageCache::fetch_usage_graceful` (rate-limit
//!      cooldowns are absorbed; we'll re-check next tick).
//!   4. Folds the response through
//!      `services::usage_alerts::UsageAlertState::apply_crossings`.
//!   5. For every newly-detected crossing, emits a frontend event
//!      `usage-threshold-crossed` carrying enough metadata for the
//!      JS side to render the OS toast.
//!
//! The task is fire-and-forget for the app's lifetime — it sleeps
//! 5 minutes between ticks, so cancelling cleanly on shutdown is
//! not load-bearing. Tokio drops the task when the runtime exits.
//!
//! The pure crossing math + persistence lives in core
//! (`services::usage_alerts`) where it can be unit-tested without
//! a webview. Orchestration (state access, scheduling, emit) is
//! Tauri's job and lives here.

use std::time::Duration;

use claudepot_core::services::usage_alerts::{Crossing, UsageAlertState, UsageWindowKind};
// `Vec<UsageWindowKind>` is built per-tick from the user's
// `notify_on_sub_windows` preference; the umbrella `seven_day` and
// `five_hour` are always included.
use claudepot_core::services::usage_cache::UsageCache;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// Production poll interval. Anthropic's `/usage` endpoint is cheap
/// (no token spend), so a tighter cadence costs little; 5 min keeps
/// crossing latency well within "I'd notice within a few minutes"
/// UX expectations while leaving headroom under the per-account 60s
/// `UsageCache::CACHE_TTL` (every 5 min poll IS a fresh fetch).
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Delay before the very first tick. Without it, a cold-start where
/// utilization is already past a threshold could fire + persist
/// before the renderer's `useUsageThresholdNotifications` listener
/// is wired up — and the persisted fired-set means the alert is
/// never re-emitted for that cycle, so the OS toast is silently
/// lost. 5 s is a generous upper bound on how long the webview
/// needs to mount + register the listener; well below the 5-minute
/// tick cadence, so users still see toggle effects "immediately"
/// in any human sense.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(5);

/// Frontend event payload for a single threshold crossing. The
/// renderer (`src/hooks/useUsageThresholdNotifications.ts`) listens
/// on the `usage-threshold-crossed` channel and translates each
/// payload into one OS toast via `dispatchOsNotification`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageThresholdCrossedPayload {
    /// Anthropic account uuid that owns this credential. Stable across
    /// renames and re-logins; the renderer maps it to a display email.
    pub account_uuid: String,
    /// Display email for the active CLI account at the moment the
    /// crossing was detected. Pre-resolved here so the renderer
    /// doesn't have to round-trip a separate `account_list` call
    /// just to render a banner title.
    pub account_email: Option<String>,
    /// Stable window kind (`five_hour`, `seven_day`, …). Mirrors
    /// `UsageWindowKind` — kept as a serialized string so the JS side
    /// can switch on it without a parallel enum.
    pub window: String,
    /// Human-readable label (e.g. "5-hour window") matching
    /// `UsageWindowKind::label`. Sent through verbatim so the toast
    /// title doesn't depend on JS knowing the canonical labels.
    pub window_label: String,
    /// The threshold (integer percent) that just fired. The renderer
    /// interpolates this into the title (e.g. "at 80%").
    pub threshold_pct: u32,
    /// Server-reported utilization at fire time. Always ≥
    /// `threshold_pct`. Sent so the toast can show the precise value
    /// (which may be well above the threshold if a poll was missed).
    pub utilization_pct: f64,
    /// ISO-8601 reset time, when known. The renderer formats it as
    /// "resets in 2h 14m" using the local clock.
    pub resets_at_iso: Option<String>,
}

/// Spawn the poll loop. Called once from `setup()`; the spawned task
/// runs for the lifetime of the app.
///
/// Only takes `AppHandle` — `UsageCache` is reached via
/// `app.state::<UsageCache>()` inside each tick, which is the only
/// safe way to share the singleton cache without exposing an Arc
/// at the `manage()` site (and thereby breaking every existing
/// `State<'_, UsageCache>` consumer).
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Load alert state from disk once. The task owns the only
        // mutable handle for its lifetime — no other writer touches
        // `usage_alert_state.json`, so an in-task mutex is unnecessary.
        let mut state = match tauri::async_runtime::spawn_blocking(UsageAlertState::load).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "usage_watcher: state load join failed; starting fresh");
                UsageAlertState::new()
            }
        };

        // Wait for the renderer's listener to come up before the
        // first tick (see FIRST_TICK_DELAY for why). After that, run
        // ticks continuously: the user wants toggle effects visible
        // within minutes of flipping a switch, not after a full poll
        // window.
        tokio::time::sleep(FIRST_TICK_DELAY).await;

        loop {
            run_tick(&app, &mut state).await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

/// Single poll cycle. Broken out so a future test can drive the
/// state machine without spawning the task. Currently the pure
/// crossing math is exercised end-to-end via the detector tests in
/// core; this orchestration layer is covered by manual smoke.
async fn run_tick(app: &AppHandle, state: &mut UsageAlertState) {
    // 1. Snapshot prefs. Holding the std::sync mutex across an
    //    `await` is forbidden, so we clone the relevant fields and
    //    drop the guard before any async call.
    //
    //    Usage-threshold polling is a separate concern from the live
    //    transcript runtime — it hits Anthropic's /usage endpoint
    //    directly and never touches PID files or transcript JSONL.
    //    Gating this on `activity_enabled` would silently link two
    //    independent opt-ins (a user who disabled the activity
    //    feature for privacy reasons would also lose their quota
    //    alerts). The threshold list IS the opt-in for this watcher.
    let (thresholds, kinds) = {
        let prefs_state = app.state::<crate::preferences::PreferencesState>();
        let guard = match prefs_state.0.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(error = %e, "usage_watcher: prefs lock poisoned, skipping tick");
                return;
            }
        };
        // `kinds` is the per-poll filter passed to apply_crossings.
        // The umbrella windows (5-hour and 7-day) are always checked;
        // the per-model sub-windows (Opus, Sonnet) are gated behind
        // `notify_on_sub_windows` because they typically track the
        // umbrella for users near cap, so leaving them on triples the
        // 7-day toast volume for what most users experience as "one
        // cap." See preferences::Preferences::notify_on_sub_windows
        // for the rationale.
        let mut kinds = vec![UsageWindowKind::FiveHour, UsageWindowKind::SevenDay];
        if guard.notify_on_sub_windows {
            kinds.push(UsageWindowKind::SevenDayOpus);
            kinds.push(UsageWindowKind::SevenDaySonnet);
        }
        (guard.notify_on_usage_thresholds.clone(), kinds)
    };
    if thresholds.is_empty() {
        // Feature off — nothing to do. We deliberately do NOT clear
        // `state` here; if the user re-enables, the existing fired-set
        // is still valid for the current cycles.
        return;
    }

    // 2. Resolve the CLI-active account. SQLite open is fast but
    //    blocking; spawn_blocking keeps us off the async worker.
    let active = match tauri::async_runtime::spawn_blocking(resolve_active_cli).await {
        Ok(Ok(Some(pair))) => pair,
        Ok(Ok(None)) => return, // No active CLI — nothing to watch.
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "usage_watcher: store lookup failed");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "usage_watcher: spawn_blocking join failed");
            return;
        }
    };
    let (account_uuid, account_email) = active;

    // 3. Fetch usage. `fetch_usage_graceful` swallows rate-limit /
    //    cooldown errors and returns None, which is what we want —
    //    the next tick will retry naturally.
    //
    //    `app.state::<UsageCache>()` returns a `State<'_, UsageCache>`
    //    whose lifetime is tied to the borrow; the `await` inside the
    //    fetch must stay within that borrow, so we hold the State for
    //    the call's duration.
    let resp = {
        let cache_state = app.state::<UsageCache>();
        let cache: &UsageCache = &cache_state;
        match cache.fetch_usage_graceful(account_uuid).await {
            Some(r) => r,
            None => return,
        }
    };

    // 4. Detect crossings. `kinds` reflects the user's
    //    sub-window opt-in; `apply_crossings` coalesces multi-
    //    threshold crossings within a single window/poll to one
    //    Crossing for the highest threshold.
    let crossings = state.apply_crossings(account_uuid, &resp, &thresholds, &kinds);
    if crossings.is_empty() {
        return;
    }

    // 5. Persist updated fired-sets BEFORE emitting events. The order
    //    matters: when persistence fails, we MUST NOT emit, because
    //    the in-memory fired-set is the next-launch dedupe and a
    //    successful emit + failed save means the user gets a
    //    duplicate toast on the next cold start.
    //
    //    Trade-off when save fails: the user silently misses this
    //    cycle's alert, but they don't get spammed on restart. The
    //    failure path is rare (disk full or ~/.claudepot/ permissions
    //    are unusual), and the journal warning gives ops a recovery
    //    breadcrumb. The previous version logged JoinError but
    //    silently dropped real I/O failures AND emitted anyway,
    //    which guaranteed dupe-on-restart with no diagnostic.
    let save_state = state.clone();
    let save_outcome = tauri::async_runtime::spawn_blocking(move || save_state.save()).await;
    match save_outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!(
                error = %e,
                "usage_watcher: alert state save failed; suppressing emit to avoid dupe-on-restart"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "usage_watcher: persistence join failed; suppressing emit to avoid dupe-on-restart"
            );
            return;
        }
    }

    // 6. Emit one event per crossing. The dispatcher on the JS side
    //    applies its own dedupe key, focus gate, and OS-permission
    //    check — we don't gate any of that here.
    for c in crossings {
        let payload = make_payload(&c, account_email.as_deref());
        if let Err(e) = app.emit("usage-threshold-crossed", payload) {
            tracing::warn!(error = %e, "usage_watcher: emit failed");
        }
    }
}

fn resolve_active_cli() -> Result<Option<(Uuid, Option<String>)>, String> {
    let store = crate::commands::open_store()?;
    let raw = store
        .active_cli_uuid()
        .map_err(|e| format!("active_cli_uuid failed: {e}"))?;
    let uuid_str = match raw {
        Some(s) => s,
        None => return Ok(None),
    };
    let uuid = Uuid::parse_str(&uuid_str).map_err(|e| format!("active uuid parse failed: {e}"))?;
    let email = store.find_by_uuid(uuid).ok().flatten().map(|a| a.email);
    Ok(Some((uuid, email)))
}

fn make_payload(c: &Crossing, account_email: Option<&str>) -> UsageThresholdCrossedPayload {
    let window_str = match c.window {
        UsageWindowKind::FiveHour => "five_hour",
        UsageWindowKind::SevenDay => "seven_day",
        UsageWindowKind::SevenDayOpus => "seven_day_opus",
        UsageWindowKind::SevenDaySonnet => "seven_day_sonnet",
    };
    UsageThresholdCrossedPayload {
        account_uuid: c.account_uuid.to_string(),
        account_email: account_email.map(str::to_owned),
        window: window_str.to_owned(),
        window_label: c.window.label().to_owned(),
        threshold_pct: c.threshold_pct,
        utilization_pct: c.utilization_pct,
        resets_at_iso: c.resets_at.map(|t| t.to_rfc3339()),
    }
}
