mod app_menu;
mod cc_doctor_watcher;
mod commands;
mod config_dto;
mod config_watch;
mod config_watch_types;
#[cfg(target_os = "macos")]
mod dock_icon;
mod dto;
mod dto_account;
mod dto_activity;
mod dto_activity_cards;
mod dto_artifact_lifecycle;
mod dto_artifact_usage;
mod dto_automations;
mod dto_cc_doctor;
mod dto_cc_tips;
mod dto_desktop;
mod dto_env;
mod dto_keys;
mod dto_memory;
mod dto_migrate;
mod dto_permission;
mod dto_project;
mod dto_project_repair;
mod dto_rotation;
mod dto_routes;
mod dto_service_status;
mod dto_session;
mod dto_session_debug;
mod dto_session_move;
mod dto_session_prune;
mod dto_templates;
mod dto_updates;
mod dto_usage;
mod live_activity_bridge;
mod memory_watch;
mod ops;
mod permission_orchestrator;
mod pr_orchestrator;
mod preferences;
mod rotation_orchestrator;
mod service_status_watcher;
mod state;
mod traffic_light;
mod tray;
mod tray_icons;
mod tray_menu;
mod updates_watcher;
mod usage_snapshot;
mod usage_watcher;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Mirror the CLI's tracing setup so GUI swaps, keychain fallback, and
    // Desktop lifecycle events surface in the terminal that launched the app.
    // Honors RUST_LOG; defaults to info-level for our crates.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("claudepot_core=info,claudepot_tauri=info")
            }),
        )
        .try_init();

    // One-time: move the legacy `~/.claude/claudepot/` repair tree into
    // `~/.claudepot/repair/`. Idempotent and safe to run on every boot;
    // any error here is non-fatal — the app still works against whatever
    // layout is currently on disk.
    if let Err(e) = claudepot_core::migrations::migrate_repair_tree() {
        tracing::warn!("repair tree migration failed: {e}");
    }

    // Load persisted preferences BEFORE the builder constructs anything
    // that might need them. `hide_dock_icon` in particular must reach
    // `set_activation_policy()` inside the very first `setup()` tick to
    // avoid a visible dock-icon flash on cold launch.
    let prefs = preferences::Preferences::load();
    // Load update settings + cached probe state alongside prefs so the
    // first `updates_status_get` call from the webview is a pure clone
    // off the mutex (no disk I/O on the hot UI render path).
    let updates_state = claudepot_core::updates::UpdateState::load();
    // `hide_dock` is only consumed inside `#[cfg(target_os = "macos")]`
    // (set_activation_policy is a no-op everywhere else), so binding it
    // unconditionally would emit `unused_variable` on Linux + Windows.
    // Gate the bind to keep the production build clean while still
    // honouring the cold-load order — `prefs` itself is read on every
    // platform; only the macOS-specific extraction is gated.
    #[cfg(target_os = "macos")]
    let hide_dock = prefs.hide_dock_icon;
    let show_window_on_startup = prefs.show_window_on_startup;

    // Open the persistent notification log before the builder chain
    // so its handle can be `.manage()`d alongside the other state. The
    // log lives at `~/.claudepot/notifications.json`; `open` returns
    // an empty log on missing or corrupt files (the corrupt file gets
    // moved aside for forensics) so this never blocks startup.
    let notification_log_path = claudepot_core::notification_log::default_path();
    let notification_log_state = match claudepot_core::notification_log::NotificationLog::open(
        notification_log_path.clone(),
    ) {
        Ok(log) => commands::notification::NotificationLogState::new(log),
        Err(e) => {
            tracing::warn!(
                target = "claudepot_tauri",
                error = %e,
                path = %notification_log_path.display(),
                "notification log open failed — appends will be no-ops, surface stays empty"
            );
            // Two-step fallback: first try a temp-dir path so at
            // least the current process gets persistence inside the
            // session. If even that fails (no temp dir, permissions,
            // disk full), drop to a fully in-memory log so the bell
            // still works for this run instead of panicking startup.
            // The previous version `.expect()`d on the temp-path
            // open and aborted the whole app on a deeper environment
            // issue — strictly worse than degraded notifications.
            let fallback = std::env::temp_dir().join("claudepot-notifications.json");
            let log = claudepot_core::notification_log::NotificationLog::open(fallback.clone())
                .unwrap_or_else(|e2| {
                    tracing::warn!(
                        target = "claudepot_tauri",
                        error = %e2,
                        path = %fallback.display(),
                        "notification log temp-dir fallback also failed — using volatile in-memory log; entries will not survive restart"
                    );
                    claudepot_core::notification_log::NotificationLog::in_memory_only()
                });
            commands::notification::NotificationLogState::new(log)
        }
    };

    // Open the activity-cards index before the builder chain so we can
    // wire it into the live runtime AND publish it as IPC state in the
    // same `.manage()` series. Lives in the same `sessions.db` file as
    // SessionIndex (SQLite WAL lets the two handles coexist). Open
    // failure (disk full, perms, corrupt) degrades gracefully — the
    // cards surface goes dark; the live-strip and everything else
    // continues to work.
    let cards_db_path = claudepot_core::paths::claudepot_data_dir().join("sessions.db");
    let cards_index: Option<std::sync::Arc<claudepot_core::activity::ActivityIndex>> =
        match claudepot_core::activity::ActivityIndex::open(&cards_db_path) {
            Ok(idx) => Some(std::sync::Arc::new(idx)),
            Err(e) => {
                tracing::warn!(
                    target = "claudepot_tauri",
                    error = %e,
                    path = %cards_db_path.display(),
                    "activity-cards index open failed — cards surface degraded"
                );
                None
            }
        };

    // Shared `SessionIndex` for the Shared Memory commands. Opens
    // the same `sessions.db`. SessionIndex owns a `Mutex<Connection>`
    // internally so writes serialize; sharing one handle across all
    // Shared Memory commands avoids the per-call open-and-migrate
    // contention that surfaced as `SQLITE_BUSY` ("database is
    // locked") in the dev pane. Open failure degrades the Shared
    // Memory section to a "session index unavailable" banner; the
    // rest of the app keeps working.
    let shared_memory_index: Option<std::sync::Arc<claudepot_core::session_index::SessionIndex>> =
        match claudepot_core::session_index::SessionIndex::open(&cards_db_path) {
            Ok(idx) => Some(std::sync::Arc::new(idx)),
            Err(e) => {
                tracing::warn!(
                    target = "claudepot_tauri",
                    error = %e,
                    path = %cards_db_path.display(),
                    "shared-memory session index open failed — Shared Memory section degraded"
                );
                None
            }
        };

    // (Live state is built inside the `.manage` chain below — the
    // service-refactor pattern owns the runtime, so we hand the
    // cards index into the service's `enable_activity` accessor at
    // construction time.)

    // Memory change-log database. Lives at
    // `~/.claudepot/memory_changes.db`. Two-step fallback mirrors the
    // notification-log pattern: try the canonical path; if that fails,
    // try a temp-dir copy so the current process still has a working
    // log; only if BOTH fail do we panic (extremely unlikely — both
    // home and temp would have to be unwritable).
    //
    // Audit 2026-05 #2: every memory IPC command requires
    // `MemoryLogState`, so this MUST always succeed. The previous
    // `Option<Arc<MemoryLog>>` shape silently broke the entire pane on
    // open failure because `.manage()` was gated on the option.
    let memory_log: std::sync::Arc<claudepot_core::memory_log::MemoryLog> = {
        let primary = claudepot_core::paths::claudepot_data_dir().join("memory_changes.db");
        match claudepot_core::memory_log::MemoryLog::open(&primary) {
            Ok(l) => std::sync::Arc::new(l),
            Err(e) => {
                tracing::warn!(
                    target = "claudepot_tauri",
                    error = %e,
                    path = %primary.display(),
                    "memory change-log open failed at canonical path; falling back to temp dir"
                );
                let fallback = std::env::temp_dir().join("claudepot-memory_changes.db");
                std::sync::Arc::new(
                    claudepot_core::memory_log::MemoryLog::open(&fallback).unwrap_or_else(|e2| {
                        // Both writable locations failed — this is
                        // catastrophic for log persistence but the
                        // app can still run. Panic with a clear
                        // message so users see the underlying issue
                        // rather than a generic "command failed".
                        panic!(
                            "memory change-log unrecoverable: primary={} ({e}); \
                                 fallback={} ({e2})",
                            primary.display(),
                            fallback.display(),
                        );
                    }),
                )
            }
        }
    };

    // Clone the memory-log handle for the setup closure so it can spawn
    // the watcher; the original Arc stays available for `.manage()`.
    let memory_log_for_watcher: std::sync::Arc<claudepot_core::memory_log::MemoryLog> =
        memory_log.clone();

    // Open the rotation audit log alongside the other persistent stores
    // so the orchestrator can record every swap attempt at boot. Same
    // boot-fallback story as the notification log: missing → empty;
    // corrupt → renamed aside; canonical-path-fails → in-memory-only.
    let rotation_audit: std::sync::Arc<claudepot_core::rotation::RotationAuditLog> =
        std::sync::Arc::new(claudepot_core::rotation::RotationAuditLog::open_default());
    let rotation_orchestrator = std::sync::Arc::new(
        rotation_orchestrator::RotationOrchestrator::new(rotation_audit.clone()),
    );

    // `mut` is only consumed by the debug-only plugin block below;
    // release builds don't touch it. Silence the release warning here.
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        // Auto-update pipeline. The plugin reads endpoints + pubkey from
        // tauri.conf.json `plugins.updater`; `latest.json` lives as a
        // release asset on this repo, so the URL self-tracks the newest
        // published release. Dev builds: the plugin loads but `check()`
        // fails fast (no signed `latest.json` for unreleased versions),
        // and the renderer surfaces that as an error toast — no crash.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // `process::relaunch` is invoked after the user clicks
        // "Restart to update" — the renderer runs it once the updater
        // has staged the new bundle. Capability gates the IPC.
        .plugin(tauri_plugin_process::init())
        // D-5/6/7: Rust-side clipboard write for `key_*_copy`. Permissions
        // restricted in `capabilities/default.json` to write/read/clear —
        // the renderer never invokes the plugin directly (its only
        // consumer is our own `commands_keys.rs`), but the read-text
        // permission is required so the 30s self-clear can verify the
        // clipboard still holds our payload before clobbering it.
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            use tauri::{
                image::Image,
                menu::MenuEvent,
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
                Listener, Manager,
            };

            // First priority: apply activation policy before the window
            // is materialized. On macOS this hides the dock icon and
            // removes the app from Cmd+Tab.
            #[cfg(target_os = "macos")]
            if hide_dock {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // Force the Dock icon through Cocoa's NSImage pipeline
            // with our 512×512 source so non-128 Dock sizes (the
            // default 96-px render in particular) get high-quality
            // Lanczos downsampling instead of the legacy IconServices
            // bilinear-from-128-layer path. See `dock_icon.rs`.
            #[cfg(target_os = "macos")]
            dock_icon::override_application_icon();

            // Honor the "show window on startup" preference. The window
            // is configured `visible: true` in tauri.conf.json so the
            // default cold-launch path stays unchanged; users who opt
            // out get an immediate `hide()` here. Recovery path is the
            // tray icon (left-click toggles visibility).
            //
            // While we have the window handle, install the close-button
            // intercept: red ✕ → hide window, NOT exit. The app is
            // tray-resident — see `app_menu.rs` for the same policy on
            // ⌘Q and ⌘W. The only surface that actually terminates
            // the process is the tray dropdown's Quit (gated by
            // RunningOps in `attempt_quit`). Without `prevent_close`
            // here, Tauri tears down the only window and macOS exits
            // the app, breaking every background watcher.
            if let Some(window) = app.get_webview_window("main") {
                if !show_window_on_startup {
                    let _ = window.hide();
                }
                let win_for_handler = window.clone();
                let win_for_tl = window.clone();
                window.on_window_event(move |event| {
                    use tauri::WindowEvent;
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Err(e) = win_for_handler.hide() {
                            tracing::warn!("close-button: hide failed: {e}");
                        }
                    }
                    // Re-emit traffic-light metrics whenever AppKit
                    // could have moved the buttons. ScaleFactorChanged
                    // covers a monitor swap; Focused covers the
                    // bring-to-front path where the lights re-tint
                    // and AppKit re-lays-out the standard window
                    // buttons.
                    if matches!(
                        event,
                        WindowEvent::Resized(_)
                            | WindowEvent::Moved(_)
                            | WindowEvent::Focused(true)
                            | WindowEvent::ScaleFactorChanged { .. }
                    ) {
                        traffic_light::emit(&win_for_tl);
                    }
                });

                // Two-stage initial emit: the first runs after the
                // renderer has surely mounted its listener, the
                // second after AppKit's first-paint shuffle of the
                // standard window buttons has settled. The renderer
                // also pulls once via `traffic_light_metrics` IPC at
                // mount time as a belt-and-suspenders for the case
                // where both emits beat the listener.
                let win_for_boot = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    traffic_light::emit(&win_for_boot);
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    traffic_light::emit(&win_for_boot);
                });
            }

            // Application menu bar (macOS top-of-screen; Windows/Linux
            // per-window). Accessory-mode apps on macOS don't render a
            // menu bar regardless, but Setting it is still safe.
            if let Err(e) = app_menu::install(app.handle()) {
                tracing::warn!("app menu install failed: {e}");
            }

            let icon_bytes = include_bytes!("../icons/tray-iconTemplate@2x.png");
            let icon = Image::from_bytes(icon_bytes)?;

            TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .tooltip("Claudepot")
                // Left-click toggles the window directly; the menu is
                // right-click only. Standard pattern for productivity
                // menubar apps (Slack, Tailscale admin) — the menu's
                // job is quick switching, the icon's job is "open".
                // Linux: unsupported by Tauri, falls through as no-op.
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let visible = window.is_visible().unwrap_or(false);
                            let focused = window.is_focused().unwrap_or(false);
                            if visible && focused {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .on_menu_event(|app, event: MenuEvent| {
                    let id = event.id().as_ref();
                    if id.starts_with("app-menu:") {
                        app_menu::handle_menu_event(app, id);
                    } else {
                        tray::handle_menu_event(app, id);
                    }
                })
                .build(app)?;

            // Build initial tray menu from current accounts. Rebuild
            // is async (it peeks the UsageCache for the new Usage
            // submenu); spawn so setup returns immediately.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = tray::rebuild(&handle).await {
                    tracing::warn!("initial tray rebuild failed: {e}");
                }
            });

            // One-shot startup reconcile (B-2). Pre-B-2 the Tauri
            // `account_list` command opportunistically rewrote
            // `has_cli_credentials` and `has_desktop_profile` on every
            // poll, which raced two GUI sections that both polled.
            // Now: a single best-effort pass at startup catches the
            // common "user mutated state out-of-band" case, and the
            // explicit `accounts_reconcile` Tauri command covers the
            // user-driven path. Failure here never blocks startup.
            tauri::async_runtime::spawn_blocking(|| {
                let store = match crate::commands::open_store() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("startup reconcile: open_store failed: {e}");
                        return;
                    }
                };
                match claudepot_core::services::account_service::reconcile_all(&store) {
                    Ok(report) => {
                        if !report.cli_flips.is_empty()
                            || !report.desktop.flag_flips.is_empty()
                            || report.desktop.orphan_pointer_cleared
                        {
                            tracing::info!(
                                cli_flipped = report.cli_flips.len(),
                                desktop_flipped = report.desktop.flag_flips.len(),
                                orphan_pointer_cleared = report.desktop.orphan_pointer_cleared,
                                "startup reconcile applied"
                            );
                        }
                    }
                    Err(e) => tracing::warn!("startup reconcile failed: {e}"),
                }
            });

            // Listen for frontend requests to rebuild the tray menu
            let handle2 = app.handle().clone();
            app.listen("rebuild-tray-menu", move |_| {
                let handle = handle2.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = tray::rebuild(&handle).await {
                        tracing::warn!("rebuild-tray-menu failed: {e}");
                    }
                });
            });

            // Register the Tauri-side listener with the
            // `LiveActivityService`. This is the only place that
            // knows how to translate service events
            // (`on_aggregate`, `on_membership_changed`, `on_detail`)
            // into webview emits + tray rebuilds. The service
            // itself is framework-free.
            let live_state = app.state::<state::LiveSessionState>();
            let service = std::sync::Arc::clone(&live_state.service);
            let listener =
                live_activity_bridge::TauriSessionEventListener::new(app.handle().clone());
            tauri::async_runtime::spawn(async move {
                service.subscribe(listener).await;
            });

            // Usage-threshold watcher: every 5 min, polls /usage for the
            // CLI-active account and emits `usage-threshold-crossed`
            // events when utilization first crosses each configured
            // threshold per cycle. Pure detector + persistence live in
            // `claudepot_core::services::usage_alerts`; this is just
            // the orchestration. No-op when the threshold list is empty
            // or activity is disabled — guarded inside `run_tick`.
            //
            // The watcher reaches the shared `UsageCache` via
            // `app.state::<UsageCache>()` inside each tick, so it
            // consumes the same cache the rest of the app uses without
            // forcing an Arc<UsageCache> at the manage() site (which
            // would break every `State<'_, UsageCache>` consumer).
            usage_watcher::spawn(app.handle().clone());

            // Periodic writer for `~/.claudepot/usage-snapshot.json`.
            // Lets non-GUI processes (cron, CC bash subprocesses,
            // bots) read per-account utilization without keychain
            // access. Same 5-min cadence as `usage_watcher`; shares
            // the in-memory `UsageCache`. See
            // `src-tauri/src/usage_snapshot.rs`.
            usage_snapshot::spawn(app.handle().clone());

            // Background poller for CC CLI + Claude Desktop updates.
            // Probes upstream every `poll_interval_minutes` (default
            // 4 h), updates the tray badge, and runs the auto-install
            // pass when the user has opted in. See
            // `src-tauri/src/updates_watcher.rs`.
            updates_watcher::spawn(app.handle().clone());

            // Background poller for `status.claude.com`. 5 min cadence,
            // gated by the `service_status.poll_status_page` preference.
            // Surfaces transitions through the existing notification log.
            // See `src-tauri/src/service_status_watcher.rs` and
            // `dev-docs/network-status.md`.
            service_status_watcher::spawn(app.handle().clone());

            // Long-running fs-watcher for memory file changes. Watches
            // `~/.claude/` recursively; records diffs to
            // `~/.claudepot/memory_changes.db` and emits `memory:changed`
            // for the MemoryPane.
            memory_watch::spawn(app.handle().clone(), memory_log_for_watcher.clone());

            // Background `claude doctor` scrape on a 5 min cadence so
            // the tray Health row stays current when the window is
            // closed. See `cc_doctor_watcher.rs` for cadence rationale.
            cc_doctor_watcher::spawn(app.handle().clone());

            Ok(())
        })
        .manage(state::LoginState::default())
        .manage(state::DesktopOpState::default())
        .manage(state::CliOpState::default())
        .manage(state::DryRunState::new())
        .manage({
            // Build the live state with the service refactor's
            // pattern, then enable activity-cards classification on
            // the inner runtime when the cards index opened
            // successfully. The setup hook (which subscribes the
            // bridge listener) reads the same `LiveSessionState`
            // afterwards and sees both the service AND the wired
            // activity index.
            let svc = claudepot_core::services::live_activity_service::LiveActivityService::new();
            if let Some(idx) = cards_index.as_ref() {
                svc.enable_activity(std::sync::Arc::clone(idx));
            }
            state::LiveSessionState::new(svc)
        })
        .manage(ops::RunningOps::new())
        .manage(commands::cc_doctor::CcDoctorState::new())
        .manage(state::TrayHealthState::default())
        .manage(state::TrayAlertState::default())
        .manage(state::UpdatesAlertState::default())
        .manage(preferences::PreferencesState::new(prefs))
        .manage(claudepot_core::updates::UpdateStateMutex::new(
            updates_state,
        ))
        // Single shared gate: serializes background poller, manual
        // check, and manual install across the whole process. See
        // `updates_watcher::tick` and `commands_updates`.
        .manage(std::sync::Arc::new(
            claudepot_core::updates::PollerGate::default(),
        ))
        // D-1: shared config-scan cache + commit race arbiter. Owns
        // the latest scanned `ConfigTree`, hands out generation tokens,
        // and arbitrates between concurrent writers (`config_scan`
        // command + watcher seed/keepalive). Was `ConfigTreeState` in
        // the IPC layer — service moved to `claudepot-core` so the
        // commit policy is testable without Tauri.
        .manage(claudepot_core::config_view::ConfigScanService::new())
        .manage(commands::config::SearchRegistry::default())
        .manage(config_watch::ConfigWatchState::default())
        .manage(claudepot_core::services::usage_cache::UsageCache::new())
        // D-3: process-wide pricing cache + singleflight refresh.
        // Replaces the old `static REFRESH_IN_FLIGHT: AtomicBool` in
        // `commands_pricing.rs`, and now correctly singleflights the
        // `pricing_refresh` button-mash path too.
        .manage(claudepot_core::pricing::PricingCacheService::new())
        .manage(notification_log_state)
        .manage(commands::service_status::ServiceStatusState::new())
        .manage(rotation_orchestrator)
        // Per-project PR detection cache + tick. Zero overhead until
        // the orchestrator's `tick_all` is called from
        // `usage_snapshot::run_tick`; `project_list` reads the cache
        // synchronously to decorate `ProjectInfoDto.pr`. Shared as
        // an `Arc<PrOrchestrator>` so the tick and the read path see
        // the same cache instance.
        .manage(std::sync::Arc::new(pr_orchestrator::PrOrchestrator::new()));

    // Conditionally publish the cards index — `None` means open
    // failed at startup, in which case the cards-* commands return
    // `Tauri State unavailable` errors that the JS side surfaces as
    // a "cards index not ready" toast. The live-strip surface and
    // every other command keeps working.
    if let Some(idx) = cards_index {
        builder = builder.manage(commands::activity_cards::ActivityCardsState { index: idx });
    }

    // Shared SessionIndex for the Shared Memory commands. Always
    // managed (even when `None`); the wrapper carries the Option so
    // commands return a graceful error rather than panicking on a
    // missing state.
    builder = builder.manage(commands::shared_memory::SharedMemoryIndex(
        shared_memory_index,
    ));

    // Memory change-log state. Always managed — `memory_log` is now
    // unconditionally `Arc<MemoryLog>` per audit 2026-05 #2.
    builder = builder.manage(commands::memory::MemoryLogState::new(memory_log.clone()));

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }

    builder
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::updater_supported,
            commands::quit_now,
            commands::tray_set_alert_count,
            traffic_light::traffic_light_metrics,
            commands::cli::sync_from_current_cc,
            commands::unlock_keychain,
            commands::reveal_in_finder,
            commands::account_list,
            commands::account_list_basic,
            commands::accounts_reconcile,
            commands::cli::cli_use,
            commands::cli::cli_is_cc_running,
            commands::desktop::desktop_use,
            commands::desktop::current_desktop_identity,
            commands::desktop::verified_desktop_identity,
            commands::desktop::desktop_adopt,
            commands::desktop::desktop_clear,
            commands::desktop::sync_from_current_desktop,
            commands::desktop::desktop_launch,
            commands::account::account_add_from_current,
            commands::account::account_register_from_browser,
            commands::account::account_register_from_browser_start,
            commands::account::account_login,
            commands::account::account_login_start,
            commands::account::account_login_status,
            commands::account::account_login_cancel,
            commands::account::account_remove,
            commands::account::fetch_all_usage,
            commands::account::refresh_usage_for,
            commands::account::verify_all_accounts,
            commands::account::verify_all_accounts_start,
            commands::account::verify_all_accounts_status,
            commands::account::verify_account,
            commands::cli::current_cc_identity,
            commands::project::project_list,
            commands::project::project_show,
            commands::project::project_move_dry_run,
            commands::project::project_clean_preview,
            commands::project::project_clean_start,
            commands::project::project_clean_status,
            commands::project::project_remove_preview,
            commands::project::project_remove_preview_basic,
            commands::project::project_remove_preview_extras,
            commands::project::project_remove_execute,
            commands::project::project_trash_list,
            commands::project::project_trash_restore,
            commands::project::project_trash_empty,
            commands::project::repair_list,
            commands::project::repair_pending_count,
            commands::repair::repair_resume_start,
            commands::repair::repair_rollback_start,
            commands::repair::repair_abandon,
            commands::repair::repair_break_lock,
            commands::repair::repair_gc,
            commands::repair::repair_preview_abandoned,
            commands::repair::repair_cleanup_abandoned,
            commands::repair::running_ops_list,
            commands::repair::project_move_start,
            commands::repair::project_move_status,
            commands::project::repair_status_summary,
            commands::session_move::session_list_orphans,
            commands::session_move::session_move,
            commands::session_move::session_move_start,
            commands::session_move::session_move_status,
            commands::session_move::session_adopt_orphan,
            commands::session_move::session_discard_orphan,
            commands::session_index::session_list_all,
            commands::session_index::session_read,
            commands::session_index::session_read_path,
            commands::session_index::session_index_rebuild,
            commands::session_index::session_chunks,
            commands::session_index::session_context_attribution,
            commands::session_index::session_export_to_file,
            commands::session_index::session_search,
            commands::session_index::session_worktree_groups,
            commands::artifact_usage::artifact_usage_for,
            commands::artifact_usage::artifact_usage_batch,
            commands::artifact_usage::artifact_usage_top,
            commands::artifact_lifecycle::artifact_classify_path,
            commands::artifact_lifecycle::artifact_disable,
            commands::artifact_lifecycle::artifact_enable,
            commands::artifact_lifecycle::artifact_list_disabled,
            commands::artifact_lifecycle::artifact_trash,
            commands::artifact_lifecycle::artifact_list_trash,
            commands::artifact_lifecycle::artifact_restore_from_trash,
            commands::artifact_lifecycle::artifact_recover_trash,
            commands::artifact_lifecycle::artifact_forget_trash,
            commands::artifact_lifecycle::artifact_purge_trash,
            commands::artifact_lifecycle::artifact_disabled_preview,
            commands::protected::protected_paths_list,
            commands::protected::protected_paths_add,
            commands::protected::protected_paths_remove,
            commands::protected::protected_paths_reset,
            commands::preferences::preferences_get,
            commands::preferences::preferences_category_prefs_get,
            commands::preferences::preferences_category_pref_set,
            commands::preferences::preferences_set_hide_dock_icon,
            commands::preferences::preferences_set_show_window_on_startup,
            commands::keys::key_api_list,
            commands::keys::key_api_add,
            commands::keys::key_api_remove,
            commands::keys::key_api_rename,
            commands::keys::key_api_copy,
            commands::keys::key_api_probe,
            commands::keys::key_oauth_list,
            commands::keys::key_oauth_add,
            commands::keys::key_oauth_remove,
            commands::keys::key_oauth_rename,
            commands::keys::key_oauth_copy,
            commands::keys::key_oauth_copy_shell,
            commands::keys::key_oauth_usage_cached,
            commands::preferences::preferences_set_activity,
            commands::preferences::preferences_set_notifications,
            commands::preferences::preferences_set_service_status,
            commands::service_status::service_status_summary_get,
            commands::service_status::service_status_probe_now,
            commands::service_status::service_status_latency_get,
            commands::service_status::network_first_run_check,
            commands::activity::session_live_start,
            commands::activity::session_live_stop,
            commands::activity::session_live_snapshot,
            commands::activity::session_live_session_snapshot,
            commands::activity::session_live_subscribe,
            commands::activity::session_live_unsubscribe,
            commands::activity::activity_trends,
            commands::activity_cards::cards_recent,
            commands::activity_cards::cards_count_new_since,
            commands::activity_cards::cards_set_last_seen,
            commands::activity_cards::cards_navigate,
            commands::activity_cards::cards_body,
            commands::activity_cards::cards_reindex,
            commands::session_prune::session_prune_plan,
            commands::session_prune::session_prune_start,
            commands::session_prune::session_slim_plan,
            commands::session_prune::session_slim_start,
            commands::session_prune::session_slim_plan_all,
            commands::session_prune::session_slim_start_all,
            commands::session_prune::session_trash_list,
            commands::session_prune::session_trash_restore,
            commands::session_prune::session_trash_empty,
            commands::session_share::session_export_preview,
            commands::session_share::session_share_gist_start,
            commands::session_share::settings_github_token_get,
            commands::session_share::settings_github_token_set,
            commands::session_share::settings_github_token_clear,
            commands::config::config_scan,
            commands::config::config_preview,
            commands::config::config_list_editors,
            commands::config::config_get_editor_defaults,
            commands::config::config_set_editor_default,
            commands::config::config_open_in_editor_path,
            commands::config::config_search_start,
            commands::config::config_search_cancel,
            commands::config::config_effective_settings,
            commands::config::config_effective_mcp,
            commands::pricing::pricing_get,
            commands::pricing::pricing_refresh,
            commands::usage_local::local_usage_aggregate,
            commands::usage_local::pricing_tier_get,
            commands::usage_local::pricing_tier_set,
            commands::usage_local::top_costly_prompts,
            commands::memory_health::memory_health_get,
            commands::cc_tips::cc_tips_list,
            commands::cc_tips::cc_tips_refresh,
            commands::cc_tips::cc_tips_record_view,
            commands::memory::memory_list_for_project,
            commands::memory::memory_read_file,
            commands::memory::memory_change_log,
            commands::memory::auto_memory_state,
            commands::memory::auto_memory_state_global,
            commands::memory::auto_memory_set,
            config_watch::config_watch_start,
            config_watch::config_watch_stop,
            commands::migrate::migrate_inspect,
            commands::migrate::migrate_export,
            commands::migrate::migrate_import,
            commands::migrate::migrate_undo,
            commands::routes::routes_list,
            commands::routes::routes_get,
            commands::routes::routes_settings_get,
            commands::routes::routes_settings_set,
            commands::routes::routes_add,
            commands::routes::routes_edit,
            commands::routes::routes_remove,
            commands::routes::routes_use_cli,
            commands::routes::routes_unuse_cli,
            commands::routes::routes_path_status,
            commands::routes::routes_add_to_path,
            commands::routes::routes_use_desktop,
            commands::routes::routes_unuse_desktop,
            commands::routes::routes_derive_slug,
            commands::routes::routes_validate_wrapper_name,
            commands::routes::routes_zero_secret,
            commands::routes::routes_desktop_running,
            commands::routes::routes_desktop_restart,
            commands::automations::automations_list,
            commands::automations::automations_get,
            commands::automations::automations_add,
            commands::automations::automations_update,
            commands::automations::automations_remove,
            commands::automations::automations_set_enabled,
            commands::automations::automations_run_now_start,
            commands::automations::automations_runs_list,
            commands::automations::automations_run_get,
            commands::automations::automations_validate_name,
            commands::automations::automations_validate_cron,
            commands::automations::automations_scheduler_capabilities,
            commands::automations::automations_dry_run_artifact,
            commands::automations::automations_open_artifact_dir,
            commands::automations::automations_linger_status,
            commands::automations::automations_linger_enable,
            commands::templates::templates_list,
            commands::templates::templates_get,
            commands::templates::templates_sample_report,
            commands::templates::templates_capable_routes,
            commands::templates::templates_install,
            commands::templates::templates_read_report,
            commands::templates::templates_pending_changes,
            commands::templates::templates_apply_pending,
            commands::templates::routing_rules_get,
            commands::templates::routing_rules_set,
            commands::templates::routing_rules_evaluate_for,
            commands::notification::notification_activate_host_for_session,
            commands::notification::notification_log_append,
            commands::notification::notification_log_append_routed,
            commands::notification::notification_log_mark_delivered,
            commands::notification::notification_log_list,
            commands::notification::notification_log_mark_all_read,
            commands::notification::notification_log_clear,
            commands::notification::notification_log_unread_count,
            commands::notification::notification_categories_metadata,
            commands::cc_doctor::cc_doctor_snapshot,
            commands::cc_doctor::cc_doctor_open_parse_failures_log,
            commands::updates::updates_status_get,
            commands::updates::updates_check_now,
            commands::updates::updates_cli_install,
            commands::updates::updates_desktop_install,
            commands::updates::updates_settings_get,
            commands::updates::updates_settings_set,
            commands::updates::updates_channel_set,
            commands::updates::updates_minimum_version_set,
            commands::rotation::rotation_rules_get,
            commands::rotation::rotation_rules_set,
            commands::rotation::rotation_rule_validate,
            commands::rotation::rotation_dry_run,
            commands::rotation::rotation_audit_get,
            commands::rotation::rotation_pending_list,
            commands::rotation::rotation_apply_pending,
            commands::rotation::rotation_dismiss_pending,
            commands::permission::permission_list,
            commands::permission::permission_get,
            commands::permission::permission_grant,
            commands::permission::permission_revert,
            commands::permission::permission_extend,
            commands::env_secret::env_vault_list,
            commands::env_secret::env_vault_add,
            commands::env_secret::env_vault_update,
            commands::env_secret::env_vault_delete,
            commands::env_secret::env_vault_copy,
            commands::env_secret::env_file_list,
            commands::env_secret::env_file_set,
            commands::env_secret::env_file_comment,
            commands::env_secret::env_file_uncomment,
            commands::env_secret::env_file_delete,
            commands::env_secret::env_file_copy_value,
            commands::env_secret::env_file_inject,
            // ─── shared_memory (WI-007, WI-009) ───────────────
            commands::shared_memory::shared_memory_search,
            commands::shared_memory::shared_memory_read_locator,
            commands::shared_memory::shared_memory_list_memories,
            commands::shared_memory::shared_memory_create_memory,
            commands::shared_memory::shared_memory_archive_memory,
            commands::shared_memory::shared_memory_list_decisions,
            commands::shared_memory::shared_memory_log_decision,
            commands::shared_memory::shared_memory_archive_decision,
            commands::shared_memory::shared_memory_list_sessions,
            commands::shared_memory::shared_memory_list_projects,
            commands::shared_memory::shared_memory_install_snippet,
            commands::shared_memory::shared_memory_snippet_body,
            commands::shared_memory::shared_memory_mcp_health,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: tauri application failed to start: {e}");
            std::process::exit(1);
        });
}
