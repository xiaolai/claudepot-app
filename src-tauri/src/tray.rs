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

    // Account items — use AppKit's native checked-state so alignment
    // is handled by the menu system instead of fake leading spaces
    // (which don't align across proportional fonts of varying email
    // lengths).
    use tauri::menu::CheckMenuItemBuilder;
    for s in &summaries {
        let suffix = if !s.credentials_healthy {
            " ⚠"
        } else if s.is_desktop_active {
            " 🖥"
        } else {
            ""
        };
        let label = format!("{}{}", s.email, suffix);
        let item = CheckMenuItemBuilder::with_id(
            format!("tray-switch-{}", s.uuid),
            &label,
        )
        .checked(s.is_cli_active)
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
        let email = match find_email_for_uuid(uuid_str) {
            Some(e) => e,
            None => {
                tracing::warn!("tray: no account found for UUID {uuid_str}");
                return;
            }
        };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match crate::commands::cli_use(email, None).await {
                Ok(()) => {
                    if let Err(e) = rebuild(&app) {
                        tracing::warn!("tray rebuild after switch failed: {e}");
                    }
                    if let Err(e) = app.emit("tray-cli-switched", ()) {
                        tracing::warn!("tray emit tray-cli-switched failed: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("tray cli_use failed: {e}");
                    // Audit Low: emit a frontend event so the window
                    // (if open) can toast the failure instead of it
                    // disappearing into tracing logs. The frontend
                    // listens on `tray-cli-switch-failed` and shows
                    // an error toast; if the window is closed the
                    // event is dropped, which is the same behavior
                    // as before (tray stays silent).
                    let _ = app.emit("tray-cli-switch-failed", e.to_string());
                }
            }
        });
    } else if id == "tray-show" {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    } else if id == "tray-refresh" {
        if let Err(e) = rebuild(app) {
            tracing::warn!("tray rebuild on refresh failed: {e}");
        }
        if let Err(e) = app.emit("tray-refresh-requested", ()) {
            tracing::warn!("tray emit tray-refresh-requested failed: {e}");
        }
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
