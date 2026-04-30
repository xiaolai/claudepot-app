//! Tauri commands for the Preferences pane.
//!
//! State lives in `crate::preferences::PreferencesState` (disk-backed
//! JSON). Each setter takes `Option<T>` per field so the webview can
//! flip one toggle without re-sending the others.

/// Read the current preferences snapshot. Cheap — a clone of the
/// mutex-guarded record.
#[tauri::command]
pub async fn preferences_get(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
) -> Result<crate::preferences::Preferences, String> {
    Ok(state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?
        .clone())
}

/// Set the complete `activity_*` preference block in one call.
/// Takes an optional value for each field so the webview can flip
/// one toggle without re-sending the others (e.g. flipping
/// `activity_enabled` from the consent modal). Returns the
/// refreshed snapshot so the UI stays in sync.
#[tauri::command]
pub async fn preferences_set_activity(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    live: tauri::State<'_, crate::state::LiveSessionState>,
    enabled: Option<bool>,
    consent_seen: Option<bool>,
    hide_thinking: Option<bool>,
    excluded_paths: Option<Vec<String>>,
) -> Result<crate::preferences::Preferences, String> {
    // Update the in-memory snapshot, drop the std::sync guard, then
    // hand the JSON write off to a blocking thread so the IPC worker
    // doesn't sit on a `write_all` (audit B8 commands_preferences.rs:47).
    // Splitting the mutate/persist phases also keeps the lock window
    // short, which matters because every other preferences read is
    // contending for the same mutex.
    let snapshot = {
        let mut prefs = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        if let Some(v) = enabled {
            prefs.activity_enabled = v;
        }
        if let Some(v) = consent_seen {
            prefs.activity_consent_seen = v;
        }
        if let Some(v) = hide_thinking {
            prefs.activity_hide_thinking = v;
        }
        if let Some(v) = excluded_paths {
            prefs.activity_excluded_paths = v.clone();
            // Propagate to the running runtime so the change takes
            // effect on the next tick instead of requiring a restart.
            // `set_excluded_paths` is async, so we fire-and-forget via
            // the tauri async runtime handle; the command itself stays
            // sync to keep its signature minimal.
            let service = live.service.clone();
            tauri::async_runtime::spawn(async move {
                service.set_excluded_paths(v).await;
            });
        }
        prefs.clone()
    };
    let to_persist = snapshot.clone();
    tokio::task::spawn_blocking(move || to_persist.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    Ok(snapshot)
}

/// Set the `notify_*` preference block in one call. Same "optional
/// per field" shape as `preferences_set_activity` — callers send
/// only the fields they changed.
#[tauri::command]
pub async fn preferences_set_notifications(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    on_error: Option<bool>,
    on_idle_done: Option<bool>,
    on_stuck_minutes: Option<Option<u32>>,
    on_op_done: Option<bool>,
    on_waiting: Option<bool>,
    on_usage_thresholds: Option<Vec<u32>>,
) -> Result<crate::preferences::Preferences, String> {
    // Mirror the audit-B8 pattern from `preferences_set_activity`:
    // mutate the in-memory snapshot under the std::sync guard, drop
    // the guard, then hand the disk write to a blocking task so the
    // IPC worker doesn't sit on a `write_all` while every other
    // preferences read contends for the same mutex.
    let snapshot = {
        let mut prefs = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        if let Some(v) = on_error {
            prefs.notify_on_error = v;
        }
        if let Some(v) = on_idle_done {
            prefs.notify_on_idle_done = v;
        }
        if let Some(v) = on_stuck_minutes {
            prefs.notify_on_stuck_minutes = v;
        }
        if let Some(v) = on_op_done {
            prefs.notify_on_op_done = v;
        }
        if let Some(v) = on_waiting {
            prefs.notify_on_waiting = v;
        }
        if let Some(mut v) = on_usage_thresholds {
            // Normalize: clamp to 1..=100, sort ascending, dedupe.
            // 0 is a no-op (always crossed), 100 is unreachable on the
            // server-reported utilization scale, so trim both ends to
            // the meaningful range. Empty vec is allowed (= feature
            // off) and survives the normalization unchanged.
            v.retain(|&t| (1..=100).contains(&t));
            v.sort_unstable();
            v.dedup();
            prefs.notify_on_usage_thresholds = v;
        }
        prefs.clone()
    };
    let to_persist = snapshot.clone();
    tokio::task::spawn_blocking(move || to_persist.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    Ok(snapshot)
}

/// Persist the "show main window on startup" toggle. Pure persistence
/// — the value is read at the next cold launch from `setup()` and
/// applied via `window.hide()` before the window is presented. Toggling
/// at runtime does not affect the currently-visible window; that is
/// intentional, the user can hide/show through the tray.
#[tauri::command]
pub async fn preferences_set_show_window_on_startup(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    show: bool,
) -> Result<(), String> {
    let mut p = state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?;
    p.show_window_on_startup = show;
    p.save()
}

/// Toggle the dock-icon visibility (macOS only). On non-macOS platforms
/// the call still persists the boolean so the UI round-trips cleanly,
/// but the activation policy is a no-op.
#[tauri::command]
pub async fn preferences_set_hide_dock_icon(
    #[allow(unused_variables)] app: tauri::AppHandle,
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    hide: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let policy = if hide {
            tauri::ActivationPolicy::Accessory
        } else {
            tauri::ActivationPolicy::Regular
        };
        app.set_activation_policy(policy)
            .map_err(|e| format!("set_activation_policy: {e}"))?;
    }
    let mut p = state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?;
    p.hide_dock_icon = hide;
    p.save()
}
