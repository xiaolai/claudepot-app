mod app_menu;
mod commands;
mod commands_config;
mod commands_config_types;
mod commands_preferences;
mod commands_pricing;
mod commands_account;
mod commands_activity;
mod commands_cli;
mod commands_desktop;
mod commands_keys;
mod commands_project;
mod commands_protected;
mod commands_repair;
mod commands_session_index;
mod commands_session_move;
mod commands_session_prune;
mod commands_session_share;
mod config_dto;
mod config_watch;
mod config_watch_types;
mod dto;
mod dto_activity;
mod dto_desktop;
mod dto_keys;
mod dto_project;
mod dto_project_repair;
mod dto_session;
mod dto_session_debug;
mod dto_session_move;
mod dto_session_prune;
mod ops;
mod preferences;
mod state;
mod tray;
mod tray_icons;
mod tray_menu;

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
    let hide_dock = prefs.hide_dock_icon;

    // `mut` is only consumed by the debug-only plugin block below;
    // release builds don't touch it. Silence the release warning here.
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
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

            Ok(())
        })
        .manage(state::LoginState::default())
        .manage(state::DesktopOpState::default())
        .manage(state::DryRunRegistry::default())
        .manage(state::LiveSessionState::default())
        .manage(ops::RunningOps::new())
        .manage(preferences::PreferencesState::new(prefs))
        .manage(commands_config::ConfigTreeState::default())
        .manage(commands_config::SearchRegistry::default())
        .manage(config_watch::ConfigWatchState::default())
        .manage(claudepot_core::services::usage_cache::UsageCache::new());

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }

    builder.invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands_cli::sync_from_current_cc,
            commands::unlock_keychain,
            commands::reveal_in_finder,
            commands::account_list,
            commands::account_list_basic,
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
            commands_account::account_login,
            commands_account::account_login_cancel,
            commands_account::account_remove,
            commands_account::fetch_all_usage,
            commands_account::refresh_usage_for,
            commands_account::verify_all_accounts,
            commands_account::verify_account,
            commands_cli::current_cc_identity,
            commands_project::project_list,
            commands_project::project_show,
            commands_project::project_move_dry_run,
            commands_project::project_clean_preview,
            commands_project::project_clean_start,
            commands_project::project_clean_status,
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
            commands_protected::protected_paths_list,
            commands_protected::protected_paths_add,
            commands_protected::protected_paths_remove,
            commands_protected::protected_paths_reset,
            commands_preferences::preferences_get,
            commands_preferences::preferences_set_hide_dock_icon,
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
            config_watch::config_watch_start,
            config_watch::config_watch_stop,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: tauri application failed to start: {e}");
            std::process::exit(1);
        });
}
