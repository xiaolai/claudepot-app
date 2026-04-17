//! System tray menu — quick-switch accounts without opening the window.
//!
//! The menu is rebuilt whenever the frontend emits `rebuild-tray-menu` or
//! when the `rebuild_tray_menu` command is called directly. Each non-active
//! account becomes a clickable item; the active account gets a checkmark.

use crate::dto::AccountSummary;
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager};

/// Build and set the tray menu from the current account state.
/// Called from the Tauri command and from event listeners.
pub fn rebuild(app: &AppHandle) -> Result<(), String> {
    let store = crate::commands::open_store()?;
    let accounts = store.list().map_err(|e| format!("list: {e}"))?;
    let summaries: Vec<AccountSummary> = accounts.iter().map(AccountSummary::from).collect();

    let cli_active = summaries.iter().find(|a| a.is_cli_active);

    let mut builder = MenuBuilder::new(app);

    // Account items
    for s in &summaries {
        let prefix = if s.is_cli_active { "✓ " } else { "   " };
        let suffix = if !s.credentials_healthy {
            " ⚠"
        } else if s.is_desktop_active {
            " 🖥"
        } else {
            ""
        };
        let label = format!("{}{}{}", prefix, s.email, suffix);
        let item = MenuItemBuilder::with_id(
            format!("tray-switch-{}", s.uuid),
            &label,
        )
        .enabled(!s.is_cli_active && s.credentials_healthy)
        .build(app)
        .map_err(|e| format!("menu item: {e}"))?;
        builder = builder.item(&item);
    }

    // Separator + utility items
    let sep = PredefinedMenuItem::separator(app).map_err(|e| format!("sep: {e}"))?;
    builder = builder.item(&sep);

    let show_item = MenuItemBuilder::with_id("tray-show", "Show Claudepot")
        .accelerator("CmdOrCtrl+1")
        .build(app)
        .map_err(|e| format!("show: {e}"))?;
    builder = builder.item(&show_item);

    let refresh_item = MenuItemBuilder::with_id("tray-refresh", "Refresh")
        .accelerator("CmdOrCtrl+R")
        .build(app)
        .map_err(|e| format!("refresh: {e}"))?;
    builder = builder.item(&refresh_item);

    let quit_item = MenuItemBuilder::with_id("tray-quit", "Quit Claudepot")
        .accelerator("CmdOrCtrl+Q")
        .build(app)
        .map_err(|e| format!("quit: {e}"))?;
    builder = builder.item(&quit_item);

    let menu = builder.build().map_err(|e| format!("menu build: {e}"))?;

    // Set the menu on the tray icon
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu)).map_err(|e| format!("set menu: {e}"))?;
        // Update tooltip with active account
        let tooltip = match cli_active {
            Some(a) => format!("Claudepot — {}", a.email),
            None => "Claudepot".to_string(),
        };
        tray.set_tooltip(Some(&tooltip)).map_err(|e| format!("tooltip: {e}"))?;
    }

    Ok(())
}

/// Handle a tray menu item click. Returns true if the event was handled.
pub fn handle_menu_event(app: &AppHandle, id: &str) {
    if let Some(uuid_str) = id.strip_prefix("tray-switch-") {
        // Find the email for this UUID and switch CLI
        let email = match find_email_for_uuid(uuid_str) {
            Some(e) => e,
            None => return,
        };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match crate::commands::cli_use(email, None).await {
                Ok(()) => {
                    let _ = rebuild(&app);
                    // Emit event so the frontend refreshes
                    let _ = app.emit("tray-cli-switched", ());
                }
                Err(e) => {
                    tracing::warn!("tray cli_use failed: {e}");
                }
            }
        });
    } else if id == "tray-show" {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    } else if id == "tray-refresh" {
        let _ = rebuild(app);
        let _ = app.emit("tray-refresh-requested", ());
    } else if id == "tray-quit" {
        app.exit(0);
    }
}

fn find_email_for_uuid(uuid_str: &str) -> Option<String> {
    let store = crate::commands::open_store().ok()?;
    let accounts = store.list().ok()?;
    accounts
        .iter()
        .find(|a| a.uuid.to_string() == uuid_str)
        .map(|a| a.email.clone())
}
