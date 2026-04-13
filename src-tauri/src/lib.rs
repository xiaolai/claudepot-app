mod commands;
mod dto;

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

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::account_list,
            commands::cli_use,
            commands::cli_clear,
            commands::desktop_use,
            commands::account_add_from_current,
            commands::account_login,
            commands::account_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
