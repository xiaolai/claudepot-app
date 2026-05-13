//! Tauri commands for notification routing + persistent log.
//!
//! Two distinct surfaces share this file:
//!
//! 1. **Click routing** ([`notification_activate_host_for_session`])
//!    — the Tauri 2 desktop notification plugin doesn't surface
//!    body-click events to JS, so when an OS banner is clicked the
//!    App-shell focus listener consults the in-renderer click queue,
//!    and for the `host` intent calls into the
//!    `claudepot_core::host_activate` walk to bring the originating
//!    terminal/editor forward.
//!
//! 2. **Notification log** (`notification_log_*`) — append-and-list
//!    surface for the bell-icon popover. Both `pushToast` and
//!    `dispatchOsNotification` in the renderer call append on every
//!    dispatch so the user can scroll back through "what did
//!    Claudepot just tell me?". State lives at
//!    `~/.claudepot/notifications.json` via
//!    [`claudepot_core::notification_log`].
//!
//! Per `.claude/rules/architecture.md`, no business logic lives here
//! beyond pulling the right state slice.

use claudepot_core::notification_log::{
    NotificationEntry, NotificationKind, NotificationLog, NotificationLogFilter,
    NotificationSource, SortOrder,
};
use claudepot_core::notifications::{Category, CategoryMeta, Surface};

/// Per-field byte caps for notification log entries. The ring
/// buffer holds 500 entries; without per-field caps a renderer bug
/// could persist multi-megabyte titles/bodies and make every list
/// IPC O(n × MB). Audit-fix Medium #12.
const MAX_TITLE_LEN: usize = 256;
const MAX_BODY_LEN: usize = 2048;
/// Target JSON cap — serialized form. We bound the input string
/// length post-serialize so the renderer's `NotificationTarget`
/// shapes (small discriminated unions) easily fit.
const MAX_TARGET_BYTES: usize = 4096;

/// Truncate `s` to `max` bytes on a char boundary, appending an
/// ellipsis when truncated. Cheap defense against a renderer bug
/// flooding the ring buffer with huge strings.
fn cap_string(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

/// Drop oversize JSON targets to `null`. The bell-popover click
/// route doesn't degrade when this happens — it just doesn't have a
/// click destination for that one entry.
fn cap_target(t: serde_json::Value) -> serde_json::Value {
    let serialized_len = serde_json::to_string(&t).map(|s| s.len()).unwrap_or(0);
    if serialized_len > MAX_TARGET_BYTES {
        serde_json::Value::Null
    } else {
        t
    }
}

/// Tauri-managed handle to the open notification log. Cheap to clone
/// (single Arc); construction is the single file read on app boot.
pub struct NotificationLogState {
    pub log: std::sync::Arc<NotificationLog>,
}

impl NotificationLogState {
    pub fn new(log: NotificationLog) -> Self {
        Self {
            log: std::sync::Arc::new(log),
        }
    }
}

// ─── Click-routing surface ─────────────────────────────────────────

/// Activate the host terminal/editor running the live session
/// identified by `session_id`. Returns `true` when a host was
/// activated, `false` when none could be resolved (session ended
/// between dispatch and click, or the host process is not in the
/// known terminal/editor table).
///
/// Best-effort — the renderer falls back to deep-linking the
/// transcript inside Claudepot when this returns `false`. Errors
/// are flattened to `String` per the codebase convention; the
/// renderer ignores them and falls back as well.
#[tauri::command]
pub async fn notification_activate_host_for_session(
    session_id: String,
    state: tauri::State<'_, crate::state::LiveSessionState>,
) -> Result<bool, String> {
    use claudepot_core::host_activate::{activate_bundle_id, find_host_bundle_id, HostLookup};

    // Look up the live session by id. Snapshot is cheap (no IO);
    // it's a clone of the in-memory aggregate. Drop the snapshot
    // immediately — we only need the PID.
    let pid = {
        let snap = state.service.snapshot();
        match snap.iter().find(|s| s.session_id == session_id) {
            Some(s) => s.pid,
            None => return Ok(false),
        }
    };

    // The PID-walk reads `/proc` (Linux), `proc_listpids` (macOS),
    // or the equivalent Windows API. Cheap but blocking — keep it
    // off the Tauri command thread by using `spawn_blocking`. The
    // renderer awaits this, so the brief context-switch overhead
    // is paid once per click.
    let lookup = tokio::task::spawn_blocking(move || find_host_bundle_id(pid))
        .await
        .map_err(|e| format!("host lookup join: {e}"))?;

    match lookup {
        HostLookup::Found { bundle_id, .. } => {
            activate_bundle_id(bundle_id).map_err(|e| format!("open -b {bundle_id}: {e}"))?;
            Ok(true)
        }
        HostLookup::NotFound | HostLookup::PidGone => Ok(false),
    }
}

// ─── Notification log surface ──────────────────────────────────────

/// DTO for `notification_log_append`. Mirrors the shape used by the
/// renderer's capture sites — `kind`, `source`, `title`, optional
/// `body`, optional `target` JSON. The id and timestamp are assigned
/// server-side so the renderer cannot lie about ordering.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationLogAppendArgs {
    pub source: NotificationSource,
    pub kind: NotificationKind,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Renderer's `NotificationTarget` value, opaque to Rust. `null`
    /// when the surface had no click target.
    #[serde(default)]
    pub target: serde_json::Value,
}

/// Sort order parameter for `notification_log_list`. Standalone enum
/// so the JS side can pass `"newestFirst" | "oldestFirst"` strings
/// without wrapping them in a doc.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationLogOrderArg {
    #[default]
    NewestFirst,
    OldestFirst,
}

impl From<NotificationLogOrderArg> for SortOrder {
    fn from(value: NotificationLogOrderArg) -> Self {
        match value {
            NotificationLogOrderArg::NewestFirst => SortOrder::NewestFirst,
            NotificationLogOrderArg::OldestFirst => SortOrder::OldestFirst,
        }
    }
}

/// Append a single entry. Returns the assigned id so the renderer
/// can correlate post-dispatch (mostly used for tests today, but
/// the wire surface is cheap).
#[tauri::command]
pub async fn notification_log_append(
    args: NotificationLogAppendArgs,
    state: tauri::State<'_, NotificationLogState>,
    app: tauri::AppHandle,
) -> Result<u64, String> {
    let log = std::sync::Arc::clone(&state.log);
    let title = cap_string(args.title, MAX_TITLE_LEN);
    let body = cap_string(args.body, MAX_BODY_LEN);
    let target = cap_target(args.target);
    let id = tokio::task::spawn_blocking(move || {
        log.append(args.source, args.kind, title, body, target)
            .map_err(|e| format!("notification_log append failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_append join: {e}"))??;
    // Refresh the tray badge so a fresh entry lights up the dot.
    crate::tray::refresh_alert_chrome(&app);
    Ok(id)
}

/// List entries matching `filter`, in `order`, capped at `limit` (or
/// the buffer cap). The filter is shape-compatible with the
/// renderer's `NotificationLogFilter` type — see `src/api/notification.ts`.
#[tauri::command]
pub async fn notification_log_list(
    filter: Option<NotificationLogFilter>,
    order: Option<NotificationLogOrderArg>,
    limit: Option<usize>,
    state: tauri::State<'_, NotificationLogState>,
) -> Result<Vec<NotificationEntry>, String> {
    let log = std::sync::Arc::clone(&state.log);
    let filter = filter.unwrap_or_default();
    let order: SortOrder = order.unwrap_or_default().into();
    tokio::task::spawn_blocking(move || Ok(log.list(&filter, order, limit)))
        .await
        .map_err(|e| format!("notification_log_list join: {e}"))?
}

/// Mark every current entry as seen. Sets the bell badge count to 0
/// until a fresh entry lands.
#[tauri::command]
pub async fn notification_log_mark_all_read(
    state: tauri::State<'_, NotificationLogState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || {
        log.mark_all_read()
            .map_err(|e| format!("notification_log mark_all_read failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_mark_all_read join: {e}"))??;
    crate::tray::refresh_alert_chrome(&app);
    Ok(())
}

/// Wipe every entry and reset the id counter. The popover surfaces
/// this behind a confirm in case the user clicks by mistake.
#[tauri::command]
pub async fn notification_log_clear(
    state: tauri::State<'_, NotificationLogState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || {
        log.clear()
            .map_err(|e| format!("notification_log clear failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_clear join: {e}"))??;
    crate::tray::refresh_alert_chrome(&app);
    Ok(())
}

/// Return the current unread count — entries with `id > last_seen_id`.
/// Drives the bell badge.
#[tauri::command]
pub async fn notification_log_unread_count(
    state: tauri::State<'_, NotificationLogState>,
) -> Result<u32, String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || Ok(log.unread_count()))
        .await
        .map_err(|e| format!("notification_log_unread_count join: {e}"))?
}

// ─── Phase 1: routed-emit surface ──────────────────────────────────
//
// The `emit()` facade in `src/lib/notifications/dispatch.ts` is the
// only TS path that should call these IPCs. Old `notification_log_append`
// stays for the migration-shim window (Phase 1 → Phase 3) so unmigrated
// sites keep their bell entries.

/// DTO for `notification_log_append_routed`. Carries the routing
/// metadata the new `emit()` facade computed before dispatch.
///
/// Audit-fix High #7: `priority` is NOT in this DTO. The server
/// derives it from `category` via `Category::priority()` so a
/// renderer drift can't persist impossible rows (e.g. a P3 category
/// tagged P0). Same applies to `surfaces_delivered` — entries
/// outside `surfaces_requested` are filtered in `append_routed`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationLogAppendRoutedArgs {
    pub category: Category,
    pub kind: NotificationKind,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub target: serde_json::Value,
    pub surfaces_requested: Vec<Surface>,
    /// Surfaces already known-delivered at append time. Toast and
    /// Banner are renderer-side and always delivered if requested,
    /// so callers populate this with `[toast]` / `[toast, banner]`
    /// at emit time. OS-banner delivery comes back through
    /// `notification_log_mark_delivered` after the OS dispatcher
    /// resolves its focus / permission / rate gates.
    pub surfaces_delivered: Vec<Surface>,
}

/// Append a routed entry. Returns the assigned id so the renderer
/// can call `notification_log_mark_delivered` later when the OS
/// dispatcher reports a delivery outcome.
#[tauri::command]
pub async fn notification_log_append_routed(
    args: NotificationLogAppendRoutedArgs,
    state: tauri::State<'_, NotificationLogState>,
    app: tauri::AppHandle,
) -> Result<u64, String> {
    let log = std::sync::Arc::clone(&state.log);
    // Derive priority server-side (audit-fix High #7) so a renderer
    // drift can never persist an impossible row. The Rust enum's
    // exhaustive `Category::priority()` binding is the
    // single source-of-truth.
    let priority = args.category.priority();
    let title = cap_string(args.title, MAX_TITLE_LEN);
    let body = cap_string(args.body, MAX_BODY_LEN);
    let target = cap_target(args.target);
    // Audit-fix High #7 (server-side surface validation): trim the
    // renderer-supplied `surfaces_requested` so it can't claim more
    // than the priority default plus the `os_override` policy. A
    // renderer bug or malicious IPC shouldn't be able to mark e.g.
    // a P3 category as having requested `[Toast, OsBanner, Banner]`.
    //
    // The rules:
    //   - P0Blocking allows {toast, osBanner, banner} per route().
    //   - P1Stalled allows {toast (override), osBanner, banner (override)}.
    //   - P2Acknowledge allows {toast, osBanner (override)}.
    //   - P3Ambient allows {} by default; `os_override = Some(true)`
    //     lets osBanner in, but nothing else.
    // For simplicity we apply the priority's MAX set: every surface
    // the priority could allow under any context override. Anything
    // outside that envelope is impossible by construction.
    let allowed: std::collections::HashSet<claudepot_core::notifications::Surface> = match priority
    {
        claudepot_core::notifications::Priority::P0Blocking => [
            claudepot_core::notifications::Surface::Toast,
            claudepot_core::notifications::Surface::OsBanner,
            claudepot_core::notifications::Surface::Banner,
        ]
        .into_iter()
        .collect(),
        claudepot_core::notifications::Priority::P1Stalled => [
            claudepot_core::notifications::Surface::Toast,
            claudepot_core::notifications::Surface::OsBanner,
            claudepot_core::notifications::Surface::Banner,
        ]
        .into_iter()
        .collect(),
        claudepot_core::notifications::Priority::P2Acknowledge => [
            claudepot_core::notifications::Surface::Toast,
            claudepot_core::notifications::Surface::OsBanner,
        ]
        .into_iter()
        .collect(),
        claudepot_core::notifications::Priority::P3Ambient => {
            [claudepot_core::notifications::Surface::OsBanner]
                .into_iter()
                .collect()
        }
    };
    let surfaces_requested: Vec<claudepot_core::notifications::Surface> = args
        .surfaces_requested
        .into_iter()
        .filter(|s| allowed.contains(s))
        .collect();
    let id = tokio::task::spawn_blocking(move || {
        log.append_routed(
            args.category,
            priority,
            args.kind,
            title,
            body,
            target,
            surfaces_requested,
            args.surfaces_delivered,
        )
        .map_err(|e| format!("notification_log append_routed failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_append_routed join: {e}"))??;
    crate::tray::refresh_alert_chrome(&app);
    Ok(id)
}

/// Mark an entry as delivered on `surface`. Used by the OS dispatcher
/// to post-confirm after focus / rate gates resolve. Idempotent.
/// Returns `true` when the entry was updated; `false` if the id is no
/// longer in the ring buffer (evicted by a burst) — caller can ignore.
#[tauri::command]
pub async fn notification_log_mark_delivered(
    id: u64,
    surface: Surface,
    state: tauri::State<'_, NotificationLogState>,
) -> Result<bool, String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || {
        log.mark_delivered(id, surface)
            .map_err(|e| format!("notification_log mark_delivered failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_mark_delivered join: {e}"))?
}

/// Return the full category metadata table for the Settings pane.
/// Source-of-truth lives in `claudepot_core::notifications::Category::display_meta`;
/// the renderer reads this at mount and renders one row per entry.
#[tauri::command]
pub async fn notification_categories_metadata() -> Result<Vec<CategoryMeta>, String> {
    Ok(Category::all().iter().map(|c| c.display_meta()).collect())
}
