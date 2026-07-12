mod app_menu;
mod cc_doctor_watcher;
// `pub` so integration tests in `tests/*.rs` can reach
// `commands::agents::route_lookup_fn` and drive the EXACT closure
// shape the Tauri command builds (grill X27 / audit A5). All public
// items inside this module were already reachable via the Tauri IPC
// surface; widening to `pub` only exposes them to external Rust
// callers, which is acceptable for the lib crate's test surface.
mod agent_event_orchestrator;
pub mod commands;
mod config_dto;
mod config_watch;
mod config_watch_types;
#[cfg(target_os = "macos")]
mod dock_icon;
mod dto;
mod dto_account;
mod dto_activity;
mod dto_activity_cards;
mod dto_agents;
mod dto_artifact_lifecycle;
mod dto_artifact_usage;
mod dto_cc_daemon;
mod dto_cc_doctor;
mod dto_cc_tips;
mod dto_desktop;
mod dto_env;
mod dto_keys;
mod dto_memory;
mod dto_migrate;
mod dto_permission;
mod dto_project;
mod dto_project_repair;
mod dto_rotation;
mod dto_routes;
mod dto_service_status;
mod dto_session;
mod dto_session_debug;
mod dto_session_move;
mod dto_session_prune;
mod dto_templates;
mod dto_updates;
mod dto_usage;
mod events;
mod live_activity_bridge;
mod memory_watch;
mod ops;
mod permission_orchestrator;
mod poller;
mod pr_orchestrator;
mod preferences;
mod rotation_orchestrator;
mod service_status_watcher;
mod state;
mod traffic_light;
mod tray;
mod tray_icons;
mod tray_menu;
mod updates_watcher;
mod usage_snapshot;
mod usage_watcher;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Diagnostic logging — stderr (for `pnpm tauri dev`) plus a
    // rolling daily file at `paths::log_dir()` (for "what happened
    // before that self-quit?" forensics on Dock-launched builds
    // where stderr goes nowhere). The `RollingFileAppender::builder`
    // pattern gives us two things `tracing_appender::rolling::daily`
    // does not: `max_log_files(7)` for built-in retention (replaces
    // the prior custom housekeeping pass) and `latest_symlink` so a
    // stable `claudepot.log` path always points at today's dated
    // file (which is what `claudepot logs --tail` and any external
    // `tail -f` rely on; the live file is `claudepot.log.YYYY-MM-DD`
    // because daily rotation always appends the date suffix).
    //
    // grill X5: the lib crate's name is `claudepot_tauri_lib` (see
    // `Cargo.toml` `[lib] name`), and EnvFilter directive matching
    // requires a `::` boundary — `claudepot_tauri` does NOT cover
    // `claudepot_tauri_lib::…`, so without an explicit
    // `claudepot_tauri_lib` directive every `tracing::warn!` /
    // `info!` from the orchestrators and sibling modules is silently
    // dropped. The seven `target = "claudepot_tauri"` overrides
    // scattered through this file were the existing workaround;
    // they remain in place (no harm) but new code in this crate
    // does not need them.
    let log_dir = claudepot_core::paths::log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let file_layer = match claudepot_core::diagnostic_logging::build_file_appender(&log_dir) {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            // The guard must outlive the process so the non-blocking
            // writer keeps draining. Drop = flush + close the queue
            // mid-run, which would truncate the very crash
            // diagnostics this exists to capture. `Box::leak` is the
            // standard pattern — single allocation, freed at process
            // exit.
            let _: &'static tracing_appender::non_blocking::WorkerGuard =
                Box::leak(Box::new(guard));
            Some(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .with_ansi(false),
            )
        }
        Err(e) => {
            // File logging is best-effort: a read-only data dir, a
            // permission issue, or a symlink-creation failure
            // shouldn't prevent the GUI from booting with
            // stderr-only logging. Emit a one-line warning to
            // stderr so a dev-mode launch still sees the cause.
            eprintln!("warning: file logging disabled: {e}");
            None
        }
    };
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let env_filter =
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(
                    "info,claudepot_core=info,claudepot_tauri=info,claudepot_tauri_lib=info",
                )
            });
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(file_layer)
            .try_init();
    }

    // Global panic hook — owned by claudepot_core so the CLI's
    // `--inject-panic` verification path exercises the SAME code as
    // production. Writes through the tracing pipeline AND
    // synchronously to `panic.log` with `sync_data`, with a
    // thread-local recursion guard. See
    // `claudepot_core::diagnostic_logging`.
    claudepot_core::diagnostic_logging::install_panic_hook(log_dir.clone());

    // Fatal-signal capture — the foreign-code aborts the panic hook
    // cannot see (AppKit assertions, Obj-C exceptions, an FFI SIGSEGV).
    // Appends a synchronous line to `crash.log`, then re-raises so the
    // OS still writes its own crash report. The v0.1.4x tray self-quits
    // were exactly this class (an AppKit `abort()`) and left the panic
    // hook silent. See `diagnostic_logging::install_signal_handler`.
    claudepot_core::diagnostic_logging::install_signal_handler(&log_dir);

    // Surface any prior macOS crash (`.ips`) in our own log dir so
    // "Reveal logs" / `claudepot logs` show self-quits without anyone
    // digging through ~/Library/Logs/DiagnosticReports. One-time
    // backfill on first run, then quiet until the next crash. See
    // `claudepot_core::crash_reports`.
    #[cfg(target_os = "macos")]
    if let Some(reports_dir) = claudepot_core::paths::diagnostic_reports_dir() {
        let crashes_log = log_dir.join("crashes.log");
        let state = log_dir.join(".crash-harvest-state");
        match claudepot_core::crash_reports::harvest(
            &reports_dir,
            "claudepot-tauri",
            &crashes_log,
            &state,
        ) {
            Ok(new) => {
                for s in &new {
                    tracing::error!(
                        target: "claudepot_crash",
                        file = %s.file_name,
                        signal = s.signal.as_deref().unwrap_or("?"),
                        exception = s.exc_type.as_deref().unwrap_or("?"),
                        thread = s.faulting_thread.as_deref().unwrap_or("?"),
                        top_frame = s.top_frame.as_deref().unwrap_or("?"),
                        "prior crash recorded by macOS (DiagnosticReports)"
                    );
                }
                if !new.is_empty() {
                    tracing::warn!(
                        "harvested {} prior crash report(s) into {}",
                        new.len(),
                        crashes_log.display()
                    );
                }
            }
            Err(e) => tracing::warn!("crash-report harvest failed: {e}"),
        }
    }

    // One-time: move the legacy `~/.claude/claudepot/` repair tree into
    // `~/.claudepot/repair/`. Idempotent and safe to run on every boot;
    // any error here is non-fatal — the app still works against whatever
    // layout is currently on disk.
    if let Err(e) = claudepot_core::migrations::migrate_repair_tree() {
        tracing::warn!("repair tree migration failed: {e}");
    }

    // Recover any orphan `Claude.app.bak-<ts>` siblings left over from
    // an interrupted Desktop update — a SIGKILL or hard reboot between
    // the `fs::rename(target, &backup)` and `ditto`-restore branch in
    // `install_via_zip` leaves Claude.app missing on disk with a usable
    // backup right next to it. Without this scan the user would see a
    // phantom "Desktop not installed" state in About / health / tray
    // until they manually rename the backup back. Idempotent and safe
    // to run on every boot — no-ops when nothing needs recovery.
    let _ = claudepot_core::updates::desktop_driver::recover_orphan_backups_at_startup();

    // Run the WAL housekeeping pass before any long-lived SQLite store
    // takes its connection. Truncates leftover `*.db-wal` files that
    // previous runs left behind — clean exits checkpoint cleanly, but
    // SIGKILL / force-quit / crashes / power loss bypass that, and
    // without `journal_size_limit` the WAL file high-water-marks even
    // after auto-checkpoint folds pages in. The 2026-05-24 incident saw
    // `sessions.db-wal` reach 6.3 GB this way. See
    // `crates/claudepot-core/src/db_housekeeping.rs`.
    let reclaimed = claudepot_core::db_housekeeping::checkpoint_known_db_files(
        &claudepot_core::paths::claudepot_data_dir(),
    );
    if reclaimed > 0 {
        tracing::info!(
            target = "claudepot_tauri",
            bytes = reclaimed,
            "startup WAL checkpoint reclaimed bytes"
        );
    }

    // Load persisted preferences BEFORE the builder constructs anything
    // that might need them. `hide_dock_icon` in particular must reach
    // `set_activation_policy()` inside the very first `setup()` tick to
    // avoid a visible dock-icon flash on cold launch.
    let prefs = preferences::Preferences::load();
    // Load update settings + cached probe state alongside prefs so the
    // first `updates_status_get` call from the webview is a pure clone
    // off the mutex (no disk I/O on the hot UI render path).
    let updates_state = claudepot_core::updates::UpdateState::load();
    // `hide_dock` is only consumed inside `#[cfg(target_os = "macos")]`
    // (set_activation_policy is a no-op everywhere else), so binding it
    // unconditionally would emit `unused_variable` on Linux + Windows.
    // Gate the bind to keep the production build clean while still
    // honouring the cold-load order — `prefs` itself is read on every
    // platform; only the macOS-specific extraction is gated.
    #[cfg(target_os = "macos")]
    let hide_dock = prefs.hide_dock_icon;
    let show_window_on_startup = prefs.show_window_on_startup;

    // Open the persistent notification log before the builder chain
    // so its handle can be `.manage()`d alongside the other state. The
    // log lives at `~/.claudepot/notifications.json`; `open` returns
    // an empty log on missing or corrupt files (the corrupt file gets
    // moved aside for forensics) so this never blocks startup.
    let notification_log_path = claudepot_core::notification_log::default_path();
    let notification_log_state = match claudepot_core::notification_log::NotificationLog::open(
        notification_log_path.clone(),
    ) {
        Ok(log) => commands::notification::NotificationLogState::new(log),
        Err(e) => {
            tracing::warn!(
                target = "claudepot_tauri",
                error = %e,
                path = %notification_log_path.display(),
                "notification log open failed — appends will be no-ops, surface stays empty"
            );
            // Two-step fallback: first try a temp-dir path so at
            // least the current process gets persistence inside the
            // session. If even that fails (no temp dir, permissions,
            // disk full), drop to a fully in-memory log so the bell
            // still works for this run instead of panicking startup.
            // The previous version `.expect()`d on the temp-path
            // open and aborted the whole app on a deeper environment
            // issue — strictly worse than degraded notifications.
            let fallback = std::env::temp_dir().join("claudepot-notifications.json");
            let log = claudepot_core::notification_log::NotificationLog::open(fallback.clone())
                .unwrap_or_else(|e2| {
                    tracing::warn!(
                        target = "claudepot_tauri",
                        error = %e2,
                        path = %fallback.display(),
                        "notification log temp-dir fallback also failed — using volatile in-memory log; entries will not survive restart"
                    );
                    claudepot_core::notification_log::NotificationLog::in_memory_only()
                });
            commands::notification::NotificationLogState::new(log)
        }
    };

    // Open the activity-cards index before the builder chain so we can
    // wire it into the live runtime AND publish it as IPC state in the
    // same `.manage()` series. Lives in the same `sessions.db` file as
    // SessionIndex (SQLite WAL lets the two handles coexist). Open
    // failure (disk full, perms, corrupt) degrades gracefully — the
    // cards surface goes dark; the live-strip and everything else
    // continues to work.
    let cards_db_path = claudepot_core::paths::claudepot_data_dir().join("sessions.db");
    let cards_index: Option<std::sync::Arc<claudepot_core::activity::ActivityIndex>> =
        match claudepot_core::activity::ActivityIndex::open(&cards_db_path) {
            Ok(idx) => {
                // Prune cards older than the retention window once
                // per startup so the table stays bounded across
                // months of sustained use. The previous shape only
                // pruned per-session (and only when the source
                // `.jsonl` was gone), leaving cards from long-
                // finished sessions accumulating forever. The
                // retention window lives in
                // `claudepot_core::retention` so the sibling
                // metrics_tick prune in `LiveRuntime::start` can't
                // drift out of sync.
                let cutoff_ms =
                    chrono::Utc::now().timestamp_millis() - claudepot_core::retention::RETENTION_MS;
                match idx.prune_before(cutoff_ms) {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(
                        target = "claudepot_tauri",
                        pruned = n,
                        cutoff_ms,
                        "activity-cards startup prune"
                    ),
                    Err(e) => tracing::warn!(
                        target = "claudepot_tauri",
                        error = %e,
                        "activity-cards startup prune failed"
                    ),
                }
                Some(std::sync::Arc::new(idx))
            }
            Err(e) => {
                tracing::warn!(
                    target = "claudepot_tauri",
                    error = %e,
                    path = %cards_db_path.display(),
                    "activity-cards index open failed — cards surface degraded"
                );
                None
            }
        };

    // Shared `SessionIndex` for the Shared Memory commands. Opens
    // the same `sessions.db`. SessionIndex owns a `Mutex<Connection>`
    // internally so writes serialize; sharing one handle across all
    // Shared Memory commands avoids the per-call open-and-migrate
    // contention that surfaced as `SQLITE_BUSY` ("database is
    // locked") in the dev pane. Open failure degrades the Shared
    // Memory section to a "session index unavailable" banner; the
    // rest of the app keeps working.
    let shared_memory_index: Option<std::sync::Arc<claudepot_core::session_index::SessionIndex>> =
        match claudepot_core::session_index::SessionIndex::open(&cards_db_path) {
            Ok(idx) => Some(std::sync::Arc::new(idx)),
            Err(e) => {
                tracing::warn!(
                    target = "claudepot_tauri",
                    error = %e,
                    path = %cards_db_path.display(),
                    "shared-memory session index open failed — Shared Memory section degraded"
                );
                None
            }
        };

    // Clone the shared index handle for the startup search-index
    // backfill spawned inside `.setup` below; the original stays
    // available for `.manage()` further down.
    let search_index_for_backfill = shared_memory_index.clone();

    // (Live state is built inside the `.manage` chain below — the
    // service-refactor pattern owns the runtime, so we hand the
    // cards index into the service's `enable_activity` accessor at
    // construction time.)

    // Memory change-log database. Lives at
    // `~/.claudepot/memory_changes.db`. Two-step fallback mirrors the
    // notification-log pattern: try the canonical path; if that fails,
    // try a temp-dir copy so the current process still has a working
    // log; only if BOTH fail do we panic (extremely unlikely — both
    // home and temp would have to be unwritable).
    //
    // Audit 2026-05 #2: every memory IPC command requires
    // `MemoryLogState`, so this MUST always succeed. The previous
    // `Option<Arc<MemoryLog>>` shape silently broke the entire pane on
    // open failure because `.manage()` was gated on the option.
    let memory_log: std::sync::Arc<claudepot_core::memory_log::MemoryLog> = {
        let primary = claudepot_core::paths::claudepot_data_dir().join("memory_changes.db");
        match claudepot_core::memory_log::MemoryLog::open(&primary) {
            Ok(l) => std::sync::Arc::new(l),
            Err(e) => {
                tracing::warn!(
                    target = "claudepot_tauri",
                    error = %e,
                    path = %primary.display(),
                    "memory change-log open failed at canonical path; falling back to temp dir"
                );
                let fallback = std::env::temp_dir().join("claudepot-memory_changes.db");
                std::sync::Arc::new(
                    claudepot_core::memory_log::MemoryLog::open(&fallback).unwrap_or_else(|e2| {
                        // Both writable locations failed — this is
                        // catastrophic for log persistence but the
                        // app can still run. Panic with a clear
                        // message so users see the underlying issue
                        // rather than a generic "command failed".
                        panic!(
                            "memory change-log unrecoverable: primary={} ({e}); \
                                 fallback={} ({e2})",
                            primary.display(),
                            fallback.display(),
                        );
                    }),
                )
            }
        }
    };

    // Clone the memory-log handle for the setup closure so it can spawn
    // the watcher; the original Arc stays available for `.manage()`.
    let memory_log_for_watcher: std::sync::Arc<claudepot_core::memory_log::MemoryLog> =
        memory_log.clone();

    // Open the rotation audit log alongside the other persistent stores
    // so the orchestrator can record every swap attempt at boot. Same
    // boot-fallback story as the notification log: missing → empty;
    // corrupt → renamed aside; canonical-path-fails → in-memory-only.
    let rotation_audit: std::sync::Arc<claudepot_core::rotation::RotationAuditLog> =
        std::sync::Arc::new(claudepot_core::rotation::RotationAuditLog::open_default());
    let rotation_orchestrator = std::sync::Arc::new(
        rotation_orchestrator::RotationOrchestrator::new(rotation_audit.clone()),
    );

    // `mut` is only consumed by the debug-only plugin block below;
    // release builds don't touch it. Silence the release warning here.
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut builder = tauri::Builder::default()
        // MUST be the first plugin. On a second launch the new process
        // forwards its argv to the already-running instance and exits;
        // the callback raises the existing main window instead of
        // spawning a duplicate. Without this, a dev build, `open -n`,
        // or a direct binary run (which bypass LaunchServices'
        // one-launch-per-bundle rule) could run a second
        // com.claudepot.app alongside the installed one.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        // Auto-update pipeline. The plugin reads endpoints + pubkey from
        // tauri.conf.json `plugins.updater`; `latest.json` lives as a
        // release asset on this repo, so the URL self-tracks the newest
        // published release. Dev builds: the plugin loads but `check()`
        // fails fast (no signed `latest.json` for unreleased versions),
        // and the renderer surfaces that as an error toast — no crash.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // `process::relaunch` is invoked after the user clicks
        // "Restart to update" — the renderer runs it once the updater
        // has staged the new bundle. Capability gates the IPC.
        .plugin(tauri_plugin_process::init())
        // D-5/6/7: Rust-side clipboard write for `key_*_copy`. Permissions
        // restricted in `capabilities/default.json` to write/read/clear —
        // the renderer never invokes the plugin directly (its only
        // consumer is our own `commands_keys.rs`), but the read-text
        // permission is required so the 30s self-clear can verify the
        // clipboard still holds our payload before clobbering it.
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            use tauri::{
                image::Image,
                menu::MenuEvent,
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
                Listener, Manager,
            };

            // First priority: apply activation policy before the window
            // is materialized. On macOS this hides the dock icon and
            // removes the app from Cmd+Tab.
            #[cfg(target_os = "macos")]
            if hide_dock {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // Force the Dock icon through Cocoa's NSImage pipeline
            // with our 512×512 source so non-128 Dock sizes (the
            // default 96-px render in particular) get high-quality
            // Lanczos downsampling instead of the legacy IconServices
            // bilinear-from-128-layer path. See `dock_icon.rs`.
            #[cfg(target_os = "macos")]
            dock_icon::override_application_icon();

            // Honor the "show window on startup" preference. The window
            // is configured `visible: true` in tauri.conf.json so the
            // default cold-launch path stays unchanged; users who opt
            // out get an immediate `hide()` here. Recovery path is the
            // tray icon (left-click toggles visibility).
            //
            // While we have the window handle, install the close-button
            // intercept: red ✕ → hide window, NOT exit. The app is
            // tray-resident — see `app_menu.rs` for the same policy on
            // ⌘Q and ⌘W. The only surface that actually terminates
            // the process is the tray dropdown's Quit (gated by
            // RunningOps in `attempt_quit`). Without `prevent_close`
            // here, Tauri tears down the only window and macOS exits
            // the app, breaking every background watcher.
            if let Some(window) = app.get_webview_window("main") {
                if !show_window_on_startup {
                    let _ = window.hide();
                }

                // On Windows, set the WebView2 background color to match
                // the OS dark/light theme so DWM repaints — which happen
                // before WebView2 can repaint — show the same color as
                // the page content instead of flashing white or the wrong
                // theme color. Without this, a hardcoded static color in
                // tauri.conf.json always mismatches one of the two themes.
                //
                // The registry value AppsUseLightTheme lives at:
                //   HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize
                // 0x0 = dark mode, 0x1 = light mode.
                // Colors are approximate sRGB equivalents of the CSS tokens:
                //   light --bg: oklch(99% 0.003 60) ≈ rgb(253, 248, 243)
                //   dark  --bg: oklch(16% 0.006 60) ≈ rgb( 40,  37,  32)
                #[cfg(target_os = "windows")]
                {
                    let prefers_dark = {
                        use claudepot_core::proc_utils::NoWindowExt;
                        std::process::Command::new("reg")
                            .args([
                                "query",
                                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                                "/v",
                                "AppsUseLightTheme",
                            ])
                            .no_window()
                            .output()
                    }
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| {
                            let s = String::from_utf8_lossy(&o.stdout);
                            // Line format: "    AppsUseLightTheme    REG_DWORD    0x1"
                            // 0x0 → dark, 0x1 → light
                            !s.contains("0x1")
                        })
                        .unwrap_or(true); // default: assume dark if query fails

                    let bg = if prefers_dark {
                        tauri::utils::config::Color(40, 37, 32, 255)    // dark --bg
                    } else {
                        tauri::utils::config::Color(253, 248, 243, 255) // light --bg
                    };
                    let _ = window.set_background_color(Some(bg));
                    tracing::info!(
                        target = "claudepot_tauri",
                        dark = prefers_dark,
                        "Windows background color set to match OS theme"
                    );
                }

                let win_for_handler = window.clone();
                let win_for_tl = window.clone();
                window.on_window_event(move |event| {
                    use tauri::WindowEvent;
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Err(e) = win_for_handler.hide() {
                            tracing::warn!("close-button: hide failed: {e}");
                        }
                    }
                    // Re-emit traffic-light metrics whenever AppKit
                    // could have moved the buttons. ScaleFactorChanged
                    // covers a monitor swap; Focused covers the
                    // bring-to-front path where the lights re-tint
                    // and AppKit re-lays-out the standard window
                    // buttons.
                    if matches!(
                        event,
                        WindowEvent::Resized(_)
                            | WindowEvent::Moved(_)
                            | WindowEvent::Focused(true)
                            | WindowEvent::ScaleFactorChanged { .. }
                    ) {
                        traffic_light::emit(&win_for_tl);
                    }
                });

                // Two-stage initial emit: the first runs after the
                // renderer has surely mounted its listener, the
                // second after AppKit's first-paint shuffle of the
                // standard window buttons has settled. The renderer
                // also pulls once via `traffic_light_metrics` IPC at
                // mount time as a belt-and-suspenders for the case
                // where both emits beat the listener.
                //
                // Each emit is routed through `run_on_main_thread`
                // because `traffic_light::emit` calls into AppKit
                // (`NSWindow.standardWindowButton`, `convertRect:toView:`),
                // which asserts the main thread and crashes the process
                // when invoked from a tokio worker. The previous shape
                // called `emit` directly from the spawned task — a
                // latent crash that fired probabilistically on cold
                // launch depending on main-thread load.
                let win_for_boot = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    traffic_light::emit_on_main_thread(&win_for_boot);
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    traffic_light::emit_on_main_thread(&win_for_boot);
                });
            }

            // Application menu bar (macOS top-of-screen; Windows/Linux
            // per-window). Accessory-mode apps on macOS don't render a
            // menu bar regardless, but Setting it is still safe.
            if let Err(e) = app_menu::install(app.handle()) {
                tracing::warn!("app menu install failed: {e}");
            }

            // Seed the menu-glyph appearance flag and rebuild the tray
            // when the system appearance flips, so dropdown icons swap
            // to the readable stroke variant (see tray_icons.rs —
            // muda never template-tints custom menu bitmaps).
            tray_icons::install_menu_appearance_watcher(app.handle());

            // macOS: pure-black Template asset, tinted by AppKit
            // against the menubar in Light and Dark. Windows/Linux:
            // no template tinting exists (`icon_as_template` is a
            // macOS-only no-op), so the black bitmap would vanish on
            // dark taskbars/panels — ship the mid-gray Mono variant
            // instead. Mirrors the TRAY_IDLE/ALERT selection in
            // `tray::apply_tray_icon`.
            #[cfg(target_os = "macos")]
            let icon_bytes = include_bytes!("../icons/tray-iconTemplate@2x.png");
            #[cfg(not(target_os = "macos"))]
            let icon_bytes = include_bytes!("../icons/tray-iconMono@2x.png");
            let icon = Image::from_bytes(icon_bytes)?;

            TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .tooltip("Claudepot")
                // Left-click toggles the window directly; the menu is
                // right-click only. Standard pattern for productivity
                // menubar apps (Slack, Tailscale admin) — the menu's
                // job is quick switching, the icon's job is "open".
                // Linux: unsupported by Tauri, falls through as no-op.
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let visible = window.is_visible().unwrap_or(false);
                            let focused = window.is_focused().unwrap_or(false);
                            if visible && focused {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .on_menu_event(|app, event: MenuEvent| {
                    let id = event.id().as_ref();
                    if id.starts_with("app-menu:") {
                        app_menu::handle_menu_event(app, id);
                    } else {
                        tray::handle_menu_event(app, id);
                    }
                })
                .build(app)?;

            // Build initial tray menu from current accounts. Rebuild
            // is async (it peeks the UsageCache for the new Usage
            // submenu); spawn so setup returns immediately.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = tray::rebuild(&handle).await {
                    tracing::warn!("initial tray rebuild failed: {e}");
                }
            });

            // One-shot startup reconcile (B-2). Pre-B-2 the Tauri
            // `account_list` command opportunistically rewrote
            // `has_cli_credentials` and `has_desktop_profile` on every
            // poll, which raced two GUI sections that both polled.
            // Now: a single best-effort pass at startup catches the
            // common "user mutated state out-of-band" case, and the
            // explicit `accounts_reconcile` Tauri command covers the
            // user-driven path. Failure here never blocks startup.
            // `tauri::async_runtime::spawn` (not `spawn_blocking`): the
            // reconcile path is async because `swap::load_private` is
            // async, and the underlying keychain subprocess calls have
            // their own bounded 5 s timeout in `cli_backend::storage`.
            tauri::async_runtime::spawn(async {
                let store = match crate::commands::open_store() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("startup reconcile: open_store failed: {e}");
                        return;
                    }
                };
                match claudepot_core::services::account_service::reconcile_all(&store).await {
                    Ok(report) => {
                        if !report.cli_flips.is_empty()
                            || !report.desktop.flag_flips.is_empty()
                            || report.desktop.orphan_pointer_cleared
                        {
                            tracing::info!(
                                cli_flipped = report.cli_flips.len(),
                                desktop_flipped = report.desktop.flag_flips.len(),
                                orphan_pointer_cleared = report.desktop.orphan_pointer_cleared,
                                "startup reconcile applied"
                            );
                        }
                    }
                    Err(e) => tracing::warn!("startup reconcile failed: {e}"),
                }
            });

            // Listen for frontend requests to rebuild the tray menu
            let handle2 = app.handle().clone();
            app.listen("rebuild-tray-menu", move |_| {
                let handle = handle2.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = tray::rebuild(&handle).await {
                        tracing::warn!("rebuild-tray-menu failed: {e}");
                    }
                });
            });

            // Register the Tauri-side listener with the
            // `LiveActivityService`. This is the only place that
            // knows how to translate service events
            // (`on_aggregate`, `on_membership_changed`, `on_detail`)
            // into webview emits + tray rebuilds. The service
            // itself is framework-free.
            let live_state = app.state::<state::LiveSessionState>();
            let service = std::sync::Arc::clone(&live_state.service);
            let listener =
                live_activity_bridge::TauriSessionEventListener::new(app.handle().clone());
            tauri::async_runtime::spawn(async move {
                service.subscribe(listener).await;
            });

            // Usage-threshold watcher: every 5 min, polls /usage for the
            // CLI-active account and emits `usage-threshold-crossed`
            // events when utilization first crosses each configured
            // threshold per cycle. Pure detector + persistence live in
            // `claudepot_core::services::usage_alerts`; this is just
            // the orchestration. No-op when the threshold list is empty
            // or activity is disabled — guarded inside `run_tick`.
            //
            // The watcher reaches the shared `UsageCache` via
            // `app.state::<UsageCache>()` inside each tick, so it
            // consumes the same cache the rest of the app uses without
            // forcing an Arc<UsageCache> at the manage() site (which
            // would break every `State<'_, UsageCache>` consumer).
            usage_watcher::spawn(app.handle().clone());

            // Periodic writer for `~/.claudepot/usage-snapshot.json`.
            // Lets non-GUI processes (cron, CC bash subprocesses,
            // bots) read per-account utilization without keychain
            // access. Same 5-min cadence as `usage_watcher`; shares
            // the in-memory `UsageCache`. See
            // `src-tauri/src/usage_snapshot.rs`.
            usage_snapshot::spawn(app.handle().clone());

            // Boot-time agent reconciliation (grill findings F15 +
            // X9). Two directions:
            //
            // 1. `reconcile_with_scheduler` (F15): every
            //    `Installed` agent must have a live scheduler
            //    artifact. Loudly logs any record claiming
            //    `Installed` whose artifact is missing — catches a
            //    hand-edited lifecycle field or an install rollback
            //    that could not re-save.
            // 2. `reconcile_orphan_artifacts_now` (X9): every
            //    Claudepot-managed scheduler artifact must have a
            //    matching `Installed` agent record. Loudly logs any
            //    artifact with no record behind it — catches
            //    `agents.json` hand-edits or third-process artifact
            //    writes that leave a `claude -p` firing on schedule
            //    with no visible record. Observability only — never
            //    removes the artifact, matching F15's conservative
            //    policy.
            //
            // Both are best-effort and run on a detached blocking
            // task so they never block setup. The reverse-direction
            // check (X15) replaces the per-tick reconcile that was
            // previously running every 5 min in the event
            // orchestrator.
            tauri::async_runtime::spawn_blocking(|| {
                claudepot_core::agent::reconcile_with_scheduler();
                claudepot_core::agent::reconcile_orphan_artifacts_now();
            });

            // Background poller for CC CLI + Claude Desktop updates.
            // Probes upstream every `poll_interval_minutes` (default
            // 4 h), updates the tray badge, and runs the auto-install
            // pass when the user has opted in. See
            // `src-tauri/src/updates_watcher.rs`.
            updates_watcher::spawn(app.handle().clone());

            // Background poller for `status.claude.com`. 5 min cadence,
            // gated by the `service_status.poll_status_page` preference.
            // Surfaces transitions through the existing notification log.
            // See `src-tauri/src/service_status_watcher.rs` and
            // `dev-docs/network-status.md`.
            service_status_watcher::spawn(app.handle().clone());

            // Long-running fs-watcher for memory file changes. Watches
            // `~/.claude/` recursively; records diffs to
            // `~/.claudepot/memory_changes.db` and emits `memory:changed`
            // for the MemoryPane.
            memory_watch::spawn(app.handle().clone(), memory_log_for_watcher.clone());

            // Background `claude doctor` scrape on a 5 min cadence so
            // the tray Health row stays current when the window is
            // closed. See `cc_doctor_watcher.rs` for cadence rationale.
            cc_doctor_watcher::spawn(app.handle().clone());

            // Keep the exchange FTS index — which backs cross-session (⌘K
            // palette) search — converged with disk.
            //
            // `session_index::refresh` writes the `sessions` staleness
            // tuples; `backfill_claude_exchanges` then fills `exchanges` +
            // `exchange_fts`. Nothing else runs this, so without it a
            // GUI-only user's palette search falls back to the slow
            // full-JSONL scan on every query.
            //
            // It LOOPS rather than running once at boot. A transcript grows
            // for as long as its session lasts, and a one-shot backfill
            // would leave everything said after launch out of the index
            // until the next restart — i.e. exactly the content the user is
            // most likely to search for. Re-running converges it instead.
            // `search_cross_session` never pays for this: a pass whose files
            // are all unchanged is a stat-walk plus a skip, and the
            // `exchange_state` guard means only genuinely-changed
            // transcripts are re-parsed.
            //
            // Detached blocking task; a failed index open (`None`) is a
            // no-op.
            if let Some(idx) = search_index_for_backfill {
                tauri::async_runtime::spawn_blocking(move || {
                    /// Long enough that an idle app is doing nothing
                    /// measurable; short enough that "find what I said a few
                    /// minutes ago" works without a restart.
                    const BACKFILL_EVERY: std::time::Duration =
                        std::time::Duration::from_secs(120);

                    loop {
                        let cfg = claudepot_core::paths::claude_config_dir();
                        if let Err(e) = idx.refresh(&cfg) {
                            tracing::warn!(
                                target = "claudepot_tauri",
                                error = %e,
                                "search index: session refresh failed; palette search uses JSONL scan"
                            );
                        } else {
                            match claudepot_core::shared_memory::claude_exchanges::backfill_claude_exchanges(
                                &idx, &cfg,
                            ) {
                                // Per-file failures leave those transcripts
                                // out of the FTS index. NOT fatal — search
                                // still finds them, because
                                // `search_cross_session` scans every session
                                // the index doesn't cover — but it silently
                                // costs speed, so say so out loud rather than
                                // reporting a clean "complete".
                                Ok(stats) if !stats.failed.is_empty() => tracing::warn!(
                                    target = "claudepot_tauri",
                                    discovered = stats.discovered,
                                    indexed = stats.indexed,
                                    skipped = stats.skipped_unchanged,
                                    failed = stats.failed.len(),
                                    first_error = %stats.failed[0].1,
                                    first_path = %stats.failed[0].0.display(),
                                    "search index: exchange backfill finished WITH FAILURES — \
                                     those transcripts fall back to the slow scan path"
                                ),
                                Ok(stats) if stats.indexed > 0 => tracing::info!(
                                    target = "claudepot_tauri",
                                    discovered = stats.discovered,
                                    indexed = stats.indexed,
                                    skipped = stats.skipped_unchanged,
                                    "search index: exchange backfill complete"
                                ),
                                // Steady state: nothing changed. Don't narrate.
                                Ok(_) => {}
                                Err(e) => tracing::warn!(
                                    target = "claudepot_tauri",
                                    error = %e,
                                    "search index: exchange backfill failed"
                                ),
                            }
                        }
                        std::thread::sleep(BACKFILL_EVERY);
                    }
                });
            }

            Ok(())
        })
        .manage(state::LoginState::default())
        .manage(state::DesktopOpState::default())
        .manage(state::CliOpState::default())
        .manage(state::DryRunState::new())
        .manage({
            // Build the live state with the service refactor's
            // pattern, then enable activity-cards classification on
            // the inner runtime when the cards index opened
            // successfully. The setup hook (which subscribes the
            // bridge listener) reads the same `LiveSessionState`
            // afterwards and sees both the service AND the wired
            // activity index.
            let svc = claudepot_core::services::live_activity_service::LiveActivityService::new();
            if let Some(idx) = cards_index.as_ref() {
                svc.enable_activity(std::sync::Arc::clone(idx));
            }
            state::LiveSessionState::new(svc)
        })
        .manage(ops::RunningOps::new())
        .manage(commands::cc_doctor::CcDoctorState::new())
        .manage(state::TrayHealthState::default())
        .manage(state::TrayAlertState::default())
        .manage(state::UpdatesAlertState::default())
        .manage(preferences::PreferencesState::new(prefs))
        .manage(claudepot_core::updates::UpdateStateMutex::new(
            updates_state,
        ))
        // Channel-aware self-updater. Holds the most recent Rust
        // `Update` handle returned by `release_update_check` so
        // `release_update_install` can act on the same object — the
        // handle is bound to the channel endpoints it was checked
        // against and can't be reconstructed from a DTO. See
        // `commands::release_update`.
        .manage(commands::release_update::ReleaseUpdateState::new())
        // Single shared gate: serializes background poller, manual
        // check, and manual install across the whole process. See
        // `updates_watcher::tick` and `commands_updates`.
        .manage(std::sync::Arc::new(
            claudepot_core::updates::PollerGate::default(),
        ))
        // D-1: shared config-scan cache + commit race arbiter. Owns
        // the latest scanned `ConfigTree`, hands out generation tokens,
        // and arbitrates between concurrent writers (`config_scan`
        // command + watcher seed/keepalive). Was `ConfigTreeState` in
        // the IPC layer — service moved to `claudepot-core` so the
        // commit policy is testable without Tauri.
        .manage(claudepot_core::config_view::ConfigScanService::new())
        .manage(commands::config::SearchRegistry::default())
        .manage(config_watch::ConfigWatchState::default())
        .manage(claudepot_core::services::usage_cache::UsageCache::new())
        // D-3: process-wide pricing cache + singleflight refresh.
        // Replaces the old `static REFRESH_IN_FLIGHT: AtomicBool` in
        // `commands_pricing.rs`, and now correctly singleflights the
        // `pricing_refresh` button-mash path too.
        .manage(claudepot_core::pricing::PricingCacheService::new())
        .manage(notification_log_state)
        .manage(commands::service_status::ServiceStatusState::new())
        .manage(rotation_orchestrator)
        // Event orchestrator state — a `Mutex<HashSet<AgentId>>` of
        // agent ids the orchestrator has already seen in at least
        // one tick this process (grill X16 replaced the earlier
        // single-boolean "booted" flag). `mark_seen` does a HashMap
        // probe per agent per tick — small, but unconditional once
        // any event-triggered agent exists. The orchestrator's
        // tick still early-returns when no `Installed && enabled`
        // event-triggered agents are present, so it remains
        // free-with-no-agents; the cost only appears once a real
        // event agent is in the store.
        .manage(std::sync::Arc::new(
            agent_event_orchestrator::EventOrchestrator::new(),
        ))
        // Per-project PR detection cache + tick. Zero overhead until
        // the orchestrator's `tick_all` is called from
        // `usage_snapshot::run_tick`; `project_list` reads the cache
        // synchronously to decorate `ProjectInfoDto.pr`. Shared as
        // an `Arc<PrOrchestrator>` so the tick and the read path see
        // the same cache instance.
        .manage(std::sync::Arc::new(pr_orchestrator::PrOrchestrator::new()));

    // Conditionally publish the cards index — `None` means open
    // failed at startup, in which case the cards-* commands return
    // `Tauri State unavailable` errors that the JS side surfaces as
    // a "cards index not ready" toast. The live-strip surface and
    // every other command keeps working.
    if let Some(idx) = cards_index {
        builder = builder.manage(commands::activity_cards::ActivityCardsState { index: idx });
    }

    // Shared SessionIndex for the Shared Memory commands. Always
    // managed (even when `None`); the wrapper carries the Option so
    // commands return a graceful error rather than panicking on a
    // missing state.
    builder = builder.manage(commands::shared_memory::SharedMemoryIndex(
        shared_memory_index,
    ));

    // Memory change-log state. Always managed — `memory_log` is now
    // unconditionally `Arc<MemoryLog>` per audit 2026-05 #2.
    builder = builder.manage(commands::memory::MemoryLogState::new(memory_log.clone()));

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }

    builder
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::updater_supported,
            commands::quit_now,
            commands::tray_set_alert_count,
            traffic_light::traffic_light_metrics,
            commands::cli::sync_from_current_cc,
            commands::unlock_keychain,
            commands::reveal_in_finder,
            commands::logs_dir_reveal,
            commands::account_list,
            commands::account_list_basic,
            commands::accounts_reconcile,
            commands::cli::cli_use,
            commands::cli::cli_is_cc_running,
            commands::desktop::desktop_use,
            commands::desktop::current_desktop_identity,
            commands::desktop::verified_desktop_identity,
            commands::desktop::desktop_adopt,
            commands::desktop::desktop_clear,
            commands::desktop::sync_from_current_desktop,
            commands::desktop::desktop_launch,
            commands::account::account_add_from_current,
            commands::account::account_register_from_browser,
            commands::account::account_register_from_browser_start,
            commands::account::account_login,
            commands::account::account_login_start,
            commands::account::account_login_status,
            commands::account::account_login_cancel,
            commands::account::account_remove,
            commands::account::fetch_all_usage,
            commands::account::refresh_usage_for,
            commands::account::verify_all_accounts,
            commands::account::verify_all_accounts_start,
            commands::account::verify_all_accounts_status,
            commands::account::verify_account,
            commands::cli::current_cc_identity,
            commands::project::project_list,
            commands::project::project_show,
            commands::project::project_move_dry_run,
            commands::project::project_clean_preview,
            commands::project::project_clean_start,
            commands::project::project_clean_status,
            commands::project::project_remove_preview,
            commands::project::project_remove_preview_basic,
            commands::project::project_remove_preview_extras,
            commands::project::project_remove_execute,
            commands::project::project_trash_list,
            commands::project::project_trash_restore,
            commands::project::project_trash_empty,
            commands::project::repair_list,
            commands::project::repair_pending_count,
            commands::repair::repair_resume_start,
            commands::repair::repair_rollback_start,
            commands::repair::repair_abandon,
            commands::repair::repair_break_lock,
            commands::repair::repair_gc,
            commands::repair::repair_preview_abandoned,
            commands::repair::repair_cleanup_abandoned,
            commands::repair::running_ops_list,
            commands::repair::project_move_start,
            commands::repair::project_move_status,
            commands::project::repair_status_summary,
            commands::session_move::session_list_orphans,
            commands::session_move::session_move,
            commands::session_move::session_move_start,
            commands::session_move::session_move_status,
            commands::session_move::session_adopt_orphan,
            commands::session_move::session_discard_orphan,
            commands::session_index::session_list_all,
            commands::session_index::session_list_by_slug,
            commands::session_index::session_read,
            commands::session_index::session_read_path,
            commands::session_index::session_index_rebuild,
            commands::session_index::session_chunks,
            commands::session_index::session_context_attribution,
            commands::session_index::session_export_to_file,
            commands::session_index::session_search,
            commands::session_index::session_worktree_groups,
            commands::artifact_usage::artifact_usage_for,
            commands::artifact_usage::artifact_usage_batch,
            commands::artifact_usage::artifact_usage_top,
            commands::artifact_lifecycle::artifact_classify_path,
            commands::artifact_lifecycle::artifact_disable,
            commands::artifact_lifecycle::artifact_enable,
            commands::artifact_lifecycle::artifact_list_disabled,
            commands::artifact_lifecycle::artifact_trash,
            commands::artifact_lifecycle::artifact_list_trash,
            commands::artifact_lifecycle::artifact_restore_from_trash,
            commands::artifact_lifecycle::artifact_recover_trash,
            commands::artifact_lifecycle::artifact_forget_trash,
            commands::artifact_lifecycle::artifact_purge_trash,
            commands::artifact_lifecycle::artifact_disabled_preview,
            commands::protected::protected_paths_list,
            commands::protected::protected_paths_add,
            commands::protected::protected_paths_remove,
            commands::protected::protected_paths_reset,
            commands::preferences::preferences_get,
            commands::preferences::preferences_category_prefs_get,
            commands::preferences::preferences_category_pref_set,
            commands::preferences::preferences_set_hide_dock_icon,
            commands::preferences::preferences_set_show_window_on_startup,
            commands::keys::key_api_list,
            commands::keys::key_api_add,
            commands::keys::key_api_remove,
            commands::keys::key_api_rename,
            commands::keys::key_api_copy,
            commands::keys::key_api_probe,
            commands::keys::key_oauth_list,
            commands::keys::key_oauth_add,
            commands::keys::key_oauth_remove,
            commands::keys::key_oauth_rename,
            commands::keys::key_oauth_copy,
            commands::keys::key_oauth_copy_shell,
            commands::keys::key_oauth_usage_cached,
            commands::preferences::preferences_set_activity,
            commands::preferences::preferences_set_notifications,
            commands::preferences::preferences_set_service_status,
            commands::service_status::service_status_summary_get,
            commands::service_status::service_status_probe_now,
            commands::service_status::service_status_latency_get,
            commands::service_status::network_first_run_check,
            commands::activity::session_live_start,
            commands::activity::session_live_stop,
            commands::activity::session_live_snapshot,
            commands::activity::session_live_session_snapshot,
            commands::activity::session_live_subscribe,
            commands::activity::session_live_unsubscribe,
            commands::activity::activity_trends,
            commands::activity_cards::cards_recent,
            commands::activity_cards::cards_count_new_since,
            commands::activity_cards::cards_set_last_seen,
            commands::activity_cards::cards_navigate,
            commands::activity_cards::cards_body,
            commands::activity_cards::cards_reindex,
            commands::session_prune::session_prune_plan,
            commands::session_prune::session_prune_start,
            commands::session_prune::session_slim_plan,
            commands::session_prune::session_slim_start,
            commands::session_prune::session_slim_plan_all,
            commands::session_prune::session_slim_start_all,
            commands::session_prune::session_trash_list,
            commands::session_prune::session_trash_restore,
            commands::session_prune::session_trash_empty,
            commands::session_share::session_export_preview,
            commands::session_share::session_share_gist_start,
            commands::session_share::settings_github_token_get,
            commands::session_share::settings_github_token_set,
            commands::session_share::settings_github_token_clear,
            commands::config::config_scan,
            commands::config::config_preview,
            commands::config::config_list_editors,
            commands::config::config_get_editor_defaults,
            commands::config::config_set_editor_default,
            commands::config::config_open_in_editor_path,
            commands::config::config_search_start,
            commands::config::config_search_cancel,
            commands::config::config_effective_settings,
            commands::config::config_effective_mcp,
            commands::pricing::pricing_get,
            commands::pricing::pricing_refresh,
            commands::usage_local::local_usage_aggregate,
            commands::usage_local::pricing_tier_get,
            commands::usage_local::pricing_tier_set,
            commands::usage_local::top_costly_prompts,
            commands::memory_health::memory_health_get,
            commands::cc_tips::cc_tips_list,
            commands::cc_tips::cc_tips_refresh,
            commands::cc_tips::cc_tips_record_view,
            commands::memory::memory_list_for_project,
            commands::memory::memory_read_file,
            commands::memory::memory_change_log,
            commands::memory::auto_memory_state,
            commands::memory::auto_memory_state_global,
            commands::memory::auto_memory_set,
            config_watch::config_watch_start,
            config_watch::config_watch_stop,
            commands::migrate::migrate_inspect,
            commands::migrate::migrate_export,
            commands::migrate::migrate_import,
            commands::migrate::migrate_undo,
            commands::routes::routes_list,
            commands::routes::routes_get,
            commands::routes::routes_settings_get,
            commands::routes::routes_settings_set,
            commands::routes::routes_add,
            commands::routes::routes_edit,
            commands::routes::routes_remove,
            commands::routes::routes_use_cli,
            commands::routes::routes_unuse_cli,
            commands::routes::routes_path_status,
            commands::routes::routes_add_to_path,
            commands::routes::routes_use_desktop,
            commands::routes::routes_unuse_desktop,
            commands::routes::routes_derive_slug,
            commands::routes::routes_validate_wrapper_name,
            commands::routes::routes_zero_secret,
            commands::routes::routes_desktop_running,
            commands::routes::routes_desktop_restart,
            commands::agents::agents_list,
            commands::agents::agents_get,
            commands::agents::agents_add,
            commands::agents::agent_install,
            commands::agents::agent_add_from_template,
            commands::agents::agents_update,
            commands::agents::agents_remove,
            commands::agents::agents_set_enabled,
            commands::agents::agents_run_now_start,
            commands::agents::agents_runs_list,
            commands::agents::agents_run_get,
            commands::agents::agents_validate_name,
            commands::agents::agents_validate_cron,
            commands::agents::agents_scheduler_capabilities,
            commands::agents::agents_dry_run_artifact,
            commands::agents::agents_open_artifact_dir,
            commands::agents::agents_linger_status,
            commands::agents::agents_linger_enable,
            commands::templates::templates_list,
            commands::templates::templates_get,
            commands::templates::templates_sample_report,
            commands::templates::templates_capable_routes,
            commands::templates::templates_install,
            commands::templates::templates_read_report,
            commands::templates::templates_pending_changes,
            commands::templates::templates_apply_pending,
            commands::templates::routing_rules_get,
            commands::templates::routing_rules_set,
            commands::templates::routing_rules_evaluate_for,
            commands::notification::notification_activate_host_for_session,
            commands::notification::notification_log_append,
            commands::notification::notification_log_append_routed,
            commands::notification::notification_log_mark_delivered,
            commands::notification::notification_log_list,
            commands::notification::notification_log_mark_all_read,
            commands::notification::notification_log_clear,
            commands::notification::notification_log_unread_count,
            commands::notification::notification_categories_metadata,
            commands::cc_doctor::cc_doctor_snapshot,
            commands::cc_doctor::cc_doctor_open_parse_failures_log,
            commands::cc_daemon::cc_daemon_status,
            commands::updates::updates_status_get,
            commands::updates::updates_check_now,
            commands::updates::updates_cli_install,
            commands::updates::updates_desktop_install,
            commands::updates::updates_settings_get,
            commands::updates::updates_settings_set,
            commands::updates::updates_channel_set,
            commands::updates::updates_minimum_version_set,
            // Channel-aware self-updater (Claudepot's own app bundle).
            // Distinct from the `updates::*` commands above, which
            // manage Claude Code's CLI/Desktop. See
            // `commands::release_update`.
            commands::release_update::release_channel_get,
            commands::release_update::release_channel_set,
            commands::release_update::release_update_check,
            commands::release_update::release_update_install,
            commands::release_update::release_relaunch_busy_ops,
            commands::rotation::rotation_rules_get,
            commands::rotation::rotation_rules_set,
            commands::rotation::rotation_rule_validate,
            commands::rotation::rotation_dry_run,
            commands::rotation::rotation_audit_get,
            commands::rotation::rotation_pending_list,
            commands::rotation::rotation_apply_pending,
            commands::rotation::rotation_dismiss_pending,
            commands::permission::permission_list,
            commands::permission::permission_get,
            commands::permission::permission_grant,
            commands::permission::permission_revert,
            commands::permission::permission_extend,
            commands::env_secret::env_vault_list,
            commands::env_secret::env_vault_add,
            commands::env_secret::env_vault_update,
            commands::env_secret::env_vault_delete,
            commands::env_secret::env_vault_copy,
            commands::env_secret::env_file_list,
            commands::env_secret::env_file_set,
            commands::env_secret::env_file_comment,
            commands::env_secret::env_file_uncomment,
            commands::env_secret::env_file_delete,
            commands::env_secret::env_file_copy_value,
            commands::env_secret::env_file_inject,
            // ─── shared_memory (WI-007, WI-009) ───────────────
            commands::shared_memory::shared_memory_search,
            commands::shared_memory::shared_memory_read_locator,
            commands::shared_memory::shared_memory_list_memories,
            commands::shared_memory::shared_memory_create_memory,
            commands::shared_memory::shared_memory_archive_memory,
            commands::shared_memory::shared_memory_list_decisions,
            commands::shared_memory::shared_memory_log_decision,
            commands::shared_memory::shared_memory_archive_decision,
            commands::shared_memory::shared_memory_list_sessions,
            commands::shared_memory::shared_memory_list_projects,
            commands::shared_memory::shared_memory_install_snippet,
            commands::shared_memory::shared_memory_snippet_body,
            commands::shared_memory::shared_memory_mcp_health,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("fatal: tauri application failed to start: {e}");
            std::process::exit(1);
        });
}
