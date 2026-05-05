//! IPC commands for the network-status feature. See
//! `dev-docs/network-status.md`.
//!
//! Two commands and one piece of shared state:
//!
//! - [`service_status_summary_get`] — read the cached summary that
//!   the watcher refreshed at most `poll_interval_minutes` ago.
//!   Pure read; never hits the network.
//! - [`service_status_probe_now`] — trigger an on-demand HEAD probe
//!   batch (the watcher does NOT do latency probing — see the doc
//!   for the cost / staleness rationale). Hits the network.
//! - [`ServiceStatusState`] — mutex-guarded last-known summary +
//!   last-known probe results. Owned here, mutated by the watcher
//!   and read by both commands.

use std::sync::Mutex;

use claudepot_core::service_status as core;
use tauri::State;

use crate::dto_service_status::{
    tier_str, ComponentDto, HostLatencyDto, IncidentDto, LatencyReportDto, ServiceStatusSummaryDto,
};

/// Tauri-managed shared state. Single mutex around both surfaces — the
/// watcher takes the lock once per cycle, the renderer takes it once
/// per `summary_get` call. Contention is bounded by user pace.
pub struct ServiceStatusState {
    inner: Mutex<Inner>,
}

struct Inner {
    summary: Option<core::StatusSummary>,
    /// `chrono::Utc` millis when `summary` was last refreshed.
    fetched_at_ms: Option<i64>,
    /// Last poll error message — surfaced in the renderer's tooltip
    /// so the user can tell "polling is off" from "polling is failing".
    last_error: Option<String>,
    /// Tier the watcher saw on the previous successful poll. The
    /// watcher uses this to detect transitions for the
    /// `notification_log` append; commands don't read it.
    pub last_tier: core::StatusTier,
    latency: Option<LatencyReport>,
}

#[derive(Clone)]
struct LatencyReport {
    probed_at_ms: i64,
    hosts: Vec<core::HostLatency>,
}

impl ServiceStatusState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                summary: None,
                fetched_at_ms: None,
                last_error: None,
                last_tier: core::StatusTier::Unknown,
                latency: None,
            }),
        }
    }

    /// Called by the watcher after a successful fetch. Returns the
    /// previous tier (for transition detection) and stores the new
    /// summary.
    pub fn store_summary(&self, summary: core::StatusSummary) -> core::StatusTier {
        let new_tier = core::summary_tier(&summary);
        let mut g = lock(&self.inner);
        let prev = g.last_tier;
        g.summary = Some(summary);
        g.fetched_at_ms = Some(chrono::Utc::now().timestamp_millis());
        g.last_error = None;
        g.last_tier = new_tier;
        prev
    }

    /// Called by the watcher after a fetch failure. Leaves the cached
    /// summary in place (stale-but-better-than-nothing) and records
    /// the error string for the renderer.
    pub fn store_fetch_error(&self, err: String) {
        let mut g = lock(&self.inner);
        g.last_error = Some(err);
        // Tier becomes Unknown only if we've never had a successful
        // poll. A transient fetch failure shouldn't paint the dot
        // grey when we already know the page is degraded.
        if g.summary.is_none() {
            g.last_tier = core::StatusTier::Unknown;
        }
    }

    pub fn store_latency(&self, hosts: Vec<core::HostLatency>) -> core::StatusTier {
        let tier = core::latency_tier(&hosts);
        let mut g = lock(&self.inner);
        g.latency = Some(LatencyReport {
            probed_at_ms: chrono::Utc::now().timestamp_millis(),
            hosts,
        });
        tier
    }
}

impl Default for ServiceStatusState {
    fn default() -> Self {
        Self::new()
    }
}

fn lock(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => {
            tracing::warn!("service_status: mutex poisoned; recovering");
            p.into_inner()
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn service_status_summary_get(
    state: State<'_, ServiceStatusState>,
) -> Result<ServiceStatusSummaryDto, String> {
    let g = lock(&state.inner);

    let (indicator, description, components, incidents) = match &g.summary {
        Some(s) => (
            Some(s.status.indicator.clone()),
            Some(s.status.description.clone()),
            s.components.iter().map(ComponentDto::from).collect(),
            s.incidents.iter().map(IncidentDto::from).collect(),
        ),
        None => (None, None, Vec::new(), Vec::new()),
    };

    Ok(ServiceStatusSummaryDto {
        tier: tier_str(g.last_tier),
        indicator,
        description,
        components,
        incidents,
        fetched_at_ms: g.fetched_at_ms,
        last_error: g.last_error.clone(),
    })
}

/// Trigger a fresh batch of HEAD probes. Worst-case wall time:
/// `service_status::PROBE_TIMEOUT` (5 s). The renderer is expected to
/// `await` this directly — there's no event channel needed because the
/// command itself returns the report.
#[tauri::command]
pub async fn service_status_probe_now(
    state: State<'_, ServiceStatusState>,
) -> Result<LatencyReportDto, String> {
    let hosts = core::probe_hosts(core::HOTPATH_HOSTS).await;
    state.store_latency(hosts.clone());

    let probed_at_ms = chrono::Utc::now().timestamp_millis();
    let tier = core::latency_tier(&hosts);
    Ok(LatencyReportDto {
        tier: tier_str(tier),
        probed_at_ms,
        hosts: hosts.iter().map(HostLatencyDto::from).collect(),
    })
}

/// Read the most recent latency report (probed by `_probe_now` or by
/// the renderer's last on-focus invocation). Returns `null`-equivalent
/// (`tier: "unknown"`, empty hosts) before the first probe.
#[tauri::command]
pub async fn service_status_latency_get(
    state: State<'_, ServiceStatusState>,
) -> Result<LatencyReportDto, String> {
    let g = lock(&state.inner);
    match &g.latency {
        Some(r) => Ok(LatencyReportDto {
            tier: tier_str(core::latency_tier(&r.hosts)),
            probed_at_ms: r.probed_at_ms,
            hosts: r.hosts.iter().map(HostLatencyDto::from).collect(),
        }),
        None => Ok(LatencyReportDto {
            tier: tier_str(core::StatusTier::Unknown),
            probed_at_ms: 0,
            hosts: Vec::new(),
        }),
    }
}
