mod commands;
mod dto;
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
        .manage(state::LoginState::default())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
