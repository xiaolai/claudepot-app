//! System tray menu — quick-switch accounts without opening the window.
//!
//! Budget: ≤ 10 top-level items. Accounts never render inline — they
//! live inside two submenus (Switch CLI, Set Desktop) so the top level
//! stays readable regardless of account count. No emoji in labels;
//! paper-mono rules ban them, and AppKit native menu rendering ignores
//! custom font hinting anyway.
//!
//! Accelerators are deliberately absent on items that duplicate in-app
//! shortcuts (⌘R refresh, ⌘1..⌘4 section switch). They belong to the
//! webview, not the tray — see earlier bug where `⌘R` shadowed the
//! in-app refresh.

use crate::dto::AccountSummary;
use tauri::image::Image;
use tauri::menu::{
    IconMenuItemBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager};

// Mono glyph icons rendered from JetBrains Mono Nerd Font — matches the
// webview's paper-mono register. Source PNGs live at
// `src-tauri/icons/menu/*.png` at 36x36 @2x (muda scales every
// menu-item icon to 18pt anyway), fill `#888` so the glyph reads on
// both Light and Dark menu backgrounds. Muda never calls
// `setTemplate:YES` on custom bitmaps, so pure-black or pure-white
// would disappear in one mode — mid-gray is the pragmatic compromise.
const ICON_TERMINAL: &[u8] = include_bytes!("../icons/menu/terminal.png");
const ICON_DESKTOP: &[u8] = include_bytes!("../icons/menu/desktop.png");
const ICON_USER_PLUS: &[u8] = include_bytes!("../icons/menu/user-plus.png");
const ICON_REFRESH: &[u8] = include_bytes!("../icons/menu/refresh.png");
const ICON_HOME: &[u8] = include_bytes!("../icons/menu/home.png");
const ICON_SLIDERS: &[u8] = include_bytes!("../icons/menu/sliders.png");
const ICON_CHECK: &[u8] = include_bytes!("../icons/menu/check.png");
const ICON_POWER: &[u8] = include_bytes!("../icons/menu/power.png");

/// Build an icon menu item from pre-rendered PNG bytes.
fn icon_item(
    app: &AppHandle,
    id: &str,
    label: &str,
    bytes: &'static [u8],
) -> Result<tauri::menu::IconMenuItem<tauri::Wry>, String> {
    let img = Image::from_bytes(bytes).map_err(|e| format!("icon {id}: {e}"))?;
    IconMenuItemBuilder::with_id(id, label)
        .icon(img)
        .build(app)
        .map_err(|e| format!("icon item {id}: {e}"))
}

const ID_SHOW: &str = "tray:show";
const ID_SETTINGS: &str = "tray:settings";
const ID_QUIT: &str = "tray:quit";
const ID_ADD: &str = "tray:add-account";
const ID_SYNC: &str = "tray:sync-cc";
const ID_ACTIVE_DISPLAY: &str = "tray:active-display";
const PREFIX_CLI: &str = "tray:cli-switch:";
const PREFIX_DESKTOP: &str = "tray:desktop-switch:";

/// Build and set the tray menu from the current account state.
pub fn rebuild(app: &AppHandle) -> Result<(), String> {
    let store = crate::commands::open_store()?;
    let accounts = store.list().map_err(|e| format!("list: {e}"))?;
    let summaries: Vec<AccountSummary> = accounts.iter().map(AccountSummary::from).collect();

    let cli_active = summaries.iter().find(|a| a.is_cli_active);
    let desktop_active = summaries.iter().find(|a| a.is_desktop_active);

    // 1. Active-account row (display-only — disabled). Uses
    // IconMenuItem with a check glyph instead of CheckMenuItem so
    // the glyph sits in the same image column as every other row;
    // AppKit renders CheckMenuItem.state in its own (leftmost)
    // slot, which visually misaligned the active row from the
    // icon stack below it.
    let active_label = match cli_active {
        Some(a) if !a.credentials_healthy => format!("{} — re-auth needed", a.email),
        Some(a) => a.email.clone(),
        None => "No CLI account active".to_string(),
    };
    let active_item = if cli_active.is_some() {
        let img = Image::from_bytes(ICON_CHECK)
            .map_err(|e| format!("check icon: {e}"))?;
        IconMenuItemBuilder::with_id(ID_ACTIVE_DISPLAY, active_label)
            .icon(img)
            .enabled(false)
            .build(app)
            .map_err(|e| format!("active item: {e}"))?
    } else {
        // No icon when nothing's active — the label itself carries
        // the state. Still an IconMenuItem (just without an image)
        // so the column alignment stays consistent.
        IconMenuItemBuilder::with_id(ID_ACTIVE_DISPLAY, active_label)
            .enabled(false)
            .build(app)
            .map_err(|e| format!("active item: {e}"))?
    };

    // 2. Switch CLI submenu.
    let cli_submenu = build_cli_submenu(app, &summaries)?;

    // 3. Set Desktop submenu.
    let desktop_submenu = build_desktop_submenu(app, &summaries)?;

    // 4. Standalone items. macOS gets IconMenuItem + a pre-rendered
    // Nerd Font PNG so the whole tray carries the same paper-mono
    // register as the webview. Windows/Linux fall back to plain
    // MenuItem (IconMenuItemBuilder exists cross-platform but the
    // visual result isn't worth the weight without template-tinting).
    let sep1 =
        PredefinedMenuItem::separator(app).map_err(|e| format!("sep1: {e}"))?;

    let add_item = icon_item(app, ID_ADD, "Add account from browser…", ICON_USER_PLUS)?;
    let sync_item = icon_item(app, ID_SYNC, "Sync from current CC", ICON_REFRESH)?;

    let sep2 =
        PredefinedMenuItem::separator(app).map_err(|e| format!("sep2: {e}"))?;

    let show_item = icon_item(app, ID_SHOW, "Show Claudepot", ICON_HOME)?;
    let settings_item = icon_item(app, ID_SETTINGS, "Settings…", ICON_SLIDERS)?;

    // Quit carries a power glyph for column consistency with the
    // rest of the stack. macOS convention leaves system Quit items
    // bare, but here every other row is iconized and a lone
    // text-only Quit row misaligned the whole menu.
    let quit_icon = Image::from_bytes(ICON_POWER)
        .map_err(|e| format!("power icon: {e}"))?;
    let quit_item = IconMenuItemBuilder::with_id(ID_QUIT, "Quit Claudepot")
        .icon(quit_icon)
        .accelerator("CmdOrCtrl+Q")
        .build(app)
        .map_err(|e| format!("quit: {e}"))?;

    // Final assembly. Exactly 10 top-level items including 2
    // separators — one below the account group, one below the
    // actions group. No separator before Quit; matches the layout
    // used by common status-bar apps (Tailscale, Raycast).
    let menu = MenuBuilder::new(app)
        .item(&active_item)
        .item(&cli_submenu)
        .item(&desktop_submenu)
        .item(&sep1)
        .item(&add_item)
        .item(&sync_item)
        .item(&sep2)
        .item(&show_item)
        .item(&settings_item)
        .item(&quit_item)
        .build()
        .map_err(|e| format!("menu build: {e}"))?;

    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))
            .map_err(|e| format!("set menu: {e}"))?;
        let tooltip = build_tooltip(cli_active, desktop_active);
        tray.set_tooltip(Some(&tooltip))
            .map_err(|e| format!("tooltip: {e}"))?;
    }

    Ok(())
}

fn build_cli_submenu(
    app: &AppHandle,
    summaries: &[AccountSummary],
) -> Result<tauri::menu::Submenu<tauri::Wry>, String> {
    let mut builder = SubmenuBuilder::new(app, "Switch CLI");
    if let Ok(img) = Image::from_bytes(ICON_TERMINAL) {
        builder = builder.submenu_icon(img);
    }
    let mut any = false;
    for s in summaries {
        if s.is_cli_active {
            continue;
        }
        any = true;
        let label = if s.credentials_healthy {
            s.email.clone()
        } else {
            format!("{} (re-auth needed)", s.email)
        };
        let item = MenuItemBuilder::with_id(format!("{PREFIX_CLI}{}", s.uuid), label)
            .enabled(s.credentials_healthy)
            .build(app)
            .map_err(|e| format!("cli item: {e}"))?;
        builder = builder.item(&item);
    }
    if !any {
        let empty = MenuItemBuilder::with_id("tray:cli-switch:empty", "No other accounts")
            .enabled(false)
            .build(app)
            .map_err(|e| format!("cli empty: {e}"))?;
        builder = builder.item(&empty);
    }
    builder.build().map_err(|e| format!("cli submenu: {e}"))
}

fn build_desktop_submenu(
    app: &AppHandle,
    summaries: &[AccountSummary],
) -> Result<tauri::menu::Submenu<tauri::Wry>, String> {
    let mut builder = SubmenuBuilder::new(app, "Set Desktop");
    if let Ok(img) = Image::from_bytes(ICON_DESKTOP) {
        builder = builder.submenu_icon(img);
    }
    let mut any = false;
    for s in summaries {
        if s.is_desktop_active {
            continue;
        }
        any = true;
        let label = if s.has_desktop_profile {
            s.email.clone()
        } else {
            format!("{} (no Desktop profile)", s.email)
        };
        let item =
            MenuItemBuilder::with_id(format!("{PREFIX_DESKTOP}{}", s.uuid), label)
                .enabled(s.has_desktop_profile)
                .build(app)
                .map_err(|e| format!("desktop item: {e}"))?;
        builder = builder.item(&item);
    }
    if !any {
        let empty =
            MenuItemBuilder::with_id("tray:desktop-switch:empty", "No eligible accounts")
                .enabled(false)
                .build(app)
                .map_err(|e| format!("desktop empty: {e}"))?;
        builder = builder.item(&empty);
    }
    builder
        .build()
        .map_err(|e| format!("desktop submenu: {e}"))
}

fn build_tooltip(
    cli_active: Option<&AccountSummary>,
    desktop_active: Option<&AccountSummary>,
) -> String {
    match (cli_active, desktop_active) {
        (Some(c), Some(d)) if c.uuid == d.uuid => format!("Claudepot — {}", c.email),
        (Some(c), Some(d)) => {
            format!("Claudepot\nCLI: {}\nDesktop: {}", c.email, d.email)
        }
        (Some(c), None) => format!("Claudepot — {}", c.email),
        (None, Some(d)) => format!("Claudepot — Desktop: {}", d.email),
        (None, None) => "Claudepot".to_string(),
    }
}

/// Handle a tray menu item click. The app_menu module handles any id
/// starting with `app-menu:`; the rest live here.
pub fn handle_menu_event(app: &AppHandle, id: &str) {
    if let Some(uuid_str) = id.strip_prefix(PREFIX_CLI) {
        handle_cli_switch(app, uuid_str);
    } else if let Some(uuid_str) = id.strip_prefix(PREFIX_DESKTOP) {
        handle_desktop_switch(app, uuid_str);
    } else if id == ID_SHOW {
        show_window(app);
    } else if id == ID_SETTINGS {
        show_window(app);
        if let Err(e) = app.emit("app-menu", "app-menu:nav:settings") {
            tracing::warn!("emit settings nav failed: {e}");
        }
    } else if id == ID_ADD {
        show_window(app);
        if let Err(e) = app.emit("app-menu", "app-menu:account:login-browser") {
            tracing::warn!("emit add-account failed: {e}");
        }
    } else if id == ID_SYNC {
        if let Err(e) = app.emit("app-menu", "app-menu:account:sync-cc") {
            tracing::warn!("emit sync-cc failed: {e}");
        }
    } else if id == ID_QUIT {
        app.exit(0);
    }
}

fn handle_cli_switch(app: &AppHandle, uuid_str: &str) {
    if uuid_str == "empty" {
        return;
    }
    let Some(email) = find_email_for_uuid(uuid_str) else {
        tracing::warn!("tray: no account found for UUID {uuid_str}");
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match crate::commands::cli_use(email, None).await {
            Ok(()) => {
                if let Err(e) = rebuild(&app) {
                    tracing::warn!("tray rebuild after cli switch failed: {e}");
                }
                let _ = app.emit("tray-cli-switched", ());
            }
            Err(e) => {
                tracing::warn!("tray cli_use failed: {e}");
                let _ = app.emit("tray-cli-switch-failed", e.to_string());
            }
        }
    });
}

fn handle_desktop_switch(app: &AppHandle, uuid_str: &str) {
    if uuid_str == "empty" {
        return;
    }
    let Some(email) = find_email_for_uuid(uuid_str) else {
        tracing::warn!("tray: no account found for UUID {uuid_str}");
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // no_launch=true mirrors the app's typical UX — swap without
        // forcing Claude Desktop to relaunch from the tray.
        match crate::commands::desktop_use(email, true).await {
            Ok(()) => {
                if let Err(e) = rebuild(&app) {
                    tracing::warn!("tray rebuild after desktop switch failed: {e}");
                }
                let _ = app.emit("tray-desktop-switched", ());
            }
            Err(e) => {
                tracing::warn!("tray desktop_use failed: {e}");
                let _ = app.emit("tray-desktop-switch-failed", e.to_string());
            }
        }
    });
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
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
