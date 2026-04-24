//! Tauri commands for the Live Activity feature.
//!
//! `LiveRuntime` polls `~/.claude/sessions` and tails each transcript;
//! these commands expose snapshot + subscribe semantics. Aggregate
//! updates fire on the `live-all` event channel; per-session deltas
//! fire on `live::<sessionId>`.
//!
//! Consent gate — **trust boundary**: the runtime only starts if the
//! user has explicitly enabled the Activity feature via the consent
//! modal or Settings. The frontend check at the consent-modal layer
//! is backed up by the server-side guard in `session_live_start`.

/// Start the live runtime. Idempotent: repeated calls after a first
/// successful start return `Ok(())` without re-spawning. The poll
/// loop publishes aggregate updates via the `live-all` event channel
/// and per-session deltas via `live::<sessionId>`.
///
/// **Consent gate — trust boundary**: the runtime only starts if the
/// user has explicitly enabled the Activity feature via the consent
/// modal or Settings. A request to start while `activity_enabled ==
/// false` returns `Ok(())` silently (so accidental callers don't
/// break) but the runtime stays off. The frontend check at the
/// consent-modal layer is backed up by this server-side guard so
/// future callers (e.g. a rogue hook or a CLI command) also respect
/// the user's choice.
#[tauri::command]
pub async fn session_live_start(
    state: tauri::State<'_, crate::state::LiveSessionState>,
    prefs: tauri::State<'_, crate::preferences::PreferencesState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Consent check MUST precede the started flag flip, or a user
    // who opted out after opting in would still get a running
    // runtime from a stale `started = true`.
    {
        let p = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
        if !p.activity_enabled {
            return Ok(());
        }
    }
    {
        let mut started = state.started.lock().map_err(|e| e.to_string())?;
        if *started {
            return Ok(());
        }
        *started = true;
    }

    // Spawn a watcher task that forwards aggregate updates to the
    // webview. The runtime publishes on the watch channel; we bridge
    // to Tauri's `emit` here so the webview sees one source of truth.
    //
    // Same task also schedules a debounced tray rebuild on live-set
    // changes so the tray's "Active: N" submenu stays in sync with
    // the sidebar strip. Debounce = 1s: at the 500ms poll cadence
    // this coalesces bursts of "session appeared/disappeared" into
    // a single rebuild, which matters on AppKit where menu rebuilds
    // are synchronous and visible.
    // Apply the user's current excluded-paths preference to the
    // runtime before it starts ticking. Without this the first
    // tick could publish a now-excluded project's live state.
    // Read-and-release the sync prefs lock in a short scope so no
    // std::sync guard lives across the .await below.
    let excluded: Vec<String> = {
        let p = prefs
            .0
            .lock()
            .map_err(|e| format!("prefs lock: {e}"))?;
        p.activity_excluded_paths.clone()
    };
    state.runtime.set_excluded_paths(excluded).await;

    let runtime = std::sync::Arc::clone(&state.runtime);
    let mut rx = runtime.subscribe_aggregate();
    let app_for_bridge = app.clone();
    let aggregate_handle = tokio::spawn(async move {
        use tokio::sync::Mutex as AsyncMutex;
        // Track the last-seen set of session IDs so we only rebuild
        // the tray when membership changes, not on every status
        // transition (which don't affect the top-level menu).
        let mut last_ids: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        // True debounce: a single shared pending flag that flips
        // off only AFTER the delayed rebuild actually runs, so
        // repeat membership changes within the window don't
        // schedule a second rebuild. Earlier impl reset the flag
        // in the same loop iteration that scheduled — no debounce
        // at all.
        let rebuild_pending = std::sync::Arc::new(AsyncMutex::new(false));
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            let list_arc = rx.borrow_and_update().clone();
            let list: Vec<crate::dto::LiveSessionSummaryDto> = list_arc
                .iter()
                .cloned()
                .map(crate::dto::LiveSessionSummaryDto::from)
                .collect();
            let _ = tauri::Emitter::emit(&app_for_bridge, "live-all", list);

            // Tray-rebuild trigger: membership set changed.
            let current_ids: std::collections::BTreeSet<String> = list_arc
                .iter()
                .map(|s| s.session_id.clone())
                .collect();
            if current_ids != last_ids {
                last_ids = current_ids;
                let mut guard = rebuild_pending.lock().await;
                if !*guard {
                    *guard = true;
                    drop(guard);
                    let handle = app_for_bridge.clone();
                    let pending = rebuild_pending.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(
                            std::time::Duration::from_secs(1),
                        )
                        .await;
                        // Clear the pending flag ONLY after the
                        // delay fires so another membership change
                        // inside the window is folded into this
                        // rebuild instead of scheduling a new one.
                        if let Err(e) = crate::tray::rebuild(&handle).await {
                            tracing::warn!(
                                "activity tray rebuild failed: {e}"
                            );
                        }
                        let mut g = pending.lock().await;
                        *g = false;
                    });
                }
            }
        }
    });

    // Stash the aggregate-bridge handle so session_live_stop can
    // abort it — without this, start/stop/start cycles accumulate
    // zombie emitters.
    {
        let mut tasks = state
            .bridge_tasks
            .lock()
            .map_err(|e| format!("bridge lock: {e}"))?;
        tasks.aggregate = Some(aggregate_handle);
    }

    // Start the poll loop.
    let _handle = std::sync::Arc::clone(&state.runtime).start();
    Ok(())
}

/// Stop the live runtime. Idempotent: stopping an already-stopped
/// runtime is a no-op. Aborts every bridge task spawned by
/// `session_live_start` (aggregate → live-all, and each
/// per-session → live::<sid>) so a subsequent start begins from a
/// clean task set instead of accumulating ghost emitters.
#[tauri::command]
pub async fn session_live_stop(
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    state.runtime.stop();
    {
        let mut tasks = state
            .bridge_tasks
            .lock()
            .map_err(|e| format!("bridge lock: {e}"))?;
        tasks.abort_all();
    }
    let mut started = state.started.lock().map_err(|e| e.to_string())?;
    *started = false;
    Ok(())
}

/// Explicit unsubscribe for a per-session detail stream. The Tauri
/// event listener on the JS side has no way to tell the backend
/// "stop forwarding" without this — dropping the listener alone
/// leaves the spawned task alive until the session ends.
#[tauri::command]
pub async fn session_live_unsubscribe(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    // Abort the backend bridge task inside a short scope so the
    // std::sync Mutex guard is dropped before we .await.
    {
        let mut tasks = state
            .bridge_tasks
            .lock()
            .map_err(|e| format!("bridge lock: {e}"))?;
        if let Some(h) = tasks.details.remove(&session_id) {
            h.abort();
        }
    }
    // Drop the backend-side slot in the DetailBus so a future
    // subscribe rebuilds cleanly without an AlreadySubscribed error.
    state.runtime.detail_end_session(&session_id).await;
    Ok(())
}

/// One-shot snapshot of currently-live sessions. Used by the webview
/// on first mount before the first `live-all` event arrives, and as
/// the resync answer after a gap. Returns `Result<_, String>` because
/// async Tauri commands that take reference inputs must return
/// `Result` (Tauri's codegen requirement).
#[tauri::command]
pub async fn session_live_snapshot(
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<Vec<crate::dto::LiveSessionSummaryDto>, String> {
    Ok(state
        .runtime
        .snapshot()
        .iter()
        .cloned()
        .map(crate::dto::LiveSessionSummaryDto::from)
        .collect())
}

/// One-session snapshot for resync after `resync_required`. Returns
/// `None` if the session is not currently live.
#[tauri::command]
pub async fn session_live_session_snapshot(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<Option<crate::dto::LiveSessionSummaryDto>, String> {
    Ok(state
        .runtime
        .session_snapshot(&session_id)
        .await
        .map(crate::dto::LiveSessionSummaryDto::from))
}

/// Query the durable activity metrics store for the time-series
/// Trends view. Returns active-session counts bucketed across the
/// requested window plus a simple error-count total. An unavailable
/// metrics store (first-launch race, permission issue) returns an
/// all-zero series rather than an error — the Trends view is
/// non-critical.
#[tauri::command]
pub async fn activity_trends(
    state: tauri::State<'_, crate::state::LiveSessionState>,
    from_ms: i64,
    to_ms: i64,
    bucket_count: u32,
) -> Result<crate::dto::ActivityTrendsDto, String> {
    let buckets = bucket_count as usize;
    let bucket_width = if buckets > 0 && to_ms > from_ms {
        (to_ms - from_ms) / buckets as i64
    } else {
        0
    };
    let Some(store) = state.runtime.metrics() else {
        return Ok(crate::dto::ActivityTrendsDto {
            from_ms,
            to_ms,
            bucket_width_ms: bucket_width,
            active_series: vec![0; buckets],
            error_count: 0,
        });
    };
    let active_series = store
        .active_series(from_ms, to_ms, buckets)
        .map_err(|e| format!("active_series: {e}"))?;
    let error_count = store
        .error_count(from_ms, to_ms)
        .map_err(|e| format!("error_count: {e}"))?;
    Ok(crate::dto::ActivityTrendsDto {
        from_ms,
        to_ms,
        bucket_width_ms: bucket_width,
        active_series,
        error_count,
    })
}

/// Subscribe to per-session deltas. Spawns a task that forwards
/// every received delta as a `live::<sessionId>` event. Single-
/// subscriber per session — concurrent calls with the same id
/// return `BusError::AlreadySubscribed`. The JS side should call
/// `session_live_unsubscribe` before dropping its listener so the
/// backend-side task doesn't outlive the frontend.
#[tauri::command]
pub async fn session_live_subscribe(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut rx = state
        .runtime
        .subscribe_detail(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    let channel = format!("live::{session_id}");
    let handle = tokio::spawn(async move {
        while let Some(delta) = rx.recv().await {
            let dto = crate::dto::LiveDeltaDto::from(delta);
            let _ = tauri::Emitter::emit(&app, &channel, dto);
        }
    });
    // Track the handle so session_live_unsubscribe (or _stop) can
    // abort it. Dropping the frontend listener alone keeps this
    // task alive until the session itself ends; with the explicit
    // unsubscribe path it goes away immediately.
    let mut tasks = state
        .bridge_tasks
        .lock()
        .map_err(|e| format!("bridge lock: {e}"))?;
    if let Some(prev) = tasks.details.insert(session_id, handle) {
        prev.abort();
    }
    Ok(())
}
