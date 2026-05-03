//! DTOs for the network-status feature. See `dev-docs/network-status.md`.
//!
//! Wire types crossing into the renderer. Mirror the shapes in
//! `claudepot_core::service_status` but stay decoupled so the renderer
//! never imports core types directly (the IPC boundary is the contract).

use claudepot_core::service_status as core;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusSummaryDto {
    /// `ok | degraded | down | unknown`. Pre-collapsed so the renderer
    /// doesn't have to know Statuspage's full vocabulary.
    pub tier: String,
    /// Page-level indicator from Statuspage (`none | minor | major |
    /// critical`). Null when we have never had a successful poll.
    pub indicator: Option<String>,
    /// Human-readable description from Statuspage (e.g. "All Systems
    /// Operational"). Null when we have never had a successful poll.
    pub description: Option<String>,
    /// Per-component statuses. Empty when no poll has succeeded yet.
    pub components: Vec<ComponentDto>,
    /// Active incidents (Statuspage filters resolved ones out of
    /// `summary.json` automatically). Empty when no poll has succeeded
    /// yet.
    pub incidents: Vec<IncidentDto>,
    /// `chrono::Utc` millis-since-epoch of the last successful poll.
    /// Null when no poll has succeeded yet.
    pub fetched_at_ms: Option<i64>,
    /// Last poll error string. Null on success or before first poll.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDto {
    pub id: String,
    pub name: String,
    /// Raw Statuspage status (`operational | degraded_performance | …`).
    pub status: String,
    /// Pre-collapsed tier (`ok | degraded | down`).
    pub tier: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentDto {
    pub id: String,
    pub name: String,
    /// `investigating | identified | monitoring | resolved | postmortem`.
    pub status: String,
    /// `none | minor | major | critical`.
    pub impact: String,
    pub created_at: String,
    pub shortlink: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyReportDto {
    /// Worst-of summary across all probes (`ok | degraded | down |
    /// unknown`).
    pub tier: String,
    pub probed_at_ms: i64,
    pub hosts: Vec<HostLatencyDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostLatencyDto {
    pub name: String,
    pub url: String,
    /// `ok | timeout | error`.
    pub kind: String,
    /// Round-trip-time in ms when `kind == "ok"`; null otherwise.
    pub ms: Option<u32>,
    /// Error message when `kind == "error"`; null otherwise.
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Conversions from core types
// ---------------------------------------------------------------------------

impl From<&core::Component> for ComponentDto {
    fn from(c: &core::Component) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            status: c.status.clone(),
            tier: tier_str(core::StatusTier::from_component_status(&c.status)),
        }
    }
}

impl From<&core::Incident> for IncidentDto {
    fn from(i: &core::Incident) -> Self {
        Self {
            id: i.id.clone(),
            name: i.name.clone(),
            status: i.status.clone(),
            impact: i.impact.clone(),
            created_at: i.created_at.clone(),
            shortlink: i.shortlink.clone(),
        }
    }
}

impl From<&core::HostLatency> for HostLatencyDto {
    fn from(h: &core::HostLatency) -> Self {
        match &h.result {
            core::LatencyResult::Ok { ms } => Self {
                name: h.name.clone(),
                url: h.url.clone(),
                kind: "ok".into(),
                ms: Some(*ms),
                message: None,
            },
            core::LatencyResult::Timeout => Self {
                name: h.name.clone(),
                url: h.url.clone(),
                kind: "timeout".into(),
                ms: None,
                message: None,
            },
            core::LatencyResult::Error { message } => Self {
                name: h.name.clone(),
                url: h.url.clone(),
                kind: "error".into(),
                ms: None,
                message: Some(message.clone()),
            },
        }
    }
}

pub fn tier_str(t: core::StatusTier) -> String {
    match t {
        core::StatusTier::Ok => "ok",
        core::StatusTier::Degraded => "degraded",
        core::StatusTier::Down => "down",
        core::StatusTier::Unknown => "unknown",
    }
    .to_string()
}
