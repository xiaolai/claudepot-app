mod commands;
mod dto;
mod ops;
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

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::{
                image::Image,
                menu::MenuEvent,
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
                Listener, Manager,
            };

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
                    tray::handle_menu_event(app, event.id().as_ref());
                })
                .build(app)?;

            // Build initial tray menu from current accounts
            let handle = app.handle().clone();
            let _ = tray::rebuild(&handle);

            // Listen for frontend requests to rebuild the tray menu
            let handle2 = app.handle().clone();
            app.listen("rebuild-tray-menu", move |_| {
                let _ = tray::rebuild(&handle2);
            });

            Ok(())
        })
        .manage(state::LoginState::default())
        .manage(state::DryRunRegistry::default())
        .manage(ops::RunningOps::new())
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
            commands::cli_clear,
            commands::desktop_use,
            commands::account_add_from_current,
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
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: tauri application failed to start: {e}");
            std::process::exit(1);
        });
}
