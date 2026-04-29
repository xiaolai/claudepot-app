//! Tauri commands for notification click routing.
//!
//! Surfaces just enough of `claudepot_core::host_activate` to let
//! the renderer ask "where was the user when this session was
//! running?" and bring that app forward.
//!
//! The single command here is the only platform-specific surface
//! the renderer needs to know about — host detection and activation
//! both live in `claudepot-core::host_activate`. Per the
//! architecture rule (`.claude/rules/architecture.md`), the Tauri
//! crate is a thin wrapper; no business logic lives here beyond
//! looking up the live session row to get its PID.

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
