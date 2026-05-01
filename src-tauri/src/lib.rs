mod app_menu;
mod commands;
mod commands_account;
mod commands_activity;
mod commands_activity_cards;
mod commands_artifact_lifecycle;
mod commands_artifact_usage;
mod commands_automations;
mod commands_cli;
mod commands_config;
mod commands_config_types;
mod commands_desktop;
mod commands_keys;
mod commands_migrate;
mod commands_notification;
mod commands_preferences;
mod commands_pricing;
mod commands_project;
mod commands_protected;
mod commands_repair;
mod commands_routes;
mod commands_session_index;
mod commands_session_move;
mod commands_session_prune;
mod commands_session_share;
mod commands_usage_local;
mod config_dto;
mod config_watch;
mod config_watch_types;
mod dto;
mod dto_account;
mod dto_activity;
mod dto_activity_cards;
mod dto_artifact_lifecycle;
mod dto_artifact_usage;
mod dto_automations;
mod dto_desktop;
mod dto_keys;
mod dto_migrate;
mod dto_project;
mod dto_project_repair;
mod dto_routes;
mod dto_session;
mod dto_session_debug;
mod dto_session_move;
mod dto_session_prune;
mod dto_usage;
mod live_activity_bridge;
mod ops;
mod preferences;
mod state;
mod tray;
mod tray_icons;
mod tray_menu;
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
    // `hide_dock` is only consumed inside `#[cfg(target_os = "macos")]`
    // (set_activation_policy is a no-op everywhere else), so binding it
    // unconditionally would emit `unused_variable` on Linux + Windows.
    // Gate the bind to keep the production build clean while still
    // honouring the cold-load order — `prefs` itself is read on every
    // platform; only the macOS-specific extraction is gated.
    #[cfg(target_os = "macos")]
    let hide_dock = prefs.hide_dock_icon;
    let show_window_on_startup = prefs.show_window_on_startup;

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

    // (Live state is built inside the `.manage` chain below — the
    // service-refactor pattern owns the runtime, so we hand the
    // cards index into the service's `enable_activity` accessor at
    // construction time.)

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
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Err(e) = win_for_handler.hide() {
                            tracing::warn!("close-button: hide failed: {e}");
                        }
                    }
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
        .manage(state::TrayAlertState::default())
        .manage(preferences::PreferencesState::new(prefs))
        // D-1: shared config-scan cache + commit race arbiter. Owns
        // the latest scanned `ConfigTree`, hands out generation tokens,
        // and arbitrates between concurrent writers (`config_scan`
        // command + watcher seed/keepalive). Was `ConfigTreeState` in
        // the IPC layer — service moved to `claudepot-core` so the
        // commit policy is testable without Tauri.
        .manage(claudepot_core::config_view::ConfigScanService::new())
        .manage(commands_config::SearchRegistry::default())
        .manage(config_watch::ConfigWatchState::default())
        .manage(claudepot_core::services::usage_cache::UsageCache::new())
        // D-3: process-wide pricing cache + singleflight refresh.
        // Replaces the old `static REFRESH_IN_FLIGHT: AtomicBool` in
        // `commands_pricing.rs`, and now correctly singleflights the
        // `pricing_refresh` button-mash path too.
        .manage(claudepot_core::pricing::PricingCacheService::new());

    // Conditionally publish the cards index — `None` means open
    // failed at startup, in which case the cards-* commands return
    // `Tauri State unavailable` errors that the JS side surfaces as
    // a "cards index not ready" toast. The live-strip surface and
    // every other command keeps working.
    if let Some(idx) = cards_index {
        builder = builder.manage(commands_activity_cards::ActivityCardsState { index: idx });
    }

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
            commands_cli::sync_from_current_cc,
            commands::unlock_keychain,
            commands::reveal_in_finder,
            commands::account_list,
            commands::account_list_basic,
            commands::accounts_reconcile,
            commands_cli::cli_use,
            commands_cli::cli_is_cc_running,
            commands_desktop::desktop_use,
            commands_desktop::current_desktop_identity,
            commands_desktop::verified_desktop_identity,
            commands_desktop::desktop_adopt,
            commands_desktop::desktop_clear,
            commands_desktop::sync_from_current_desktop,
            commands_desktop::desktop_launch,
            commands_account::account_add_from_current,
            commands_account::account_register_from_browser,
            commands_account::account_register_from_browser_start,
            commands_account::account_login,
            commands_account::account_login_start,
            commands_account::account_login_status,
            commands_account::account_login_cancel,
            commands_account::account_remove,
            commands_account::fetch_all_usage,
            commands_account::refresh_usage_for,
            commands_account::verify_all_accounts,
            commands_account::verify_all_accounts_start,
            commands_account::verify_all_accounts_status,
            commands_account::verify_account,
            commands_cli::current_cc_identity,
            commands_project::project_list,
            commands_project::project_show,
            commands_project::project_move_dry_run,
            commands_project::project_clean_preview,
            commands_project::project_clean_start,
            commands_project::project_clean_status,
            commands_project::project_remove_preview,
            commands_project::project_remove_preview_basic,
            commands_project::project_remove_preview_extras,
            commands_project::project_remove_execute,
            commands_project::project_trash_list,
            commands_project::project_trash_restore,
            commands_project::project_trash_empty,
            commands_project::repair_list,
            commands_project::repair_pending_count,
            commands_repair::repair_resume_start,
            commands_repair::repair_rollback_start,
            commands_repair::repair_abandon,
            commands_repair::repair_break_lock,
            commands_repair::repair_gc,
            commands_repair::repair_preview_abandoned,
            commands_repair::repair_cleanup_abandoned,
            commands_repair::running_ops_list,
            commands_repair::project_move_start,
            commands_repair::project_move_status,
            commands_project::repair_status_summary,
            commands_session_move::session_list_orphans,
            commands_session_move::session_move,
            commands_session_move::session_move_start,
            commands_session_move::session_move_status,
            commands_session_move::session_adopt_orphan,
            commands_session_move::session_discard_orphan,
            commands_session_index::session_list_all,
            commands_session_index::session_read,
            commands_session_index::session_read_path,
            commands_session_index::session_index_rebuild,
            commands_session_index::session_chunks,
            commands_session_index::session_context_attribution,
            commands_session_index::session_export_to_file,
            commands_session_index::session_search,
            commands_session_index::session_worktree_groups,
            commands_artifact_usage::artifact_usage_for,
            commands_artifact_usage::artifact_usage_batch,
            commands_artifact_usage::artifact_usage_top,
            commands_artifact_lifecycle::artifact_classify_path,
            commands_artifact_lifecycle::artifact_disable,
            commands_artifact_lifecycle::artifact_enable,
            commands_artifact_lifecycle::artifact_list_disabled,
            commands_artifact_lifecycle::artifact_trash,
            commands_artifact_lifecycle::artifact_list_trash,
            commands_artifact_lifecycle::artifact_restore_from_trash,
            commands_artifact_lifecycle::artifact_recover_trash,
            commands_artifact_lifecycle::artifact_forget_trash,
            commands_artifact_lifecycle::artifact_purge_trash,
            commands_artifact_lifecycle::artifact_disabled_preview,
            commands_protected::protected_paths_list,
            commands_protected::protected_paths_add,
            commands_protected::protected_paths_remove,
            commands_protected::protected_paths_reset,
            commands_preferences::preferences_get,
            commands_preferences::preferences_set_hide_dock_icon,
            commands_preferences::preferences_set_show_window_on_startup,
            commands_keys::key_api_list,
            commands_keys::key_api_add,
            commands_keys::key_api_remove,
            commands_keys::key_api_rename,
            commands_keys::key_api_copy,
            commands_keys::key_api_probe,
            commands_keys::key_oauth_list,
            commands_keys::key_oauth_add,
            commands_keys::key_oauth_remove,
            commands_keys::key_oauth_rename,
            commands_keys::key_oauth_copy,
            commands_keys::key_oauth_copy_shell,
            commands_keys::key_oauth_usage_cached,
            commands_preferences::preferences_set_activity,
            commands_preferences::preferences_set_notifications,
            commands_activity::session_live_start,
            commands_activity::session_live_stop,
            commands_activity::session_live_snapshot,
            commands_activity::session_live_session_snapshot,
            commands_activity::session_live_subscribe,
            commands_activity::session_live_unsubscribe,
            commands_activity::activity_trends,
            commands_activity_cards::cards_recent,
            commands_activity_cards::cards_count_new_since,
            commands_activity_cards::cards_set_last_seen,
            commands_activity_cards::cards_navigate,
            commands_activity_cards::cards_body,
            commands_activity_cards::cards_reindex,
            commands_session_prune::session_prune_plan,
            commands_session_prune::session_prune_start,
            commands_session_prune::session_slim_plan,
            commands_session_prune::session_slim_start,
            commands_session_prune::session_slim_plan_all,
            commands_session_prune::session_slim_start_all,
            commands_session_prune::session_trash_list,
            commands_session_prune::session_trash_restore,
            commands_session_prune::session_trash_empty,
            commands_session_share::session_export_preview,
            commands_session_share::session_share_gist_start,
            commands_session_share::settings_github_token_get,
            commands_session_share::settings_github_token_set,
            commands_session_share::settings_github_token_clear,
            commands_config::config_scan,
            commands_config::config_preview,
            commands_config::config_list_editors,
            commands_config::config_get_editor_defaults,
            commands_config::config_set_editor_default,
            commands_config::config_open_in_editor_path,
            commands_config::config_search_start,
            commands_config::config_search_cancel,
            commands_config::config_effective_settings,
            commands_config::config_effective_mcp,
            commands_pricing::pricing_get,
            commands_pricing::pricing_refresh,
            commands_usage_local::local_usage_aggregate,
            config_watch::config_watch_start,
            config_watch::config_watch_stop,
            commands_migrate::migrate_inspect,
            commands_migrate::migrate_export,
            commands_migrate::migrate_import,
            commands_migrate::migrate_undo,
            commands_routes::routes_list,
            commands_routes::routes_get,
            commands_routes::routes_settings_get,
            commands_routes::routes_settings_set,
            commands_routes::routes_add,
            commands_routes::routes_edit,
            commands_routes::routes_remove,
            commands_routes::routes_use_cli,
            commands_routes::routes_unuse_cli,
            commands_routes::routes_use_desktop,
            commands_routes::routes_unuse_desktop,
            commands_routes::routes_derive_slug,
            commands_routes::routes_validate_wrapper_name,
            commands_routes::routes_zero_secret,
            commands_routes::routes_desktop_running,
            commands_routes::routes_desktop_restart,
            commands_automations::automations_list,
            commands_automations::automations_get,
            commands_automations::automations_add,
            commands_automations::automations_update,
            commands_automations::automations_remove,
            commands_automations::automations_set_enabled,
            commands_automations::automations_run_now_start,
            commands_automations::automations_runs_list,
            commands_automations::automations_run_get,
            commands_automations::automations_validate_name,
            commands_automations::automations_validate_cron,
            commands_automations::automations_scheduler_capabilities,
            commands_automations::automations_dry_run_artifact,
            commands_automations::automations_open_artifact_dir,
            commands_automations::automations_linger_status,
            commands_automations::automations_linger_enable,
            commands_notification::notification_activate_host_for_session,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: tauri application failed to start: {e}");
            std::process::exit(1);
        });
}
