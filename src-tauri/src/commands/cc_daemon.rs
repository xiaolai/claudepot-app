//! Tauri command for CC's background-daemon status.
//!
//! Parallel to [`super::cc_doctor`] but far simpler — the status is a
//! single small file read, and the renderer polls it once a minute for
//! the Sidebar Activity-strip badge. No caching: the value is cheap to
//! obtain and changes with bg-session lifecycle, so a TTL cache would
//! hide live transitions for no visible win.
//!
//! It used to spawn `claude daemon status` on every one of those
//! polls, which on a CC build predating that subcommand was billed as
//! a headless model prompt (issue #94). It spawns nothing now.
//!
//! Per `.claude/rules/architecture.md`: no business logic here. Wraps
//! [`claudepot_core::cc_daemon::daemon_status`] and converts the core
//! type to a DTO.

use crate::dto_cc_daemon::DaemonStatusDto;
use crate::dto_error::ErrorDto;

/// One-shot read. Runs on a blocking thread so the IPC worker isn't
/// tied up for the (sub-millisecond but synchronous) file read and
/// process-table probe.
///
/// The read itself is infallible — [`claudepot_core::cc_daemon`]
/// reports an unavailable daemon in the snapshot, not as an error — so
/// the only rejection is the shared `spawn_blocking` join failure.
#[tauri::command]
pub async fn cc_daemon_status() -> Result<DaemonStatusDto, ErrorDto> {
    let snapshot = tokio::task::spawn_blocking(claudepot_core::cc_daemon::daemon_status)
        .await
        .map_err(ErrorDto::task_join)?;
    Ok(snapshot.into())
}
