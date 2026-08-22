//! Tauri commands for the time-boxed remote-control window.
//!
//! All pure logic lives in `claudepot_core::peer::inbound`; this module
//! marshals a DTO and takes the process lock the orchestrator shares.
//! The DTO is defined here rather than in a `dto_*.rs` of its own — it
//! is one struct with one consumer, and a file per struct buys nothing.

use chrono::{Duration, Utc};
use claudepot_core::peer::inbound::{self, Decision, InboundState};
use serde::Serialize;

use crate::dto_error::{codes, ErrorDto};
use crate::peer_inbound_orchestrator::inbound_file_guard;

/// Guard rail against a malformed call, not policy — core enforces the
/// real cap (`MAX_GRANT_HOURS`) and reports it in its own words.
const MIN_GRANT_SECS: u64 = 60;
const MAX_GRANT_SECS: u64 = 12 * 3600;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInboundStateDto {
    /// Peer messages are being delivered without asking, however that
    /// came about.
    pub open: bool,
    /// Open **and** nothing holds a deadline on it. The UI must render
    /// this differently from a managed window: no timer will close it.
    pub unmanaged_open: bool,
    pub remaining_secs: Option<i64>,
    pub expires_at: Option<String>,
    /// `accept` | `hold` | `refuse` | `absent` | `invalid`. Distinct
    /// from `open` because "absent" and "a value CC rejects" are
    /// different problems with different fixes.
    pub observed: String,
}

fn observed_word(state: &InboundState) -> String {
    use claudepot_core::peer::inbound::settings::ModeValue;
    match &state.observed {
        ModeValue::Absent => "absent".into(),
        ModeValue::Valid(m) => m.as_wire().to_string(),
        ModeValue::Unrecognized(_) => "invalid".into(),
    }
}

fn to_dto(state: &InboundState) -> PeerInboundStateDto {
    let (remaining_secs, expires_at) = match &state.decision {
        Decision::Active { remaining_secs } => (Some(*remaining_secs), None),
        _ => (None, None),
    };
    PeerInboundStateDto {
        open: state.is_open(),
        unmanaged_open: state.is_unmanaged_open(),
        remaining_secs,
        expires_at,
        observed: observed_word(state),
    }
}

fn map_err(e: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::new(codes::PEER_INBOUND_FAILED, e.to_string())
}

/// Read-only. Does **not** reconcile: the orchestrator owns that, and a
/// read that mutates would fire on every render.
#[tauri::command]
pub async fn peer_inbound_state() -> Result<PeerInboundStateDto, ErrorDto> {
    tokio::task::spawn_blocking(|| {
        let state = inbound::state(Utc::now()).map_err(map_err)?;
        Ok(to_dto(&state))
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
pub async fn peer_inbound_grant(
    duration_secs: u64,
    reason: Option<String>,
) -> Result<PeerInboundStateDto, ErrorDto> {
    if !(MIN_GRANT_SECS..=MAX_GRANT_SECS).contains(&duration_secs) {
        return Err(ErrorDto::with_params(
            codes::PEER_INBOUND_DURATION_OUT_OF_RANGE,
            serde_json::json!({
                "min_secs": MIN_GRANT_SECS,
                "max_secs": MAX_GRANT_SECS,
                "got_secs": duration_secs,
            }),
            format!(
                "a remote-control window must be between {MIN_GRANT_SECS}s and {MAX_GRANT_SECS}s"
            ),
        ));
    }
    tokio::task::spawn_blocking(move || {
        let _guard = inbound_file_guard();
        let dur = Duration::try_seconds(duration_secs as i64)
            .ok_or_else(|| ErrorDto::new(codes::PEER_INBOUND_FAILED, "duration out of range"))?;
        inbound::open(dur, reason, Utc::now()).map_err(map_err)?;
        let state = inbound::state(Utc::now()).map_err(map_err)?;
        Ok(to_dto(&state))
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
pub async fn peer_inbound_revoke() -> Result<PeerInboundStateDto, ErrorDto> {
    tokio::task::spawn_blocking(|| {
        let _guard = inbound_file_guard();
        inbound::revoke(Utc::now()).map_err(map_err)?;
        let state = inbound::state(Utc::now()).map_err(map_err)?;
        Ok(to_dto(&state))
    })
    .await
    .map_err(map_err)?
}
