//! Submenu builders + row formatting helpers for the tray menu.
//!
//! Extracted from `tray.rs` so that file can focus on the top-level
//! menu composition and click-routing. Each builder takes the current
//! app state snapshot and returns the finished `Submenu`; the callers
//! in `tray::rebuild` wire them together.

use crate::dto::AccountSummary;
use crate::tray_icons::{
    ICON_BAR_CHART, ICON_BOLT, ICON_CIRCLE_DOT, ICON_CIRCLE_PAUSE, ICON_CIRCLE_PLAY,
    ICON_CIRCLE_USER, ICON_DESKTOP, ICON_REFRESH, ICON_TERMINAL, ID_DESKTOP_BIND,
    ID_DESKTOP_CLEAR, ID_DESKTOP_LAUNCH, ID_DESKTOP_RECONCILE, ID_USAGE_REFRESH, PREFIX_CLI,
    PREFIX_DESKTOP, PREFIX_LIVE,
};
use claudepot_core::oauth::usage::UsageResponse;
use tauri::image::Image;
use tauri::menu::{
    IconMenuItemBuilder, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::{AppHandle, Manager, Wry};

pub fn build_cli_submenu(
    app: &AppHandle,
    summaries: &[AccountSummary],
) -> Result<Submenu<Wry>, String> {
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

pub fn build_desktop_submenu(
    app: &AppHandle,
    summaries: &[AccountSummary],
) -> Result<Submenu<Wry>, String> {
    let mut builder = SubmenuBuilder::new(app, "Set Desktop");
    if let Ok(img) = Image::from_bytes(ICON_DESKTOP) {
        builder = builder.submenu_icon(img);
    }

    // Phase 5 header items — unconditional utilities that apply to
    // the live Desktop session regardless of which account is
    // targeted. Placing them above the per-account switch list keeps
    // the account rows as the "pick one" block.
    let bind_item = MenuItemBuilder::with_id(ID_DESKTOP_BIND, "Bind current Desktop session")
        .enabled(true)
        .build(app)
        .map_err(|e| format!("desktop bind: {e}"))?;
    let clear_item = MenuItemBuilder::with_id(ID_DESKTOP_CLEAR, "Sign Desktop out")
        .enabled(true)
        .build(app)
        .map_err(|e| format!("desktop clear: {e}"))?;
    let launch_item = MenuItemBuilder::with_id(ID_DESKTOP_LAUNCH, "Launch Claude Desktop")
        .enabled(true)
        .build(app)
        .map_err(|e| format!("desktop launch: {e}"))?;
    let reconcile_item = MenuItemBuilder::with_id(ID_DESKTOP_RECONCILE, "Reconcile profile flags")
        .enabled(true)
        .build(app)
        .map_err(|e| format!("desktop reconcile: {e}"))?;
    let header_sep =
        PredefinedMenuItem::separator(app).map_err(|e| format!("desktop header sep: {e}"))?;
    builder = builder
        .item(&bind_item)
        .item(&clear_item)
        .item(&launch_item)
        .item(&reconcile_item)
        .item(&header_sep);

    let mut any = false;
    for s in summaries {
        if s.is_desktop_active {
            continue;
        }
        any = true;
        // Gate on disk truth (plan v2 D18). The DB flag can be
        // stale; desktop_profile_on_disk tracks reality.
        let label = if s.desktop_profile_on_disk {
            s.email.clone()
        } else {
            format!("{} (no profile)", s.email)
        };
        let item = MenuItemBuilder::with_id(format!("{PREFIX_DESKTOP}{}", s.uuid), label)
            .enabled(s.desktop_profile_on_disk)
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
pub fn build_usage_submenu(
    app: &AppHandle,
    snapshots: &[(AccountSummary, Option<UsageResponse>)],
) -> Result<Submenu<Wry>, String> {
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
pub fn build_live_submenu(app: &AppHandle) -> Result<Option<Submenu<Wry>>, String> {
    let Some(state) = app.try_state::<crate::state::LiveSessionState>() else {
        return Ok(None);
    };
    let list = state.service.snapshot();
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

pub fn build_tooltip(
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
