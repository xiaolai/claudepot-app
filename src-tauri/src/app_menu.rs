//! macOS application menu bar — the top-of-screen menus shown while the
//! window is frontmost. Accessory-mode apps (hide_dock_icon) don't get
//! a menu bar at all, so this code only matters when the dock icon is
//! visible.
//!
//! The menu routes clicks to the webview via a single `app-menu`
//! event. The React side dispatches each command id to its existing
//! handler (section switch, theme toggle, etc.). This avoids
//! double-firing with in-app keyboard shortcuts: we deliberately do
//! NOT bind accelerators on View items (⌘1..⌘4 / ⌘R) here so the React
//! listeners stay authoritative. The only accelerators we keep are the
//! macOS-standard ⌘, and ⌘Q on the Claudepot menu.

use tauri::menu::{
    AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem,
    SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Build the app menu and set it on the given app handle.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // ---- Claudepot submenu --------------------------------------------------
    let about_md = AboutMetadataBuilder::new()
        .name(Some("Claudepot"))
        .version(Some(env!("CARGO_PKG_VERSION")))
        .copyright(Some("© HANDO K.K."))
        .website(Some("https://github.com/xiaolai/claudepot-app"))
        .website_label(Some("GitHub"))
        .build();

    let about = PredefinedMenuItem::about(app, Some("About Claudepot"), Some(about_md))
        .map_err(|e| format!("about: {e}"))?;

    let settings = MenuItemBuilder::with_id("app-menu:settings", "Settings…")
        .accelerator("Cmd+,")
        .build(app)
        .map_err(|e| format!("settings: {e}"))?;

    let services = PredefinedMenuItem::services(app, Some("Services"))
        .map_err(|e| format!("services: {e}"))?;
    let hide = PredefinedMenuItem::hide(app, Some("Hide Claudepot"))
        .map_err(|e| format!("hide: {e}"))?;
    let hide_others = PredefinedMenuItem::hide_others(app, Some("Hide Others"))
        .map_err(|e| format!("hide_others: {e}"))?;
    let show_all = PredefinedMenuItem::show_all(app, Some("Show All"))
        .map_err(|e| format!("show_all: {e}"))?;
    let quit = PredefinedMenuItem::quit(app, Some("Quit Claudepot"))
        .map_err(|e| format!("quit: {e}"))?;

    let claudepot = SubmenuBuilder::new(app, "Claudepot")
        .item(&about)
        .separator()
        .item(&settings)
        .separator()
        .item(&services)
        .separator()
        .item(&hide)
        .item(&hide_others)
        .item(&show_all)
        .separator()
        .item(&quit)
        .build()
        .map_err(|e| format!("claudepot submenu: {e}"))?;

    // ---- Account submenu (static; no dynamic rebuild needed) ---------------
    let login_browser = MenuItemBuilder::with_id(
        "app-menu:account:login-browser",
        "Sign in from browser…",
    )
    .build(app)
    .map_err(|e| format!("login-browser: {e}"))?;
    let sync_cc = MenuItemBuilder::with_id("app-menu:account:sync-cc", "Sync from current CC")
        .build(app)
        .map_err(|e| format!("sync-cc: {e}"))?;
    let verify_all = MenuItemBuilder::with_id("app-menu:account:verify-all", "Verify all")
        .build(app)
        .map_err(|e| format!("verify-all: {e}"))?;
    let manage = MenuItemBuilder::with_id("app-menu:nav:accounts:manage", "Manage accounts…")
        .build(app)
        .map_err(|e| format!("manage: {e}"))?;

    let account = SubmenuBuilder::new(app, "Account")
        .item(&login_browser)
        .item(&sync_cc)
        .item(&verify_all)
        .separator()
        .item(&manage)
        .build()
        .map_err(|e| format!("account submenu: {e}"))?;

    // ---- View submenu ------------------------------------------------------
    let nav_accounts = MenuItemBuilder::with_id("app-menu:nav:accounts", "Accounts")
        .build(app).map_err(|e| format!("nav-accounts: {e}"))?;
    let nav_projects = MenuItemBuilder::with_id("app-menu:nav:projects", "Projects")
        .build(app).map_err(|e| format!("nav-projects: {e}"))?;
    let nav_sessions = MenuItemBuilder::with_id("app-menu:nav:sessions", "Sessions")
        .build(app).map_err(|e| format!("nav-sessions: {e}"))?;
    let nav_settings = MenuItemBuilder::with_id("app-menu:nav:settings", "Settings")
        .build(app).map_err(|e| format!("nav-settings: {e}"))?;
    let toggle_theme = MenuItemBuilder::with_id("app-menu:view:toggle-theme", "Toggle theme")
        .build(app).map_err(|e| format!("toggle-theme: {e}"))?;
    let reload = MenuItemBuilder::with_id("app-menu:view:reload", "Refresh")
        .build(app).map_err(|e| format!("reload: {e}"))?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&nav_accounts)
        .item(&nav_projects)
        .item(&nav_sessions)
        .item(&nav_settings)
        .separator()
        .item(&toggle_theme)
        .item(&reload)
        .build()
        .map_err(|e| format!("view submenu: {e}"))?;

    // ---- Window submenu (standard macOS set) -------------------------------
    let minimize = PredefinedMenuItem::minimize(app, Some("Minimize"))
        .map_err(|e| format!("minimize: {e}"))?;
    let maximize = PredefinedMenuItem::maximize(app, Some("Zoom"))
        .map_err(|e| format!("maximize: {e}"))?;
    let close_window = PredefinedMenuItem::close_window(app, Some("Close Window"))
        .map_err(|e| format!("close_window: {e}"))?;

    let window = SubmenuBuilder::new(app, "Window")
        .item(&minimize)
        .item(&maximize)
        .separator()
        .item(&close_window)
        .build()
        .map_err(|e| format!("window submenu: {e}"))?;

    // ---- Help submenu ------------------------------------------------------
    let help_github = MenuItemBuilder::with_id("app-menu:help:github", "Open GitHub")
        .build(app).map_err(|e| format!("help-github: {e}"))?;
    let help_diag = MenuItemBuilder::with_id("app-menu:help:copy-diag", "Copy diagnostics")
        .build(app).map_err(|e| format!("help-diag: {e}"))?;
    let help_data = MenuItemBuilder::with_id("app-menu:help:reveal-data-dir", "Reveal data dir in Finder")
        .build(app).map_err(|e| format!("help-data: {e}"))?;

    let help = SubmenuBuilder::new(app, "Help")
        .item(&help_github)
        .item(&help_diag)
        .item(&help_data)
        .build()
        .map_err(|e| format!("help submenu: {e}"))?;

    let menu = MenuBuilder::new(app)
        .items(&[&claudepot, &account, &view, &window, &help])
        .build()
        .map_err(|e| format!("root menu: {e}"))?;

    app.set_menu(menu).map_err(|e| format!("set_menu: {e}"))?;
    Ok(())
}

/// Route a menu event. Settings gets opened directly (section switch
/// + window focus); everything else forwards to the webview via a
/// single `app-menu` event, with the menu id as payload.
pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    // "app-menu:settings" is identical to "app-menu:nav:settings" for
    // the webview. Collapse here so React only binds one handler.
    let payload = if id == "app-menu:settings" {
        "app-menu:nav:settings".to_string()
    } else {
        id.to_string()
    };

    // Side-effects that don't require the webview:
    if id == "app-menu:help:reveal-data-dir" {
        let path = claudepot_core::paths::claudepot_data_dir();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(&path)
            .spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer")
            .arg(&path)
            .spawn();
        return;
    }
    if id == "app-menu:help:github" {
        let url = "https://github.com/xiaolai/claudepot-app";
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(url).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("cmd").args(["/c", "start", "", url]).spawn();
        return;
    }

    // For nav/settings, also ensure the window is focused so the
    // click is visible to the user (menu bar is active even when
    // no window is up).
    if payload.starts_with("app-menu:nav:") {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }

    if let Err(e) = app.emit("app-menu", &payload) {
        tracing::warn!("emit app-menu failed: {e}");
    }
}
