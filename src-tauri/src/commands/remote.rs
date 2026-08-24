//! Tauri commands for Settings → Remote.
//!
//! Every verb is `claudepot_core::remote::service`, which `claudepot
//! remote …` also calls — see that module on why there is exactly one
//! implementation. What lives here is marshalling and the process-local
//! server handle.
//!
//! ## Secret direction
//!
//! `.claude/rules/architecture.md`: a secret entering Rust as an IPC
//! argument is acceptable — the user typed it — and must be zeroized on
//! every exit path. A secret *returning* over IPC is not. So
//! [`remote_set_password`] takes the password and scrubs it, and there
//! is deliberately no command that returns one. The device token is
//! never in reach here at all: the store holds a SHA-256 and
//! `DeviceSummary` does not carry even that.

use claudepot_core::remote::approval;
use claudepot_core::remote::service::{self, exposure_word};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::dto_error::{codes, ErrorDto};
use crate::remote_server::RemoteServerState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeviceDto {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub revoked_at: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatusDto {
    /// The stored preference.
    pub enabled: bool,
    /// A server is up **somewhere on this machine**, from the
    /// heartbeat. True of a `claudepot remote serve` in a terminal too.
    pub serving: bool,
    /// This process is the one serving. `serving && !running_here`
    /// means a CLI server owns the port, and Stop here cannot touch it.
    pub running_here: bool,
    pub url: Option<String>,
    pub bind: String,
    pub port: u16,
    /// `loopback` | `private_network` | `every_interface`, or null when
    /// the address is refused.
    pub exposure: Option<String>,
    pub bind_error: Option<String>,
    pub requires_tls: bool,
    pub password_set: bool,
    pub totp_enabled: bool,
    pub passkeys: usize,
    pub config_recovered: bool,
    pub devices_recovered: bool,
    /// Why the last start failed, or why a running server died.
    pub last_error: Option<String>,
    /// Approval-from-the-phone is off while these are non-empty.
    pub warnings: Vec<String>,
    pub devices: Vec<RemoteDeviceDto>,
    pub active_devices: usize,
}

fn map_err(e: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::detail(codes::REMOTE_FAILED, e)
}

fn to_device_dto(d: &service::DeviceSummary) -> RemoteDeviceDto {
    RemoteDeviceDto {
        id: d.id.to_string(),
        name: d.name.clone(),
        created_at: d.created_at.to_rfc3339(),
        last_seen: d.last_seen.map(|t| t.to_rfc3339()),
        revoked_at: d.revoked_at.map(|t| t.to_rfc3339()),
        expires_at: d.expires_at.map(|t| t.to_rfc3339()),
    }
}

/// Read the whole surface, including whether anything is serving.
///
/// Two liveness fields on purpose. `serving` is the heartbeat, which
/// belongs to the machine; `running_here` is this process. A pane that
/// had only the first would offer Stop for a server it cannot stop, and
/// one that had only the second would report "off" while a terminal was
/// serving the panel to a phone.
#[tauri::command]
pub async fn remote_status(
    server: State<'_, RemoteServerState>,
) -> Result<RemoteStatusDto, ErrorDto> {
    let local = server.describe().await;
    let st = tokio::task::spawn_blocking(|| service::status(approval::now_ms()))
        .await
        .map_err(ErrorDto::task_join)?
        .map_err(map_err)?;

    let now = chrono::Utc::now();
    // Counted before the struct literal moves the rest of `st`.
    let active_devices = st.active_device_count(now);
    let devices = st.devices.iter().map(to_device_dto).collect();
    Ok(RemoteStatusDto {
        enabled: st.enabled,
        serving: st.serving,
        running_here: local.running_here,
        url: local.url,
        bind: st.bind.to_string(),
        port: st.port,
        exposure: st.exposure.map(|e| exposure_word(e).to_string()),
        bind_error: st.bind_error,
        requires_tls: st.requires_tls,
        password_set: st.password_set,
        totp_enabled: st.totp_enabled,
        passkeys: st.passkeys,
        config_recovered: st.config_recovered,
        devices_recovered: st.devices_recovered,
        last_error: local.last_error,
        warnings: local.warnings,
        active_devices,
        devices,
    })
}

/// Turn the preference on. Does **not** start the server — the pane
/// asks for that separately, so "I want this available" and "start it
/// now" stay two decisions rather than one button that does both.
#[tauri::command]
pub async fn remote_enable(bind: Option<String>, port: Option<u16>) -> Result<(), ErrorDto> {
    tokio::task::spawn_blocking(move || service::enable(bind.as_deref(), port))
        .await
        .map_err(ErrorDto::task_join)?
        .map_err(map_err)?;
    Ok(())
}

/// Turn the preference off **and** stop a server this process is
/// running.
///
/// Both, because the alternative is the state the whole pane exists to
/// make impossible: "disabled" on screen while a socket is still
/// accepting logins. A server started from a terminal is not ours to
/// stop, and `remote_status` reports that as `serving && !runningHere`.
#[tauri::command]
pub async fn remote_disable(server: State<'_, RemoteServerState>) -> Result<(), ErrorDto> {
    tokio::task::spawn_blocking(service::disable)
        .await
        .map_err(ErrorDto::task_join)?
        .map_err(map_err)?;
    server.stop().await;
    Ok(())
}

/// Set the admin password.
///
/// `password` is moved in and zeroized on every path — success, empty,
/// and hash failure alike. `service::set_password` owns the scrub, so
/// there is one place it can be got wrong rather than two.
#[tauri::command]
pub async fn remote_set_password(password: String) -> Result<(), ErrorDto> {
    tokio::task::spawn_blocking(move || {
        let mut plain = password;
        service::set_password(&mut plain)
    })
    .await
    .map_err(ErrorDto::task_join)?
    .map_err(map_err)
}

#[tauri::command]
pub async fn remote_start(server: State<'_, RemoteServerState>) -> Result<String, ErrorDto> {
    server.start().await.map_err(map_err)
}

#[tauri::command]
pub async fn remote_stop(server: State<'_, RemoteServerState>) -> Result<bool, ErrorDto> {
    Ok(server.stop().await)
}

/// Revoke every session and paired device. Returns how many changed.
#[tauri::command]
pub async fn remote_revoke_all() -> Result<usize, ErrorDto> {
    tokio::task::spawn_blocking(service::revoke_all)
        .await
        .map_err(ErrorDto::task_join)?
        .map_err(map_err)
}

/// Revoke one device. `false` when it was already revoked.
#[tauri::command]
pub async fn remote_revoke_device(id: String) -> Result<bool, ErrorDto> {
    let uuid = Uuid::parse_str(&id).map_err(|e| {
        ErrorDto::with_params(
            codes::REMOTE_BAD_DEVICE_ID,
            serde_json::json!({ "detail": e.to_string() }),
            format!("not a device id: {id}"),
        )
    })?;
    tokio::task::spawn_blocking(move || service::revoke_device(uuid))
        .await
        .map_err(ErrorDto::task_join)?
        .map_err(map_err)
}
