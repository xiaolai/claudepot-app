//! Tauri commands for the Live Activity feature.
//!
//! Pure pass-through to `LiveActivityService` in `claudepot-core`.
//! Policy (consent gate, listener fan-out, membership debounce,
//! tray rebuild) lives in the service or in the registered
//! `TauriSessionEventListener`. These commands carry no business
//! logic of their own — they exist solely to bridge JS into core.

/// Start the live runtime. Idempotent: repeated calls after a first
/// successful start return `Ok(())` without re-spawning.
///
/// **Consent gate — trust boundary**: the runtime only starts if the
/// user has explicitly enabled the Activity feature via the consent
/// modal or Settings. A request to start while `activity_enabled ==
/// false` returns `Ok(())` silently — backed up server-side so a
/// rogue hook or CLI command also respects the user's choice. The
/// service has no opinion about preferences; the gate stays here.
#[tauri::command]
pub async fn session_live_start(
    state: tauri::State<'_, crate::state::LiveSessionState>,
    prefs: tauri::State<'_, crate::preferences::PreferencesState>,
) -> Result<(), String> {
    let (enabled, excluded) = {
        let p = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
        (p.activity_enabled, p.activity_excluded_paths.clone())
    };
    if !enabled {
        return Ok(());
    }
    state
        .service
        .start(excluded)
        .await
        .map_err(|e| e.to_string())
}

/// Stop the live runtime. Idempotent.
#[tauri::command]
pub async fn session_live_stop(
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    state.service.stop().await;
    Ok(())
}

/// Explicit unsubscribe for a per-session detail stream.
#[tauri::command]
pub async fn session_live_unsubscribe(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    state.service.unsubscribe_detail(&session_id).await;
    Ok(())
}

/// One-shot snapshot of currently-live sessions.
#[tauri::command]
pub async fn session_live_snapshot(
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<Vec<crate::dto::LiveSessionSummaryDto>, String> {
    Ok(state
        .service
        .snapshot()
        .iter()
        .cloned()
        .map(crate::dto::LiveSessionSummaryDto::from)
        .collect())
}

/// One-session snapshot for resync after `resync_required`.
#[tauri::command]
pub async fn session_live_session_snapshot(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<Option<crate::dto::LiveSessionSummaryDto>, String> {
    Ok(state
        .service
        .session_snapshot(&session_id)
        .await
        .map(crate::dto::LiveSessionSummaryDto::from))
}

/// Query the durable activity metrics store for the time-series
/// Trends view. Returns active-session counts bucketed across the
/// requested window plus a simple error-count total. An unavailable
/// metrics store (first-launch race, permission issue) returns an
/// all-zero series rather than an error.
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
    let Some(store) = state.service.metrics() else {
        return Ok(crate::dto::ActivityTrendsDto {
            from_ms,
            to_ms,
            bucket_width_ms: bucket_width,
            active_series: vec![0; buckets],
            error_count: 0,
        });
    };
    // `active_series` + `error_count` are SQLite reads on a shared
    // mutex; pushing them onto a blocking thread keeps the IPC
    // worker free even when the metrics DB is hot. The store is
    // `Arc<MetricsStore>` so it crosses thread boundaries safely.
    let series_store = store.clone();
    let error_store = store;
    let series_task = tokio::task::spawn_blocking(move || {
        series_store
            .active_series(from_ms, to_ms, buckets)
            .map_err(|e| format!("active_series: {e}"))
    });
    let error_task = tokio::task::spawn_blocking(move || {
        error_store
            .error_count(from_ms, to_ms)
            .map_err(|e| format!("error_count: {e}"))
    });
    let active_series = series_task
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    let error_count = error_task
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    Ok(crate::dto::ActivityTrendsDto {
        from_ms,
        to_ms,
        bucket_width_ms: bucket_width,
        active_series,
        error_count,
    })
}

/// Subscribe to per-session deltas. The service forwards every
/// received delta to all registered `SessionEventListener`s — the
/// `TauriSessionEventListener` re-emits them as `live::<sid>`
/// events. Single-subscriber per session — concurrent calls with
/// the same id return `LiveActivityError::AlreadySubscribed`.
#[tauri::command]
pub async fn session_live_subscribe(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<(), String> {
    state
        .service
        .subscribe_detail(&session_id)
        .await
        .map_err(|e| e.to_string())
}
