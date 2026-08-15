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
//! macOS-standard ⌘, ⌘Q, and ⌘W.
//!
//! Tray-resident model. The app is meant to live in the menubar; ⌘Q,
//! ⌘W, and the red ✕ all hide the main window rather than quitting.
//! Background work (live activity runtime, usage_watcher, OS
//! notifications) keeps running. The single "Quit" entry that
//! actually exits the process is in the tray icon's dropdown — it
//! routes through `attempt_quit`, which gates on the `RunningOps`
//! map and surfaces a confirm modal when in-flight ops would be
//! abandoned. Window close handlers must NOT call `attempt_quit`
//! or `app.exit`, otherwise this surface stops being tray-resident.

use crate::i18n::{tr, tr_args, tr_n};
use crate::ops::{OpKind, OpStatus, RunningOps};
use serde::Serialize;
use tauri::menu::{
    AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Build the app menu and set it on the given app handle.
///
/// Re-runnable: `app.set_menu` replaces the previous menu wholesale, so
/// a locale change re-installs by calling this again (see
/// `preferences_set_locale`). Labels resolve through `i18n::tr` at
/// build time — never cache them across a locale switch.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // ---- Claudepot submenu --------------------------------------------------
    let about_md = AboutMetadataBuilder::new()
        .name(Some("Claudepot"))
        .version(Some(env!("CARGO_PKG_VERSION")))
        .copyright(Some("© HANDO K.K."))
        .website(Some("https://github.com/xiaolai/claudepot-app"))
        .website_label(Some("GitHub"))
        .build();

    let about = PredefinedMenuItem::about(app, Some(&tr("menu.about")), Some(about_md))
        .map_err(|e| format!("about: {e}"))?;

    let settings = MenuItemBuilder::with_id("app-menu:settings", tr("menu.settings"))
        .accelerator("Cmd+,")
        .build(app)
        .map_err(|e| format!("settings: {e}"))?;

    let services = PredefinedMenuItem::services(app, Some(&tr("menu.services")))
        .map_err(|e| format!("services: {e}"))?;
    let hide =
        PredefinedMenuItem::hide(app, Some(&tr("menu.hide"))).map_err(|e| format!("hide: {e}"))?;
    let hide_others = PredefinedMenuItem::hide_others(app, Some(&tr("menu.hideOthers")))
        .map_err(|e| format!("hide_others: {e}"))?;
    let show_all = PredefinedMenuItem::show_all(app, Some(&tr("menu.showAll")))
        .map_err(|e| format!("show_all: {e}"))?;
    // ⌘Q is rebound to "Close Window" so the app behaves as a tray-
    // resident background worker: the gesture users press to dismiss
    // the UI hides the window and lets background tasks (live runtime,
    // usage watcher, notifications) keep running. The id stays
    // `app-menu:quit` to avoid churning the menu-event router; the
    // handler in `handle_menu_event` is what changed (now hides
    // instead of calling `attempt_quit`). `CmdOrCtrl+Q` so Linux /
    // Windows builds use Ctrl+Q.
    //
    // The actual process-terminating Quit lives in the tray dropdown
    // — see `tray.rs::handle_menu_event` (id `quit`) → `attempt_quit`.
    let quit = MenuItemBuilder::with_id("app-menu:quit", tr("menu.closeWindow"))
        .accelerator("CmdOrCtrl+Q")
        .build(app)
        .map_err(|e| format!("quit: {e}"))?;

    let claudepot = SubmenuBuilder::new(app, tr("menu.claudepot"))
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

    // ---- Edit submenu ------------------------------------------------------
    // Required on macOS: without the predefined copy/cut/paste/select_all
    // items in an Edit menu, WebKit `<input>` fields don't receive ⌘C/⌘V/
    // ⌘X/⌘A keyboard shortcuts. `app.set_menu()` replaces the default menu
    // entirely, so we have to put these back by hand.
    let undo =
        PredefinedMenuItem::undo(app, Some(&tr("menu.undo"))).map_err(|e| format!("undo: {e}"))?;
    let redo =
        PredefinedMenuItem::redo(app, Some(&tr("menu.redo"))).map_err(|e| format!("redo: {e}"))?;
    let cut =
        PredefinedMenuItem::cut(app, Some(&tr("menu.cut"))).map_err(|e| format!("cut: {e}"))?;
    let copy =
        PredefinedMenuItem::copy(app, Some(&tr("menu.copy"))).map_err(|e| format!("copy: {e}"))?;
    let paste = PredefinedMenuItem::paste(app, Some(&tr("menu.paste")))
        .map_err(|e| format!("paste: {e}"))?;
    let select_all = PredefinedMenuItem::select_all(app, Some(&tr("menu.selectAll")))
        .map_err(|e| format!("select_all: {e}"))?;

    let edit = SubmenuBuilder::new(app, tr("menu.edit"))
        .item(&undo)
        .item(&redo)
        .separator()
        .item(&cut)
        .item(&copy)
        .item(&paste)
        .separator()
        .item(&select_all)
        .build()
        .map_err(|e| format!("edit submenu: {e}"))?;

    // ---- Account submenu (static; no dynamic rebuild needed) ---------------
    // No "Manage accounts…" entry — it would emit `app-menu:nav:accounts`
    // (the router strips the `:manage` suffix), which is the same target
    // as View → Accounts. One nav target, one menu item.
    let login_browser =
        MenuItemBuilder::with_id("app-menu:account:login-browser", tr("menu.signInBrowser"))
            .build(app)
            .map_err(|e| format!("login-browser: {e}"))?;
    let sync_cc = MenuItemBuilder::with_id("app-menu:account:sync-cc", tr("menu.syncFromCc"))
        .build(app)
        .map_err(|e| format!("sync-cc: {e}"))?;
    let verify_all = MenuItemBuilder::with_id("app-menu:account:verify-all", tr("menu.verifyAll"))
        .build(app)
        .map_err(|e| format!("verify-all: {e}"))?;

    let account = SubmenuBuilder::new(app, tr("menu.account"))
        .item(&login_browser)
        .item(&sync_cc)
        .item(&verify_all)
        .build()
        .map_err(|e| format!("account submenu: {e}"))?;

    // ---- View submenu ------------------------------------------------------
    // Mirrors `src/sections/registry.tsx` in order. Each id must match
    // a registered section id — App.tsx silently drops unknown ids
    // (`sectionIds.includes(sub)`), so a typo here = dead menu item.
    // Note: Activities's section id is `events` for localStorage
    // back-compat; the label is "Activities".
    let nav_accounts = MenuItemBuilder::with_id("app-menu:nav:accounts", tr("menu.navAccounts"))
        .build(app)
        .map_err(|e| format!("nav-accounts: {e}"))?;
    let nav_activities = MenuItemBuilder::with_id("app-menu:nav:events", tr("menu.navActivities"))
        .build(app)
        .map_err(|e| format!("nav-activities: {e}"))?;
    let nav_projects = MenuItemBuilder::with_id("app-menu:nav:projects", tr("menu.navProjects"))
        .build(app)
        .map_err(|e| format!("nav-projects: {e}"))?;
    let nav_keys = MenuItemBuilder::with_id("app-menu:nav:keys", tr("menu.navKeys"))
        .build(app)
        .map_err(|e| format!("nav-keys: {e}"))?;
    let nav_third_party =
        MenuItemBuilder::with_id("app-menu:nav:third-party", tr("menu.navThirdParty"))
            .build(app)
            .map_err(|e| format!("nav-third-party: {e}"))?;
    // `automations`, not `agents`: the registry id predates the "Agents"
    // label and is kept for localStorage compatibility. Emitting the
    // LABEL meant View -> Agents named a section that does not exist,
    // so the item was built, shown, and did nothing when clicked.
    let nav_agents = MenuItemBuilder::with_id("app-menu:nav:automations", tr("menu.navAgents"))
        .build(app)
        .map_err(|e| format!("nav-agents: {e}"))?;
    let nav_global = MenuItemBuilder::with_id("app-menu:nav:global", tr("menu.navGlobal"))
        .build(app)
        .map_err(|e| format!("nav-global: {e}"))?;
    let nav_settings = MenuItemBuilder::with_id("app-menu:nav:settings", tr("menu.navSettings"))
        .build(app)
        .map_err(|e| format!("nav-settings: {e}"))?;
    let toggle_theme =
        MenuItemBuilder::with_id("app-menu:view:toggle-theme", tr("menu.toggleTheme"))
            .build(app)
            .map_err(|e| format!("toggle-theme: {e}"))?;
    // Honest label: the handler in App.tsx only refreshes the Accounts
    // section. A section-aware refresh contract doesn't exist yet, so
    // don't promise one in the label.
    let reload = MenuItemBuilder::with_id("app-menu:view:reload", tr("menu.refreshAccounts"))
        .build(app)
        .map_err(|e| format!("reload: {e}"))?;

    let view = SubmenuBuilder::new(app, tr("menu.view"))
        .item(&nav_accounts)
        .item(&nav_activities)
        .item(&nav_projects)
        .item(&nav_keys)
        .item(&nav_third_party)
        .item(&nav_agents)
        .item(&nav_global)
        .item(&nav_settings)
        .separator()
        .item(&toggle_theme)
        .item(&reload)
        .build()
        .map_err(|e| format!("view submenu: {e}"))?;

    // ---- Window submenu (standard macOS set) -------------------------------
    // `close_window` is a CUSTOM item, not `PredefinedMenuItem::close_window`.
    // The predefined variant tears the window down; we want ⌘W to
    // *hide* it so the tray-resident model holds (the only Quit lives
    // in the tray dropdown — see the `quit` item above for full
    // rationale). Same handler as `app-menu:quit`; same intent.
    let minimize = PredefinedMenuItem::minimize(app, Some(&tr("menu.minimize")))
        .map_err(|e| format!("minimize: {e}"))?;
    let maximize = PredefinedMenuItem::maximize(app, Some(&tr("menu.zoom")))
        .map_err(|e| format!("maximize: {e}"))?;
    let close_window = MenuItemBuilder::with_id("app-menu:close-window", tr("menu.closeWindow"))
        .accelerator("CmdOrCtrl+W")
        .build(app)
        .map_err(|e| format!("close_window: {e}"))?;

    let window = SubmenuBuilder::new(app, tr("menu.window"))
        .item(&minimize)
        .item(&maximize)
        .separator()
        .item(&close_window)
        .build()
        .map_err(|e| format!("window submenu: {e}"))?;

    // ---- Help submenu ------------------------------------------------------
    let help_github = MenuItemBuilder::with_id("app-menu:help:github", tr("menu.openGithub"))
        .build(app)
        .map_err(|e| format!("help-github: {e}"))?;
    let help_diag = MenuItemBuilder::with_id("app-menu:help:copy-diag", tr("menu.copyDiagnostics"))
        .build(app)
        .map_err(|e| format!("help-diag: {e}"))?;
    let help_data =
        MenuItemBuilder::with_id("app-menu:help:reveal-data-dir", tr("menu.revealDataDir"))
            .build(app)
            .map_err(|e| format!("help-data: {e}"))?;

    let help = SubmenuBuilder::new(app, tr("menu.help"))
        .item(&help_github)
        .item(&help_diag)
        .item(&help_data)
        .build()
        .map_err(|e| format!("help submenu: {e}"))?;

    let menu = MenuBuilder::new(app)
        .items(&[&claudepot, &edit, &account, &view, &window, &help])
        .build()
        .map_err(|e| format!("root menu: {e}"))?;

    app.set_menu(menu).map_err(|e| format!("set_menu: {e}"))?;
    Ok(())
}

/// Route a menu event. Settings gets opened directly (section switch
/// plus window focus); everything else forwards to the webview via
/// a single `app-menu` event, with the menu id as payload.
pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    // ⌘Q (id `app-menu:quit`) and ⌘W (id `app-menu:close-window`)
    // both hide the main window — the app is tray-resident, so
    // these gestures dismiss the UI rather than ending the process.
    // The actual process-terminating Quit lives in the tray dropdown
    // (see `tray::handle_menu_event` for id `quit` → `attempt_quit`).
    // Return before the generic forward path so the renderer never
    // sees a stray `app-menu:quit` / `app-menu:close-window` event.
    if id == "app-menu:quit" || id == "app-menu:close-window" {
        if let Some(window) = app.get_webview_window("main") {
            if let Err(e) = window.hide() {
                tracing::warn!("hide main window failed: {e}");
            }
        }
        return;
    }

    // "app-menu:settings" is identical to "app-menu:nav:settings" for
    // the webview. Collapse here so React only binds one handler.
    let payload = if id == "app-menu:settings" {
        "app-menu:nav:settings".to_string()
    } else {
        id.to_string()
    };

    // Side-effects that don't require the webview. Both route through
    // tauri-plugin-opener (initialized in lib.rs), which carries
    // maintained cross-platform escaping — the previous hand-rolled
    // per-OS subprocess blocks mixed a PATH-resolved `open` with an
    // absolute `/usr/bin/open` and used the `cmd /c start` injection-
    // footgun shape on Windows.
    if id == "app-menu:help:reveal-data-dir" {
        use tauri_plugin_opener::OpenerExt;
        let path = claudepot_core::paths::claudepot_data_dir();
        if let Err(e) = app.opener().reveal_item_in_dir(&path) {
            tracing::warn!("reveal data dir failed: {e}");
        }
        return;
    }
    if id == "app-menu:help:github" {
        use tauri_plugin_opener::OpenerExt;
        let url = "https://github.com/xiaolai/claudepot-app";
        if let Err(e) = app.opener().open_url(url, None::<&str>) {
            tracing::warn!("open GitHub url failed: {e}");
        }
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

/// Snapshot row sent to the renderer when quit is gated. Carries only
/// what the modal needs — kind for the icon, label for the body. Full
/// `RunningOpInfo` would leak file paths into the JS heap unnecessarily.
#[derive(Debug, Clone, Serialize)]
pub struct QuitGateOp {
    pub op_id: String,
    pub kind: OpKind,
    pub label: String,
}

/// Quit gate. Reads `RunningOps`; if any op is still in `Running` status
/// (the map also lingers Complete/Error rows for a 5 s grace), emits
/// `cp-quit-requested` with a snapshot so the renderer can show a
/// confirm modal. Otherwise exits immediately.
///
/// Both the menubar Quit and the tray Quit route through this — adding
/// a third quit surface (e.g., a window-close handler) without using
/// this helper turns the gate into a sieve.
pub fn attempt_quit<R: Runtime>(app: &AppHandle<R>) {
    let in_flight: Vec<QuitGateOp> = match app.try_state::<RunningOps>() {
        Some(ops) => ops
            .list()
            .into_iter()
            .filter(|op| op.status == OpStatus::Running)
            .map(|op| QuitGateOp {
                op_id: op.op_id.clone(),
                kind: op.kind,
                label: inflight_label(&op),
            })
            .collect(),
        // No state managed — happens in unusual harness builds. Quit
        // is the safe default; there are no tracked ops to lose.
        None => Vec::new(),
    };

    if in_flight.is_empty() {
        app.exit(0);
        return;
    }

    // Preferred path: surface the rich React modal in the main
    // webview. Falls back to a native dialog when the webview is
    // unavailable (window destroyed, or emit failure) so the user
    // always has an explicit quit/stay choice — never a silent
    // dead-end and never a silent exit mid-op.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        match app.emit("cp-quit-requested", &in_flight) {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(
                    "quit gate: emit cp-quit-requested failed: {e}; falling back to native dialog"
                );
            }
        }
    } else {
        tracing::info!(
            "quit gate: no main webview — falling back to native dialog ({} op(s) in flight)",
            in_flight.len()
        );
    }

    show_native_quit_dialog(app, &in_flight);
}

/// Last-resort quit confirmation. Used when the React modal can't be
/// shown (no main webview, or emit failure). Leans on
/// `tauri-plugin-dialog` — a non-blocking message dialog so the menu
/// event handler returns promptly. The dialog runs on the OS main
/// thread; the callback fires when the user clicks a button.
fn show_native_quit_dialog<R: Runtime>(app: &AppHandle<R>, in_flight: &[QuitGateOp]) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let heading = tr_n("quit.opsInProgress", in_flight.len() as u64);
    let body = {
        let mut lines = tr("quit.body");
        lines.push_str("\n\n");
        for op in in_flight {
            lines.push_str("• ");
            lines.push_str(&op.label);
            lines.push('\n');
        }
        lines
    };

    let app_for_cb = app.clone();
    app.dialog()
        .message(body)
        .title(heading)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            tr("quit.quitAnyway"),
            tr("quit.stay"),
        ))
        .show(move |ok| {
            if ok {
                app_for_cb.exit(0);
            }
        });
}

/// One-line in-flight label for the quit modal and the pre-relaunch
/// quiesce probe (`commands::release_update::release_relaunch_busy_ops`).
/// Mirrors the shape of `op_terminal_label` in `ops.rs` but uses
/// gerunds ("Renaming", not "Renamed") since the op is still running.
pub(crate) fn inflight_label(op: &crate::ops::RunningOpInfo) -> String {
    let from = basename(&op.old_path);
    let to = basename(&op.new_path);
    let from_to: [(&str, &str); 2] = [("from", from), ("to", to)];
    match op.kind {
        OpKind::CleanProjects => tr("ops.cleaningProjects"),
        OpKind::SessionPrune => tr("ops.pruningSessions"),
        OpKind::SessionSlim => {
            if from.is_empty() {
                tr("ops.slimmingSession")
            } else {
                tr_args("ops.slimming", &[("from", from)])
            }
        }
        OpKind::SessionShare => tr("ops.sharingSession"),
        OpKind::SessionMove => tr_args("ops.movingSession", &from_to),
        OpKind::MoveProject => tr_args("ops.renaming", &from_to),
        OpKind::RepairResume => tr_args("ops.resuming", &from_to),
        OpKind::RepairRollback => tr_args("ops.rollingBack", &from_to),
        OpKind::AccountLogin => tr("ops.accountLogin"),
        OpKind::AccountRegister => tr("ops.addingAccount"),
        OpKind::VerifyAll => tr("ops.verifyingAccounts"),
        OpKind::AgentRun => tr("ops.runningAgent"),
    }
}

fn basename(path: &str) -> &str {
    if path.is_empty() {
        return path;
    }
    let trimmed = path.trim_end_matches(['/', '\\']);
    let idx = trimmed.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    &trimmed[idx..]
}
