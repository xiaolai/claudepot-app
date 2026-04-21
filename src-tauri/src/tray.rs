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
use claudepot_core::oauth::usage::UsageResponse;
use claudepot_core::services::usage_cache::UsageCache;
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
const ICON_BAR_CHART: &[u8] = include_bytes!("../icons/menu/bar-chart.png");
const ICON_BOLT: &[u8] = include_bytes!("../icons/menu/bolt.png");
// Per-row glyphs. Usage rows carry a single account-identity anchor
// (circle-user); Live/Activity rows vary by status so the user can
// scan "what's actually happening" from the tray without opening
// the window — play = busy, pause = waiting, dot = idle.
const ICON_CIRCLE_USER: &[u8] = include_bytes!("../icons/menu/circle-user.png");
const ICON_CIRCLE_PLAY: &[u8] = include_bytes!("../icons/menu/circle-play.png");
const ICON_CIRCLE_PAUSE: &[u8] = include_bytes!("../icons/menu/circle-pause.png");
const ICON_CIRCLE_DOT: &[u8] = include_bytes!("../icons/menu/circle-dot.png");

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
const ID_USAGE_REFRESH: &str = "tray:usage:refresh";
pub const PREFIX_CLI: &str = "tray:cli-switch:";
pub const PREFIX_DESKTOP: &str = "tray:desktop-switch:";
/// Prefix for per-session rows in the Live submenu. Suffix is the
/// sessionId — the menu-event handler routes by this prefix and
/// opens the window with that session selected.
pub const PREFIX_LIVE: &str = "tray:live:";

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

/// One submenu row per account with credentials:
///   - Label: `email — 5h N% · 7d N% · Extra NN%/off`
///   - Disabled (display-only): clicking opens nothing, the value IS
///     the content. Entries without a cached snapshot render with a
///     "no data yet" suffix so the row doesn't lie.
///
/// Footer: a single "Refresh" item that triggers a fresh batch fetch
/// and rebuild, so users can top up the numbers without opening the
/// main window.
fn build_usage_submenu(
    app: &AppHandle,
    snapshots: &[(AccountSummary, Option<UsageResponse>)],
) -> Result<tauri::menu::Submenu<tauri::Wry>, String> {
    let mut builder = SubmenuBuilder::new(app, "Usage");
    if let Ok(img) = Image::from_bytes(ICON_BAR_CHART) {
        builder = builder.submenu_icon(img);
    }

    let mut any = false;
    for (s, snap) in snapshots {
        if !s.credentials_healthy {
            // Accounts without creds can't have usage; skip rather
            // than render a dead row. The active-account line above
            // already surfaces the "re-auth needed" signal.
            continue;
        }
        any = true;
        let label = format_usage_line(&s.email, snap.as_ref());
        let id = format!("tray:usage:row:{}", s.uuid);
        // Icon is best-effort — a broken asset must not take down
        // the whole submenu. Fall back to a plain text row. Each
        // branch is self-contained because IconMenuItem and
        // MenuItem are distinct concrete types — the builder's
        // `item(&dyn IsMenuItem<_>)` signature is polymorphic.
        match Image::from_bytes(ICON_CIRCLE_USER) {
            Ok(img) => {
                let item = IconMenuItemBuilder::with_id(&id, label)
                    .icon(img)
                    .enabled(false)
                    .build(app)
                    .map_err(|e| format!("usage item: {e}"))?;
                builder = builder.item(&item);
            }
            Err(_) => {
                let item = MenuItemBuilder::with_id(&id, label)
                    .enabled(false)
                    .build(app)
                    .map_err(|e| format!("usage item: {e}"))?;
                builder = builder.item(&item);
            }
        }
    }

    if !any {
        let empty = MenuItemBuilder::with_id("tray:usage:empty", "No accounts with credentials")
            .enabled(false)
            .build(app)
            .map_err(|e| format!("usage empty: {e}"))?;
        builder = builder.item(&empty);
    } else {
        let sep =
            PredefinedMenuItem::separator(app).map_err(|e| format!("usage sep: {e}"))?;
        builder = builder.item(&sep);
        let refresh_img =
            Image::from_bytes(ICON_REFRESH).map_err(|e| format!("usage refresh icon: {e}"))?;
        let refresh_item = IconMenuItemBuilder::with_id(ID_USAGE_REFRESH, "Refresh")
            .icon(refresh_img)
            .build(app)
            .map_err(|e| format!("usage refresh: {e}"))?;
        builder = builder.item(&refresh_item);
    }

    builder.build().map_err(|e| format!("usage submenu: {e}"))
}

/// Build the Live sessions submenu from the current aggregate
/// snapshot exposed by `LiveSessionState`. Returns `Ok(None)` when
/// no sessions are live — the caller omits the menu item entirely,
/// preserving the "render-if-nonzero" rule. Each row is disabled
/// (display-only) EXCEPT for the per-session opener, which routes
/// via `PREFIX_LIVE<sessionId>` in the menu-event handler.
fn build_live_submenu(
    app: &AppHandle,
) -> Result<Option<tauri::menu::Submenu<tauri::Wry>>, String> {
    let Some(state) = app.try_state::<crate::state::LiveSessionState>() else {
        return Ok(None);
    };
    let list = state.runtime.snapshot();
    if list.is_empty() {
        return Ok(None);
    }
    let label = format!("Active: {}", list.len());
    let mut builder = SubmenuBuilder::new(app, &label);
    if let Ok(img) = Image::from_bytes(ICON_BOLT) {
        builder = builder.submenu_icon(img);
    }
    for s in list.iter() {
        use claudepot_core::session_live::types::Status;
        let action = s
            .current_action
            .clone()
            .unwrap_or_else(|| match s.status {
                Status::Waiting => {
                    if let Some(w) = &s.waiting_for {
                        format!("waiting — {w}")
                    } else {
                        "waiting".to_string()
                    }
                }
                Status::Idle => "idle".to_string(),
                Status::Busy => "working".to_string(),
            });
        let line = format_live_row(&s.cwd, s.model.as_deref(), &action, s.idle_ms);
        let id = format!("{}{}", PREFIX_LIVE, s.session_id);
        // Status-varied per-row glyph so the tray conveys
        // "what's happening" without requiring the user to parse
        // the text after each label.
        let icon_bytes: &[u8] = match s.status {
            Status::Busy => ICON_CIRCLE_PLAY,
            Status::Waiting => ICON_CIRCLE_PAUSE,
            Status::Idle => ICON_CIRCLE_DOT,
        };
        match Image::from_bytes(icon_bytes) {
            Ok(img) => {
                let item = IconMenuItemBuilder::with_id(&id, line)
                    .icon(img)
                    .build(app)
                    .map_err(|e| format!("live item: {e}"))?;
                builder = builder.item(&item);
            }
            Err(_) => {
                let item = MenuItemBuilder::with_id(&id, line)
                    .build(app)
                    .map_err(|e| format!("live item: {e}"))?;
                builder = builder.item(&item);
            }
        }
    }
    Ok(Some(
        builder.build().map_err(|e| format!("live submenu: {e}"))?,
    ))
}

/// Format a single live-session row for the tray. Tray rows are
/// plain `&str` — no rich formatting available — so we pack
/// `project · model · action · elapsed` into a compact one-liner.
fn format_live_row(cwd: &str, model: Option<&str>, action: &str, idle_ms: i64) -> String {
    let project = cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(cwd);
    let family = match model {
        Some(m) if m.contains("opus") => "OPUS",
        Some(m) if m.contains("sonnet") => "SON",
        Some(m) if m.contains("haiku") => "HAI",
        Some(_) => "?",
        None => "?",
    };
    let elapsed = format_elapsed_short(idle_ms);
    // Clip action to 32 chars so the tray row doesn't wrap.
    let clipped: String = if action.chars().count() > 32 {
        let mut s: String = action.chars().take(31).collect();
        s.push('…');
        s
    } else {
        action.to_string()
    };
    format!("{project} · {family} · {clipped} · {elapsed}")
}

fn format_elapsed_short(ms: i64) -> String {
    if ms < 1_000 {
        return "—".to_string();
    }
    let secs = ms / 1_000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let m = secs / 60;
    let s = secs % 60;
    if m < 60 {
        return format!("{m}:{s:02}");
    }
    let h = m / 60;
    format!("{h}h{}m", m % 60)
}

/// Compact one-liner for a tray row. `5h 77% · 7d 33% · Extra 100%`.
/// Only non-null windows contribute; extras appears only when enabled.
/// Returns a "(no data)" sentinel when the snapshot is None so the row
/// doesn't pretend to have information.
fn format_usage_line(email: &str, snap: Option<&UsageResponse>) -> String {
    let Some(u) = snap else {
        return format!("{email} — (no data — click Refresh)");
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(w) = u.five_hour.as_ref() {
        parts.push(format!("5h {}%", w.utilization.round() as i64));
    }
    if let Some(w) = u.seven_day.as_ref() {
        parts.push(format!("7d {}%", w.utilization.round() as i64));
    }
    if let Some(extra) = u.extra_usage.as_ref() {
        if extra.is_enabled {
            let pct = extra
                .utilization
                .or_else(|| match (extra.used_credits, extra.monthly_limit) {
                    (Some(used), Some(limit)) if limit > 0.0 => Some((used / limit) * 100.0),
                    _ => None,
                })
                .map(|p| p.round() as i64);
            match pct {
                Some(p) => parts.push(format!("Extra {p}%")),
                None => parts.push("Extra on".to_string()),
            }
        } else {
            parts.push("Extra off".to_string());
        }
    }
    if parts.is_empty() {
        format!("{email} — (no windows reported)")
    } else {
        format!("{email} — {}", parts.join(" · "))
    }
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
    } else if id == ID_QUIT {
        app.exit(0);
    }
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
        match crate::commands::cli_use(email, None).await {
            Ok(()) => {
                if let Err(e) = rebuild(&app).await {
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
