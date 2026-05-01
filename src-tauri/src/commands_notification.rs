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
) -> Result<u64, String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || {
        log.append(args.source, args.kind, args.title, args.body, args.target)
            .map_err(|e| format!("notification_log append failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_append join: {e}"))?
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
) -> Result<(), String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || {
        log.mark_all_read()
            .map_err(|e| format!("notification_log mark_all_read failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_mark_all_read join: {e}"))?
}

/// Wipe every entry and reset the id counter. The popover surfaces
/// this behind a confirm in case the user clicks by mistake.
#[tauri::command]
pub async fn notification_log_clear(
    state: tauri::State<'_, NotificationLogState>,
) -> Result<(), String> {
    let log = std::sync::Arc::clone(&state.log);
    tokio::task::spawn_blocking(move || {
        log.clear()
            .map_err(|e| format!("notification_log clear failed: {e}"))
    })
    .await
    .map_err(|e| format!("notification_log_clear join: {e}"))?
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
