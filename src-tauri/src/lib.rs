mod app_menu;
mod commands;
mod dto;
mod ops;
mod preferences;
mod state;
mod tray;

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
                let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
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
        .manage(state::DryRunRegistry::default())
        .manage(state::LiveSessionState::default())
        .manage(ops::RunningOps::new())
        .manage(preferences::PreferencesState::new(prefs))
        .manage(claudepot_core::services::usage_cache::UsageCache::new());

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }

    builder.invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::sync_from_current_cc,
            commands::unlock_keychain,
            commands::reveal_in_finder,
            commands::account_list,
            commands::cli_use,
            commands::cli_is_cc_running,
            commands::cli_clear,
            commands::desktop_use,
            commands::account_add_from_current,
            commands::account_register_from_browser,
            commands::account_login,
            commands::account_login_cancel,
            commands::account_remove,
            commands::fetch_all_usage,
            commands::refresh_usage_for,
            commands::verify_all_accounts,
            commands::verify_account,
            commands::current_cc_identity,
            commands::project_list,
            commands::project_show,
            commands::project_move_dry_run,
            commands::project_clean_preview,
            commands::project_clean_start,
            commands::project_clean_status,
            commands::repair_list,
            commands::repair_pending_count,
            commands::repair_resume_start,
            commands::repair_rollback_start,
            commands::repair_abandon,
            commands::repair_break_lock,
            commands::repair_gc,
            commands::running_ops_list,
            commands::project_move_start,
            commands::project_move_status,
            commands::repair_status_summary,
            commands::session_list_orphans,
            commands::session_move,
            commands::session_adopt_orphan,
            commands::session_list_all,
            commands::session_read,
            commands::session_read_path,
            commands::session_index_rebuild,
            commands::session_chunks,
            commands::session_context_attribution,
            commands::session_export_to_file,
            commands::session_search,
            commands::session_worktree_groups,
            commands::protected_paths_list,
            commands::protected_paths_add,
            commands::protected_paths_remove,
            commands::protected_paths_reset,
            commands::preferences_get,
            commands::preferences_set_hide_dock_icon,
            commands::key_api_list,
            commands::key_api_add,
            commands::key_api_remove,
            commands::key_api_copy,
            commands::key_oauth_list,
            commands::key_oauth_add,
            commands::key_oauth_remove,
            commands::key_oauth_copy,
            commands::key_oauth_probe,
            commands::key_oauth_usage,
            commands::preferences_set_activity,
            commands::preferences_set_notifications,
            commands::session_live_start,
            commands::session_live_stop,
            commands::session_live_snapshot,
            commands::session_live_session_snapshot,
            commands::session_live_subscribe,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: tauri application failed to start: {e}");
            std::process::exit(1);
        });
}
