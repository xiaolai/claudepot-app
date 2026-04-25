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
use crate::tray_icons::{
    icon_item, ICON_CHECK, ICON_HOME, ICON_POWER, ICON_REFRESH, ICON_SLIDERS, ICON_USER_PLUS,
    ID_ACTIVE_DISPLAY, ID_ADD, ID_DESKTOP_BIND, ID_DESKTOP_CLEAR, ID_DESKTOP_LAUNCH,
    ID_DESKTOP_RECONCILE, ID_QUIT, ID_SETTINGS, ID_SHOW, ID_SYNC, ID_USAGE_REFRESH, PREFIX_CLI,
    PREFIX_DESKTOP, PREFIX_LIVE,
};
use crate::tray_menu::{
    build_cli_submenu, build_desktop_submenu, build_live_submenu, build_tooltip,
    build_usage_submenu,
};
use claudepot_core::oauth::usage::UsageResponse;
use claudepot_core::services::usage_cache::UsageCache;
use tauri::image::Image;
use tauri::menu::{IconMenuItemBuilder, MenuBuilder, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager};

/// Build and set the tray menu from the current account state.
///
/// Async because the Usage submenu peeks the UsageCache (tokio Mutex).
/// The peek is sub-millisecond when uncontended and never blocks on
/// network — the submenu only renders cached snapshots, never forces a
/// refetch. Callers from sync contexts (setup hook, event listener)
/// should wrap in `tauri::async_runtime::spawn`.
pub async fn rebuild(app: &AppHandle) -> Result<(), String> {
    let store = crate::commands::open_store()?;
    let accounts = store.list().map_err(|e| format!("list: {e}"))?;
    let summaries: Vec<AccountSummary> = accounts.iter().map(AccountSummary::from).collect();

    let cli_active = summaries.iter().find(|a| a.is_cli_active);
    let desktop_active = summaries.iter().find(|a| a.is_desktop_active);

    // Peek usage cache for every account with credentials. Unmanaged
    // state is possible during test harness use; fall through with an
    // empty map in that case so the tray still builds.
    let usage_snapshots: Vec<(AccountSummary, Option<UsageResponse>)> =
        if let Some(cache) = app.try_state::<UsageCache>() {
            let mut pairs = Vec::with_capacity(summaries.len());
            for s in &summaries {
                let snapshot = if s.credentials_healthy {
                    // Summary carries uuid as a String for JS; parse
                    // back to Uuid for the cache key. A malformed
                    // string here would mean an upstream bug, so fall
                    // through to None rather than panic.
                    match uuid::Uuid::parse_str(&s.uuid) {
                        Ok(id) => cache.peek_cached(id).await,
                        Err(_) => None,
                    }
                } else {
                    None
                };
                pairs.push((s.clone(), snapshot));
            }
            pairs
        } else {
            summaries.iter().cloned().map(|s| (s, None)).collect()
        };

    // 1. Active-account row (display-only — disabled). Uses
    // IconMenuItem with a check glyph instead of CheckMenuItem so
    // the glyph sits in the same image column as every other row;
    // AppKit renders CheckMenuItem.state in its own (leftmost)
    // slot, which visually misaligned the active row from the
    // icon stack below it.
    // Active-account display priority (Codex G7 — v1 showed CLI only,
    // leaving Desktop bindings invisible in the menu header when CLI
    // was unbound):
    //   CLI bound → show CLI (existing behavior)
    //   CLI unbound + Desktop bound → show Desktop
    //   neither → "No accounts active"
    let active_label = match (cli_active, desktop_active) {
        (Some(a), _) if !a.credentials_healthy => format!("{} — re-auth needed", a.email),
        (Some(a), _) => a.email.clone(),
        (None, Some(d)) => format!("{} (Desktop)", d.email),
        (None, None) => "No accounts active".to_string(),
    };
    let has_active = cli_active.is_some() || desktop_active.is_some();
    let active_item = if has_active {
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

    // 3b. Usage report submenu — cached snapshot of 5h / 7d / extras per
    //     account. "Briefly" per the feature request: one line per
    //     account, no chrome. Always reflects whatever was last cached;
    //     a "Refresh" footer triggers a fresh fetch + tray rebuild.
    let usage_submenu = build_usage_submenu(app, &usage_snapshots)?;

    // 3c. Live sessions submenu — reads the current aggregate from
    //     LiveSessionState. One row per live session (project · model
    //     · current action · elapsed). Empty → submenu itself is
    //     hidden (render-if-nonzero on the menu level).
    let live_submenu = build_live_submenu(app)?;

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
    let mut menu_builder = MenuBuilder::new(app)
        .item(&active_item)
        .item(&cli_submenu)
        .item(&desktop_submenu)
        .item(&usage_submenu);
    if let Some(ref ls) = live_submenu {
        menu_builder = menu_builder.item(ls);
    }
    let menu = menu_builder
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


/// Handle a tray menu item click. The app_menu module handles any id
/// starting with `app-menu:`; the rest live here.
pub fn handle_menu_event(app: &AppHandle, id: &str) {
    if let Some(uuid_str) = id.strip_prefix(PREFIX_CLI) {
        handle_cli_switch(app, uuid_str);
    } else if let Some(uuid_str) = id.strip_prefix(PREFIX_DESKTOP) {
        handle_desktop_switch(app, uuid_str);
    } else if let Some(sid) = id.strip_prefix(PREFIX_LIVE) {
        // Open the window and forward the session id to the React
        // side so it can route to Sessions with that session selected.
        // The existing `cp-goto-session` event bus takes a file_path,
        // not a session id — so we pair with a new event the JS App
        // listens to.
        show_window(app);
        if let Err(e) = app.emit("cp-activity-open-session", sid) {
            tracing::warn!("emit activity-open-session failed: {e}");
        }
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
    } else if id == ID_USAGE_REFRESH {
        handle_usage_refresh(app);
    } else if id == ID_DESKTOP_BIND {
        // Open the main window + route to Accounts — the user picks
        // which account to bind into. (Tray action alone can't know
        // the target; the match requires a /profile round-trip.)
        show_window(app);
        if let Err(e) = app.emit("cp-tray-desktop-bind", ()) {
            tracing::warn!("emit desktop-bind failed: {e}");
        }
    } else if id == ID_DESKTOP_CLEAR {
        // Tier 1 (Codex follow-up MEDIUM): route through the webview
        // so the shell's DesktopConfirmDialog shows before the
        // destructive clear runs. Never fire clear_session without
        // an explicit user confirmation.
        show_window(app);
        if let Err(e) = app.emit("cp-tray-desktop-clear", ()) {
            tracing::warn!("emit desktop-clear failed: {e}");
        }
    } else if id == ID_DESKTOP_LAUNCH {
        handle_desktop_launch(app);
    } else if id == ID_DESKTOP_RECONCILE {
        handle_desktop_reconcile(app);
    } else if id == ID_QUIT {
        app.exit(0);
    }
}

fn handle_desktop_launch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(platform) = claudepot_core::desktop_backend::create_platform() else {
            return;
        };
        if let Err(e) = platform.launch().await {
            tracing::warn!("tray desktop-launch failed: {e}");
            let _ = app.emit("tray-desktop-launch-failed", e.to_string());
        }
    });
}

fn handle_desktop_reconcile(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let store = match crate::commands::open_store() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("tray desktop-reconcile: open_store: {e}");
                return;
            }
        };
        match claudepot_core::services::desktop_service::reconcile_flags(&store) {
            Ok(outcome) => {
                if !outcome.flag_flips.is_empty() || outcome.orphan_pointer_cleared {
                    let _ = rebuild(&app).await;
                }
                let _ = app.emit("desktop-reconciled", outcome.flag_flips.len());
            }
            Err(e) => tracing::warn!("tray desktop-reconcile failed: {e}"),
        }
    });
}

/// Force-fetch usage for every credential-bearing account and rebuild
/// the tray with the fresh snapshot. The tray itself blocks only on
/// the peek; the fetch runs off the UI thread and rebuilds on
/// completion. Notifies the main window so its usage cards stay in
/// sync with the tray view.
fn handle_usage_refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(cache) = app.try_state::<UsageCache>() else {
            tracing::warn!("tray usage refresh: UsageCache not managed");
            return;
        };
        let store = match crate::commands::open_store() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("tray usage refresh: open_store failed: {e}");
                return;
            }
        };
        let accounts = match store.list() {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("tray usage refresh: list failed: {e}");
                return;
            }
        };
        let uuids: Vec<uuid::Uuid> = accounts
            .iter()
            .filter(|a| a.has_cli_credentials)
            .map(|a| a.uuid)
            .collect();
        // Invalidate first so the batch actually refetches instead of
        // replaying a stale cached response.
        for id in &uuids {
            cache.invalidate(*id).await;
        }
        let _ = cache.fetch_batch_detailed_verified(&store, &uuids).await;
        if let Err(e) = rebuild(&app).await {
            tracing::warn!("tray rebuild after usage refresh failed: {e}");
        }
        // Let the webview know so its cards re-query /oauth/usage and
        // pick up the same fresh values from the cache.
        let _ = app.emit("tray-usage-refreshed", ());
    });
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
        match crate::commands_cli::cli_use(email.clone(), None).await {
            Ok(()) => {
                if let Err(e) = rebuild(&app).await {
                    tracing::warn!("tray rebuild after cli switch failed: {e}");
                }
                let _ = app.emit("tray-cli-switched", ());
            }
            Err(e) => {
                tracing::warn!("tray cli_use failed: {e}");
                // Route the live-session conflict to a typed event so
                // the React layer can surface the same Override
                // affordance the in-app card path already has via
                // `useActions.useCli`. The tray's own click handler
                // has no `--force` channel, so a generic error toast
                // here would leave the user with no way to proceed.
                if e.to_lowercase().contains("claude code process is running") {
                    let _ = app.emit("tray-cli-switch-needs-override", email);
                } else {
                    let _ = app.emit("tray-cli-switch-failed", e);
                }
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
        //
        // desktop_use now takes a DesktopOpState reference; pull it
        // from the app's managed state so the operation-lock wraps
        // the switch (Codex follow-up review D1).
        let lock = match app.try_state::<crate::state::DesktopOpState>() {
            Some(l) => l,
            None => {
                tracing::warn!("tray: DesktopOpState not managed");
                return;
            }
        };
        match crate::commands_desktop::desktop_use(email, true, lock).await {
            Ok(()) => {
                if let Err(e) = rebuild(&app).await {
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
