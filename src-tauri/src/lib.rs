mod commands;
mod dto;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::account_list,
            commands::cli_use,
            commands::cli_clear,
            commands::desktop_use,
            commands::account_add_from_current,
            commands::account_add_from_token,
            commands::account_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
