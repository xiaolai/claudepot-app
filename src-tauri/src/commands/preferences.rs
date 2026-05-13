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
//
// One arg per preference — Tauri commands deserialize each named
// arg from the JS side, so bundling these into a struct would mean
// every JS caller has to send one nested object instead of a flat
// patch. Keep the flat shape; allow the lint per-function.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn preferences_set_notifications(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    on_error: Option<bool>,
    on_idle_done: Option<bool>,
    on_stuck_minutes: Option<Option<u32>>,
    on_op_done: Option<bool>,
    on_waiting: Option<bool>,
    on_usage_thresholds: Option<Vec<u32>>,
    on_sub_windows: Option<bool>,
) -> Result<crate::preferences::Preferences, String> {
    // Mirror the audit-B8 pattern from `preferences_set_activity`:
    // mutate the in-memory snapshot under the std::sync guard, drop
    // the guard, then hand the disk write to a blocking task so the
    // IPC worker doesn't sit on a `write_all` while every other
    // preferences read contends for the same mutex.
    use claudepot_core::notifications::Category;
    let snapshot = {
        let mut prefs = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        // Mirror every scalar setter back into `category_prefs` via
        // `set_category_pref` so the new routing pipeline sees the
        // change immediately. Without this, the emit() facade keeps
        // reading the stale CategoryPrefs map and toggles "applied"
        // by the user via this legacy IPC have no behavioral effect
        // on dispatch. (Audit-fix High #3.)
        //
        // We snapshot the existing os_override BEFORE calling
        // set_category_pref so the borrow checker is happy (one
        // mutable borrow at a time on `prefs`).
        if let Some(v) = on_error {
            let os_override = prefs.category_pref(Category::SessionErrorBurst).os_override;
            prefs.set_category_pref(
                Category::SessionErrorBurst,
                crate::preferences::CategoryPrefs {
                    enabled: v,
                    os_override,
                },
            );
        }
        if let Some(v) = on_idle_done {
            prefs.notify_on_idle_done = v;
            // OpDoneUnfocused maps to both idle-done and op-done
            // scalars; only flip the category to disabled if BOTH
            // scalars are off (matches migrate_to_v1's OR semantics).
            let combined = v || prefs.notify_on_op_done;
            let os_override = prefs.category_pref(Category::OpDoneUnfocused).os_override;
            prefs.set_category_pref(
                Category::OpDoneUnfocused,
                crate::preferences::CategoryPrefs {
                    enabled: combined,
                    os_override,
                },
            );
        }
        if let Some(v) = on_stuck_minutes {
            prefs.notify_on_stuck_minutes = v;
            let os_override = prefs.category_pref(Category::SessionStuck).os_override;
            prefs.set_category_pref(
                Category::SessionStuck,
                crate::preferences::CategoryPrefs {
                    enabled: v.is_some(),
                    os_override,
                },
            );
        }
        if let Some(v) = on_op_done {
            prefs.notify_on_op_done = v;
            let combined = v || prefs.notify_on_idle_done;
            let os_override = prefs.category_pref(Category::OpDoneUnfocused).os_override;
            prefs.set_category_pref(
                Category::OpDoneUnfocused,
                crate::preferences::CategoryPrefs {
                    enabled: combined,
                    os_override,
                },
            );
        }
        if let Some(v) = on_waiting {
            let os_override = prefs.category_pref(Category::SessionWaiting).os_override;
            prefs.set_category_pref(
                Category::SessionWaiting,
                crate::preferences::CategoryPrefs {
                    enabled: v,
                    os_override,
                },
            );
        }
        if let Some(mut v) = on_usage_thresholds {
            // Normalize: clamp to 1..=100, sort ascending, dedupe.
            // 0 is a no-op (always crossed); 100 is the upper bound
            // the watcher will actually fire when usage saturates,
            // so include it in the meaningful range. Empty vec is
            // allowed (= feature off) and survives the
            // normalization unchanged.
            v.retain(|&t| (1..=100).contains(&t));
            v.sort_unstable();
            v.dedup();
            prefs.notify_on_usage_thresholds = v.clone();
            let os_override = prefs.category_pref(Category::UsageThreshold).os_override;
            prefs.set_category_pref(
                Category::UsageThreshold,
                crate::preferences::CategoryPrefs {
                    enabled: !v.is_empty(),
                    os_override,
                },
            );
        }
        if let Some(v) = on_sub_windows {
            prefs.notify_on_sub_windows = v;
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
    // Audit-fix Medium #13: snapshot under the lock, drop the
    // guard, then persist on a blocking thread so the std::sync
    // mutex isn't held across the disk write. Matches the pattern
    // used by `preferences_set_activity` /
    // `preferences_set_notifications` etc.
    let snapshot = {
        let mut p = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        p.show_window_on_startup = show;
        p.clone()
    };
    tokio::task::spawn_blocking(move || snapshot.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))?
}

/// Set fields on the `service_status` preference block. Same
/// "optional per field" shape as `preferences_set_activity`. The
/// poll-interval value is clamped to `[2, 60]` minutes; values
/// outside that range are silently coerced rather than rejected so
/// the renderer doesn't have to mirror the policy.
#[tauri::command]
pub async fn preferences_set_service_status(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    poll_status_page: Option<bool>,
    poll_interval_minutes: Option<u32>,
    os_notify_on_status_change: Option<bool>,
    probe_latency_on_focus: Option<bool>,
) -> Result<crate::preferences::Preferences, String> {
    let snapshot = {
        let mut prefs = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        if let Some(v) = poll_status_page {
            prefs.service_status.poll_status_page = v;
        }
        if let Some(v) = poll_interval_minutes {
            prefs.service_status.poll_interval_minutes = v.clamp(2, 60);
        }
        if let Some(v) = os_notify_on_status_change {
            // Mirror into the routed CategoryPrefs map: the OS
            // banner gate for ServiceStatusChanged is honored
            // via `os_override`, so emit() / the watcher both
            // see the toggle. Audit-fix High #3.
            prefs.service_status.os_notify_on_status_change = v;
            let cat = claudepot_core::notifications::Category::ServiceStatusChanged;
            let enabled = prefs.category_pref(cat).enabled;
            prefs.set_category_pref(
                cat,
                crate::preferences::CategoryPrefs {
                    enabled,
                    os_override: Some(v),
                },
            );
        }
        if let Some(v) = probe_latency_on_focus {
            prefs.service_status.probe_latency_on_focus = v;
        }
        prefs.clone()
    };
    let to_persist = snapshot.clone();
    tokio::task::spawn_blocking(move || to_persist.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    Ok(snapshot)
}

/// Read every category's effective preference. The Settings pane
/// (Phase 4) reads this once on mount; emit() reads it via the
/// regular `preferences_get` snapshot. Includes implicit defaults
/// for categories that aren't yet in the persisted map.
#[tauri::command]
pub async fn preferences_category_prefs_get(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
) -> Result<
    std::collections::HashMap<
        claudepot_core::notifications::Category,
        crate::preferences::CategoryPrefs,
    >,
    String,
> {
    let p = state
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?;
    let mut out = p.category_prefs.clone();
    // Backfill missing categories with their per-category defaults
    // (reads `display_meta().default_enabled`). Plain
    // `.or_default()` would clobber the policy and treat every
    // category as enabled.
    for c in claudepot_core::notifications::Category::all() {
        out.entry(*c)
            .or_insert_with(|| crate::preferences::default_prefs_for(*c));
    }
    Ok(out)
}

/// Update a single category's preference. Mirrors any legacy scalar
/// (`notify_on_*`) that maps to the same category — see
/// `Preferences::set_category_pref`. Persists and returns the
/// refreshed `CategoryPrefs` for the renderer to mirror locally.
#[tauri::command]
pub async fn preferences_category_pref_set(
    state: tauri::State<'_, crate::preferences::PreferencesState>,
    category: claudepot_core::notifications::Category,
    prefs: crate::preferences::CategoryPrefs,
) -> Result<crate::preferences::CategoryPrefs, String> {
    // Audit-fix Medium #13: snapshot under the lock, drop, persist
    // on a blocking task. The std::sync mutex MUST NOT be held
    // across the disk write — every other preferences reader
    // contends on the same lock.
    let (snapshot, refreshed) = {
        let mut p = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        p.set_category_pref(category, prefs);
        let refreshed = p.category_pref(category);
        (p.clone(), refreshed)
    };
    tokio::task::spawn_blocking(move || snapshot.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    Ok(refreshed)
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
    let snapshot = {
        let mut p = state
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        p.hide_dock_icon = hide;
        p.clone()
    };
    // Audit-fix Medium #13: persist off the std::sync mutex.
    tokio::task::spawn_blocking(move || snapshot.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))?
}
