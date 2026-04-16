mod commands;
mod dto;
mod ops;
mod state;

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
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
                Manager,
            };

            // Menubar icon: monochrome template so macOS tints it for
            // light/dark menu bars. Ship the @2x (44×44) PNG baked with
            // 144 DPI metadata — NSImage reports its logical size as 22pt
            // (44px ÷ 144/72), giving native 44px on Retina without
            // upscaling, and a clean downsample on 1x displays. White
            // pixels would get tinted like the body, so the "window"
            // holes are genuinely transparent in the source PNG.
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
                .build(app)?;

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
            commands::account_list,
            commands::cli_use,
            commands::cli_clear,
            commands::desktop_use,
            commands::account_add_from_current,
            commands::account_login,
            commands::account_login_cancel,
            commands::account_remove,
            commands::fetch_all_usage,
            commands::verify_all_accounts,
            commands::current_cc_identity,
            commands::project_list,
            commands::project_show,
            commands::project_move_dry_run,
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
        .expect("error while running tauri application");
}
