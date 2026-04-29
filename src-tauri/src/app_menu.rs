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
//!
//! Quit gate. ⌘Q is a custom menu item (id `app-menu:quit`), not the
//! Tauri-predefined Quit, because the predefined item ignores Rust-side
//! state and would tear down the process mid-op. The gate consults the
//! `RunningOps` map: empty → `app.exit(0)`; non-empty → emit
//! `cp-quit-requested` with an op snapshot so the renderer can show a
//! confirm modal. Both this menu and the tray's Quit row route through
//! the same `attempt_quit` helper so the gate is not a sieve.

use crate::ops::{OpKind, OpStatus, RunningOps};
use serde::Serialize;
use tauri::menu::{
    AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
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
    let hide =
        PredefinedMenuItem::hide(app, Some("Hide Claudepot")).map_err(|e| format!("hide: {e}"))?;
    let hide_others = PredefinedMenuItem::hide_others(app, Some("Hide Others"))
        .map_err(|e| format!("hide_others: {e}"))?;
    let show_all = PredefinedMenuItem::show_all(app, Some("Show All"))
        .map_err(|e| format!("show_all: {e}"))?;
    // Custom quit item — accelerator binds ⌘Q so macOS routes it here
    // before the system Quit fires. Click handler in `handle_menu_event`
    // calls `attempt_quit`, which gates on `RunningOps`.
    // `CmdOrCtrl+Q` so Linux/Windows builds keep their conventional
    // Ctrl+Q accelerator. macOS resolves CmdOrCtrl to ⌘.
    let quit = MenuItemBuilder::with_id("app-menu:quit", "Quit Claudepot")
        .accelerator("CmdOrCtrl+Q")
        .build(app)
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

    // ---- Edit submenu ------------------------------------------------------
    // Required on macOS: without the predefined copy/cut/paste/select_all
    // items in an Edit menu, WebKit `<input>` fields don't receive ⌘C/⌘V/
    // ⌘X/⌘A keyboard shortcuts. `app.set_menu()` replaces the default menu
    // entirely, so we have to put these back by hand.
    let undo = PredefinedMenuItem::undo(app, Some("Undo")).map_err(|e| format!("undo: {e}"))?;
    let redo = PredefinedMenuItem::redo(app, Some("Redo")).map_err(|e| format!("redo: {e}"))?;
    let cut = PredefinedMenuItem::cut(app, Some("Cut")).map_err(|e| format!("cut: {e}"))?;
    let copy = PredefinedMenuItem::copy(app, Some("Copy")).map_err(|e| format!("copy: {e}"))?;
    let paste = PredefinedMenuItem::paste(app, Some("Paste")).map_err(|e| format!("paste: {e}"))?;
    let select_all = PredefinedMenuItem::select_all(app, Some("Select All"))
        .map_err(|e| format!("select_all: {e}"))?;

    let edit = SubmenuBuilder::new(app, "Edit")
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
        MenuItemBuilder::with_id("app-menu:account:login-browser", "Sign in from browser…")
            .build(app)
            .map_err(|e| format!("login-browser: {e}"))?;
    let sync_cc = MenuItemBuilder::with_id("app-menu:account:sync-cc", "Sync from current CC")
        .build(app)
        .map_err(|e| format!("sync-cc: {e}"))?;
    let verify_all = MenuItemBuilder::with_id("app-menu:account:verify-all", "Verify all")
        .build(app)
        .map_err(|e| format!("verify-all: {e}"))?;

    let account = SubmenuBuilder::new(app, "Account")
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
    let nav_accounts = MenuItemBuilder::with_id("app-menu:nav:accounts", "Accounts")
        .build(app)
        .map_err(|e| format!("nav-accounts: {e}"))?;
    let nav_activities = MenuItemBuilder::with_id("app-menu:nav:events", "Activities")
        .build(app)
        .map_err(|e| format!("nav-activities: {e}"))?;
    let nav_projects = MenuItemBuilder::with_id("app-menu:nav:projects", "Projects")
        .build(app)
        .map_err(|e| format!("nav-projects: {e}"))?;
    let nav_keys = MenuItemBuilder::with_id("app-menu:nav:keys", "Keys")
        .build(app)
        .map_err(|e| format!("nav-keys: {e}"))?;
    let nav_third_party = MenuItemBuilder::with_id("app-menu:nav:third-party", "Third-parties")
        .build(app)
        .map_err(|e| format!("nav-third-party: {e}"))?;
    let nav_automations = MenuItemBuilder::with_id("app-menu:nav:automations", "Automations")
        .build(app)
        .map_err(|e| format!("nav-automations: {e}"))?;
    let nav_global = MenuItemBuilder::with_id("app-menu:nav:global", "Global")
        .build(app)
        .map_err(|e| format!("nav-global: {e}"))?;
    let nav_settings = MenuItemBuilder::with_id("app-menu:nav:settings", "Settings")
        .build(app)
        .map_err(|e| format!("nav-settings: {e}"))?;
    let toggle_theme = MenuItemBuilder::with_id("app-menu:view:toggle-theme", "Toggle theme")
        .build(app)
        .map_err(|e| format!("toggle-theme: {e}"))?;
    // Honest label: the handler in App.tsx only refreshes the Accounts
    // section. A section-aware refresh contract doesn't exist yet, so
    // don't promise one in the label.
    let reload = MenuItemBuilder::with_id("app-menu:view:reload", "Refresh Accounts")
        .build(app)
        .map_err(|e| format!("reload: {e}"))?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&nav_accounts)
        .item(&nav_activities)
        .item(&nav_projects)
        .item(&nav_keys)
        .item(&nav_third_party)
        .item(&nav_automations)
        .item(&nav_global)
        .item(&nav_settings)
        .separator()
        .item(&toggle_theme)
        .item(&reload)
        .build()
        .map_err(|e| format!("view submenu: {e}"))?;

    // ---- Window submenu (standard macOS set) -------------------------------
    let minimize = PredefinedMenuItem::minimize(app, Some("Minimize"))
        .map_err(|e| format!("minimize: {e}"))?;
    let maximize =
        PredefinedMenuItem::maximize(app, Some("Zoom")).map_err(|e| format!("maximize: {e}"))?;
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
        .build(app)
        .map_err(|e| format!("help-github: {e}"))?;
    let help_diag = MenuItemBuilder::with_id("app-menu:help:copy-diag", "Copy diagnostics")
        .build(app)
        .map_err(|e| format!("help-diag: {e}"))?;
    let help_data =
        MenuItemBuilder::with_id("app-menu:help:reveal-data-dir", "Reveal data dir in Finder")
            .build(app)
            .map_err(|e| format!("help-data: {e}"))?;

    let help = SubmenuBuilder::new(app, "Help")
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
    // ⌘Q (and the menubar Quit click) — gate on RunningOps. Returns
    // before the generic forward path so the renderer never sees a
    // stray `app-menu:quit` event.
    if id == "app-menu:quit" {
        attempt_quit(app);
        return;
    }

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
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
        return;
    }
    if id == "app-menu:help:github" {
        let url = "https://github.com/xiaolai/claudepot-app";
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(url).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn();
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
    use tauri_plugin_dialog::{
        DialogExt, MessageDialogButtons, MessageDialogKind,
    };

    let count = in_flight.len();
    let heading = if count == 1 {
        "1 operation in progress".to_string()
    } else {
        format!("{count} operations in progress")
    };
    let body = {
        let mut lines = String::from(
            "Quitting now will abandon the work below. \
             Repairable operations leave a journal entry you can resume \
             later; one-shot operations will need to be restarted.\n\n",
        );
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
            "Quit anyway".to_string(),
            "Stay".to_string(),
        ))
        .show(move |ok| {
            if ok {
                app_for_cb.exit(0);
            }
        });
}

/// One-line in-flight label for the quit modal. Mirrors the shape of
/// `op_terminal_label` in `ops.rs` but uses gerunds ("Renaming", not
/// "Renamed") since the op is still running.
fn inflight_label(op: &crate::ops::RunningOpInfo) -> String {
    let from = basename(&op.old_path);
    let to = basename(&op.new_path);
    match op.kind {
        OpKind::CleanProjects => "Cleaning projects".to_string(),
        OpKind::SessionPrune => "Pruning sessions".to_string(),
        OpKind::SessionSlim => {
            if from.is_empty() {
                "Slimming session".to_string()
            } else {
                format!("Slimming {from}")
            }
        }
        OpKind::SessionShare => "Sharing session".to_string(),
        OpKind::SessionMove => format!("Moving session {from} → {to}"),
        OpKind::MoveProject => format!("Renaming {from} → {to}"),
        OpKind::RepairResume => format!("Resuming {from} → {to}"),
        OpKind::RepairRollback => format!("Rolling back {from} → {to}"),
        OpKind::AccountLogin => "Account login".to_string(),
        OpKind::AccountRegister => "Adding account".to_string(),
        OpKind::VerifyAll => "Verifying accounts".to_string(),
        OpKind::AutomationRun => "Running automation".to_string(),
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
