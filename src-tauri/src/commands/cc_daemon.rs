//! Tauri command for `claude daemon status`.
//!
//! Parallel to [`super::cc_doctor`] but simpler — the daemon scrape
//! is ~50ms and the renderer polls once per minute for the Sidebar
//! Activity strip badge + Activities dashboard tile. No caching: the
//! value is cheap to obtain and changes rapidly with bg-session
//! lifecycle, so a TTL cache would hide live transitions for no
//! visible win.
//!
//! Per `.claude/rules/architecture.md`: no business logic here. Wraps
//! [`claudepot_core::cc_daemon::scrape_daemon_status`] and converts
//! the core type to a DTO.

use crate::dto_cc_daemon::DaemonStatusDto;
use crate::dto_error::ErrorDto;

/// One-shot scrape. Runs on a blocking thread so the IPC worker isn't
/// tied up for the (sub-second but synchronous) process spawn.
///
/// The scrape itself is infallible — `scrape_daemon_status` reports an
/// unavailable daemon in the snapshot, not as an error — so the only
/// rejection is the shared `spawn_blocking` join failure.
#[tauri::command]
pub async fn cc_daemon_status() -> Result<DaemonStatusDto, ErrorDto> {
    let snapshot = tokio::task::spawn_blocking(claudepot_core::cc_daemon::scrape_daemon_status)
        .await
        .map_err(ErrorDto::task_join)?;
    Ok(snapshot.into())
}
