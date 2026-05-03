//! Anthropic / Claude service status + latency probes.
//!
//! Two distinct probes, deliberately split (see
//! `dev-docs/network-status.md`):
//!
//! 1. [`fetch_summary`] — one cheap GET against `status.claude.com/api/v2/summary.json`
//!    (Statuspage v2). Cadence is set by the caller; the watcher in
//!    `src-tauri/src/service_status_watcher.rs` runs it every 5 min.
//! 2. [`probe_hosts`] — concurrent HEAD probes against the
//!    [`HOTPATH_HOSTS`] list. On-demand only (window-focus + manual);
//!    background polling is intentionally not provided here. Burning
//!    every Claudepot install's battery for data that goes unread 95%
//!    of the time is a worse default than "compute when the user looks".
//!
//! Pure logic; no Tauri dependency. The Tauri layer wraps these
//! functions and threads results through DTOs / events.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Outbound probe target.
///
/// `name` is what the user sees in the StatusBar tooltip; `url` is the
/// full URL hit by the HEAD probe. Each entry's reason for being on
/// the hot-path list is documented at [`HOTPATH_HOSTS`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ProbeHost {
    pub name: &'static str,
    pub url: &'static str,
}

/// Hot-path hosts probed for "is my path to Claude fast" diagnostics.
///
/// Final list verified 2026-05-03 against the Claude Code source at
/// `~/github/claude_code_src/src`. Each entry is grounded in a
/// runtime call site, not just a registry observation.
///
/// **Canonical source: CC's own startup preflight**
/// (`utils/preflightChecks.tsx::checkEndpoints`):
///
/// ```text
/// endpoints = [
///   `${BASE_API_URL}/api/hello`,            // → api.anthropic.com
///   `${TOKEN_URL.origin}/v1/oauth/hello`,   // → platform.claude.com
/// ]
/// ```
///
/// CC pings exactly these two hosts at every startup to decide whether
/// "Anthropic services" are reachable. They are the floor of any
/// honest "is my path fast" check.
///
/// `claude.ai` is added for the practical case ("is the manifest /
/// download redirector reachable") even though CC only fetches its
/// OAuth client-metadata once and caches the result.
///
/// Hosts intentionally excluded — each with the reason from the
/// 2026-05-03 source audit:
///
/// - `statsig.anthropic.com` — NXDOMAIN globally; zero references in
///   CC source. xiaolai's `domains.yaml` lists it as "feature flags"
///   but the host doesn't exist publicly and CC's runtime FF stack is
///   GrowthBook configured with `apiHost: 'https://api.anthropic.com/'`,
///   not a Statsig endpoint. The `console.statsig.com/...` URLs in
///   CC source are doc-pointers to Anthropic's internal admin
///   dashboard, not client endpoints. See lock-in test
///   `statsig_anthropic_com_stays_excluded`.
///
/// - `cdn.growthbook.io` — listed in `domains.yaml` as "GrowthBook
///   feature flag service used by Claude Code CLI", but a `grep` of
///   CC source returns only one match — a `docs.growthbook.io` URL
///   in a generated event-schema comment. The GrowthBook SDK in
///   `services/analytics/growthbook.ts` is constructed with
///   `apiHost: baseUrl` where `baseUrl = 'https://api.anthropic.com/'`
///   (lines 503-527), so feature-flag fetches go through
///   `api.anthropic.com`, not the public GrowthBook CDN. The registry
///   entry's "active connection monitoring" source is contradicted by
///   the actual SDK config; until that's reconciled, probing
///   `cdn.growthbook.io` is misleading.
///
/// - `downloads.claude.ai` — only fetched on auto-update
///   (`utils/autoUpdater.ts`, `utils/nativeInstaller/download.ts`)
///   and plugin-marketplace install
///   (`utils/plugins/officialMarketplaceGcs.ts`). Cold path; not
///   relevant to "CC feels slow right now."
///
/// - `mcp-proxy.anthropic.com` — only used when the user has MCP
///   connectors configured. Cold path for the average user.
///
/// - `storage.googleapis.com` (shared host; can't isolate Claude
///   traffic — every other GCS user would also be probed).
///
/// - `claudeusercontent.com`, `claude.com`, `clau.de` (rarely fetched
///   by the CC client; mostly user-clicked links).
///
/// - `modelcontextprotocol.io`, `code.claude.com`, `docs.claude.com`,
///   `support.{anthropic,claude}.com` (docs / support links, never
///   auto-fetched by the client).
pub const HOTPATH_HOSTS: &[ProbeHost] = &[
    ProbeHost {
        name: "api.anthropic.com",
        url: "https://api.anthropic.com/",
    },
    ProbeHost {
        name: "platform.claude.com",
        url: "https://platform.claude.com/",
    },
    ProbeHost {
        name: "claude.ai",
        url: "https://claude.ai/",
    },
];

/// Default Statuspage summary endpoint. Overridable for tests via
/// [`fetch_summary_from`].
pub const STATUS_SUMMARY_URL: &str = "https://status.claude.com/api/v2/summary.json";

/// Per-host probe timeout. 5 s is generous enough that a real slow
/// path (saturated link, captive portal) still reports a number, but
/// short enough that the on-focus refresh feels responsive.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Statuspage v2 schema
// ---------------------------------------------------------------------------

/// Top-level shape returned by `/api/v2/summary.json`.
///
/// We deserialize only the fields we use; Statuspage adds new optional
/// fields over time and a tight schema would break on every upstream
/// update.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusSummary {
    pub page: PageInfo,
    pub status: StatusIndicator,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub incidents: Vec<Incident>,
    #[serde(default)]
    pub scheduled_maintenances: Vec<ScheduledMaintenance>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PageInfo {
    pub id: String,
    pub name: String,
    pub url: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusIndicator {
    pub indicator: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Incident {
    pub id: String,
    pub name: String,
    pub status: String,
    pub impact: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub shortlink: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduledMaintenance {
    pub id: String,
    pub name: String,
    pub status: String,
    pub scheduled_for: String,
    pub scheduled_until: String,
    pub shortlink: Option<String>,
}

// ---------------------------------------------------------------------------
// User-facing tier collapse
// ---------------------------------------------------------------------------

/// User-facing severity tier — a 4-state collapse of Statuspage's
/// fuller vocabulary so the StatusBar dot can pick a color without
/// the renderer having to know the upstream taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusTier {
    /// `operational` or `under_maintenance` (planned ≠ broken).
    Ok,
    /// `degraded_performance | partial_outage`, OR Statuspage indicator
    /// `minor`.
    Degraded,
    /// `major_outage`, OR Statuspage indicator `major | critical`.
    Down,
    /// We have no recent successful poll. Distinct from `Ok` so the
    /// renderer can show a neutral grey rather than a misleading green.
    Unknown,
}

impl StatusTier {
    /// Collapse a Statuspage component-status string to our 3-tier
    /// enum. Unknown values are treated as `Degraded` — better to
    /// over-warn than to silently classify a new severity as OK.
    pub fn from_component_status(s: &str) -> Self {
        match s {
            "operational" | "under_maintenance" => Self::Ok,
            "degraded_performance" | "partial_outage" => Self::Degraded,
            "major_outage" => Self::Down,
            _ => Self::Degraded,
        }
    }

    /// Collapse the top-level `status.indicator` field. `none` → Ok,
    /// `minor` → Degraded, `major | critical` → Down, anything else →
    /// Degraded.
    pub fn from_indicator(s: &str) -> Self {
        match s {
            "none" => Self::Ok,
            "minor" => Self::Degraded,
            "major" | "critical" => Self::Down,
            _ => Self::Degraded,
        }
    }
}

/// Compute the worst (= most severe) tier across the whole summary.
/// Combines the top-level indicator with each component's individual
/// status — Statuspage occasionally lists a degraded component while
/// the page-level indicator is still `none`, and we want to surface
/// that.
pub fn summary_tier(summary: &StatusSummary) -> StatusTier {
    let mut worst = StatusTier::from_indicator(&summary.status.indicator);
    for c in &summary.components {
        let t = StatusTier::from_component_status(&c.status);
        worst = worst.max(t);
    }
    worst
}

impl Ord for StatusTier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        fn rank(t: StatusTier) -> u8 {
            match t {
                StatusTier::Ok => 0,
                StatusTier::Unknown => 1,
                StatusTier::Degraded => 2,
                StatusTier::Down => 3,
            }
        }
        rank(*self).cmp(&rank(*other))
    }
}

impl PartialOrd for StatusTier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Latency probe
// ---------------------------------------------------------------------------

/// Outcome of a single host probe. `Ok` carries milliseconds; the
/// other variants are reported as their own categories so the UI can
/// distinguish "slow but reachable" from "unreachable" without a
/// sentinel value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LatencyResult {
    Ok {
        ms: u32,
    },
    Timeout,
    /// Connection / DNS / TLS failure. The string is rendered verbatim
    /// in tooltips; redact any potentially-sensitive shape (cookies,
    /// auth) in the upstream `reqwest::Error::Display`.
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostLatency {
    pub name: String,
    pub url: String,
    pub result: LatencyResult,
}

/// Worst-of summary across a probe batch. Returns
/// [`StatusTier::Unknown`] when the batch is empty.
pub fn latency_tier(results: &[HostLatency]) -> StatusTier {
    if results.is_empty() {
        return StatusTier::Unknown;
    }
    let mut worst = StatusTier::Ok;
    for r in results {
        let t = match &r.result {
            LatencyResult::Ok { ms } => {
                if *ms > 1500 {
                    StatusTier::Degraded
                } else {
                    StatusTier::Ok
                }
            }
            LatencyResult::Timeout | LatencyResult::Error { .. } => StatusTier::Down,
        };
        worst = worst.max(t);
    }
    worst
}

// ---------------------------------------------------------------------------
// HTTP entry points
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ServiceStatusError {
    #[error("status fetch failed: {0}")]
    Fetch(String),
    #[error("status parse failed: {0}")]
    Parse(String),
}

/// Fetch the Claude Statuspage summary from the canonical URL. Hits
/// the network on every call — the caller is responsible for
/// rate-limiting (see `service_status_watcher.rs`).
pub async fn fetch_summary() -> Result<StatusSummary, ServiceStatusError> {
    fetch_summary_from(STATUS_SUMMARY_URL).await
}

/// Lower-level form for tests + alternate endpoints.
pub async fn fetch_summary_from(url: &str) -> Result<StatusSummary, ServiceStatusError> {
    let client = build_client();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ServiceStatusError::Fetch(redact(&e.to_string())))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(ServiceStatusError::Fetch(format!(
            "HTTP {} from {url}",
            status.as_u16()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ServiceStatusError::Fetch(redact(&e.to_string())))?;
    serde_json::from_slice(&bytes).map_err(|e| ServiceStatusError::Parse(e.to_string()))
}

/// Probe each host in parallel, return per-host latency results in the
/// same order as the input slice.
///
/// All probes share a single `reqwest::Client` (connection-pool
/// reuse across hosts). Each probe is fire-and-forget on its own
/// `tokio::spawn`; we collect the results via channel rather than
/// `JoinSet` so the result ordering is stable. Worst-case wall time
/// is `PROBE_TIMEOUT` (5 s).
pub async fn probe_hosts(hosts: &[ProbeHost]) -> Vec<HostLatency> {
    let client = build_client();
    let mut handles = Vec::with_capacity(hosts.len());
    for h in hosts {
        let client = client.clone();
        let host = *h;
        handles.push(tokio::spawn(async move { probe_one(&client, host).await }));
    }

    let mut out = Vec::with_capacity(hosts.len());
    for (i, h) in handles.into_iter().enumerate() {
        let host = hosts[i];
        let result = match h.await {
            Ok(r) => r,
            // Task panic — extremely unlikely with the simple body
            // below, but reportable if it does happen so we don't
            // silently drop a host.
            Err(e) => HostLatency {
                name: host.name.to_string(),
                url: host.url.to_string(),
                result: LatencyResult::Error {
                    message: format!("probe task aborted: {e}"),
                },
            },
        };
        out.push(result);
    }
    out
}

async fn probe_one(client: &reqwest::Client, host: ProbeHost) -> HostLatency {
    let start = Instant::now();
    let result = match client.head(host.url).send().await {
        Ok(_resp) => {
            // Any HTTP response — including 4xx / 5xx — proves the
            // path is alive. We're measuring round-trip time, not
            // protocol correctness. `api.anthropic.com` HEAD without
            // auth typically returns 401, which is a successful probe.
            let ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            LatencyResult::Ok { ms }
        }
        Err(e) if e.is_timeout() => LatencyResult::Timeout,
        Err(e) => LatencyResult::Error {
            message: redact(&e.to_string()),
        },
    };
    HostLatency {
        name: host.name.to_string(),
        url: host.url.to_string(),
        result,
    }
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(PROBE_TIMEOUT)
        .timeout(PROBE_TIMEOUT)
        // Don't follow redirects — a 301/302 already proves the path
        // is alive. Following inflates RTT and gives a misleading
        // "claude.ai" measurement that's actually claude.ai/login.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Claudepot/", env!("CARGO_PKG_VERSION")))
        .build()
        // The builder only fails on invalid TLS / DNS resolver setup —
        // both impossible with the default rustls backend. Falling
        // back to `Client::new()` is strictly safer than panicking the
        // process on a transient init blip.
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Sanitize an error string before surfacing it to the renderer.
/// `reqwest::Error::Display` is generally clean (it doesn't include
/// request bodies or headers), but we still strip anything that looks
/// like a token in case a custom error path leaked one. Mirrors the
/// sk-ant truncation policy in `claudepot_core::redaction`.
fn redact(s: &str) -> String {
    if s.contains("sk-ant-") {
        // Truncated form keeps the prefix so a developer can still
        // identify which token leaked, without exposing the full
        // value in toasts / logs.
        return s
            .split_whitespace()
            .map(|w| {
                if w.starts_with("sk-ant-") && w.len() > 16 {
                    format!("{}...{}", &w[..12], &w[w.len() - 4..])
                } else {
                    w.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    s.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal recorded snapshot of `status.claude.com/api/v2/summary.json`
    /// (verified live during plan-doc creation, 2026-05-03). We round-trip
    /// it through serde to lock the shape down — Statuspage adds optional
    /// fields over time and a strict schema would break on upstream churn.
    const SAMPLE_SUMMARY: &str = r#"{
        "page": {
            "id": "abc",
            "name": "Claude Status",
            "url": "https://status.claude.com",
            "updated_at": "2026-05-03T00:00:00Z"
        },
        "status": { "indicator": "none", "description": "All Systems Operational" },
        "components": [
            { "id": "c1", "name": "Claude Code", "status": "operational" },
            { "id": "c2", "name": "API", "status": "operational" }
        ],
        "incidents": [],
        "scheduled_maintenances": []
    }"#;

    #[test]
    fn parses_canonical_summary() {
        let s: StatusSummary = serde_json::from_str(SAMPLE_SUMMARY).unwrap();
        assert_eq!(s.page.name, "Claude Status");
        assert_eq!(s.status.indicator, "none");
        assert_eq!(s.components.len(), 2);
        assert!(s.incidents.is_empty());
    }

    #[test]
    fn parses_with_active_incident() {
        let json = r#"{
            "page": { "id": "p", "name": "n", "url": "u" },
            "status": { "indicator": "major", "description": "Service disruption" },
            "components": [
                { "id": "c1", "name": "API", "status": "major_outage" }
            ],
            "incidents": [
                {
                    "id": "i1",
                    "name": "API errors",
                    "status": "investigating",
                    "impact": "major",
                    "created_at": "2026-05-03T00:00:00Z"
                }
            ],
            "scheduled_maintenances": []
        }"#;
        let s: StatusSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary_tier(&s), StatusTier::Down);
        assert_eq!(s.incidents.len(), 1);
        assert_eq!(s.incidents[0].name, "API errors");
    }

    #[test]
    fn collapses_component_status() {
        assert_eq!(
            StatusTier::from_component_status("operational"),
            StatusTier::Ok
        );
        assert_eq!(
            StatusTier::from_component_status("under_maintenance"),
            StatusTier::Ok
        );
        assert_eq!(
            StatusTier::from_component_status("degraded_performance"),
            StatusTier::Degraded
        );
        assert_eq!(
            StatusTier::from_component_status("partial_outage"),
            StatusTier::Degraded
        );
        assert_eq!(
            StatusTier::from_component_status("major_outage"),
            StatusTier::Down
        );
        // Unknown shape from upstream → Degraded (over-warn over silent
        // misclassification).
        assert_eq!(
            StatusTier::from_component_status("future_severity"),
            StatusTier::Degraded
        );
    }

    #[test]
    fn collapses_indicator() {
        assert_eq!(StatusTier::from_indicator("none"), StatusTier::Ok);
        assert_eq!(StatusTier::from_indicator("minor"), StatusTier::Degraded);
        assert_eq!(StatusTier::from_indicator("major"), StatusTier::Down);
        assert_eq!(StatusTier::from_indicator("critical"), StatusTier::Down);
    }

    #[test]
    fn summary_tier_takes_worst_of_indicator_and_components() {
        // Indicator says "none" but a component is degraded → Degraded.
        let json = r#"{
            "page": { "id": "p", "name": "n", "url": "u" },
            "status": { "indicator": "none", "description": "All Systems Operational" },
            "components": [
                { "id": "c1", "name": "API", "status": "degraded_performance" }
            ],
            "incidents": [],
            "scheduled_maintenances": []
        }"#;
        let s: StatusSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary_tier(&s), StatusTier::Degraded);
    }

    #[test]
    fn tier_ordering_respects_severity() {
        assert!(StatusTier::Down > StatusTier::Degraded);
        assert!(StatusTier::Degraded > StatusTier::Unknown);
        assert!(StatusTier::Unknown > StatusTier::Ok);
    }

    #[test]
    fn latency_tier_collapses_to_worst() {
        let ok = vec![HostLatency {
            name: "a".into(),
            url: "u".into(),
            result: LatencyResult::Ok { ms: 50 },
        }];
        assert_eq!(latency_tier(&ok), StatusTier::Ok);

        let slow = vec![HostLatency {
            name: "a".into(),
            url: "u".into(),
            result: LatencyResult::Ok { ms: 2500 },
        }];
        assert_eq!(latency_tier(&slow), StatusTier::Degraded);

        let timed_out = vec![HostLatency {
            name: "a".into(),
            url: "u".into(),
            result: LatencyResult::Timeout,
        }];
        assert_eq!(latency_tier(&timed_out), StatusTier::Down);

        let empty: Vec<HostLatency> = vec![];
        assert_eq!(latency_tier(&empty), StatusTier::Unknown);
    }

    #[test]
    fn redact_truncates_sk_ant_tokens() {
        let s = "auth failed for sk-ant-oat01-AbcdefghijklmnopQRSTUVwxyz";
        let r = redact(s);
        assert!(r.contains("sk-ant-oat01"));
        assert!(!r.contains("UVwxyz"));
        // No token in the string → unchanged.
        assert_eq!(redact("plain error message"), "plain error message");
    }

    #[test]
    fn hotpath_hosts_are_https_and_well_formed() {
        for h in HOTPATH_HOSTS {
            assert!(h.url.starts_with("https://"), "host {} not https", h.name);
            assert!(!h.name.is_empty());
            assert!(!h.url.is_empty());
        }
        // The two hosts CC's own startup preflight checks
        // (`utils/preflightChecks.tsx::checkEndpoints` — `/api/hello`
        // and `/v1/oauth/hello`). Removing either should be a
        // deliberate decision, not an accident.
        assert!(HOTPATH_HOSTS.iter().any(|h| h.name == "api.anthropic.com"));
        assert!(HOTPATH_HOSTS
            .iter()
            .any(|h| h.name == "platform.claude.com"));
        assert!(HOTPATH_HOSTS.iter().any(|h| h.name == "claude.ai"));
    }

    /// Lock-in test for the 2026-05-03 audit findings. Each host below
    /// was probed and traced through CC source; re-adding any of them
    /// would re-introduce a known-bad signal. If a future audit
    /// confirms the underlying conditions changed, delete the matching
    /// assertion in the same commit (with the source citation).
    #[test]
    fn known_bad_hosts_stay_excluded() {
        // NXDOMAIN globally; zero references in CC source.
        assert!(
            !HOTPATH_HOSTS
                .iter()
                .any(|h| h.name == "statsig.anthropic.com"),
            "statsig.anthropic.com is NXDOMAIN globally and not on \
             CC's runtime path; see HOTPATH_HOSTS docstring."
        );
        // CC's GrowthBook SDK is configured with apiHost=api.anthropic.com;
        // the public GrowthBook CDN is not on the runtime path.
        assert!(
            !HOTPATH_HOSTS.iter().any(|h| h.name == "cdn.growthbook.io"),
            "cdn.growthbook.io is not on CC's runtime path — its \
             GrowthBook client uses apiHost=api.anthropic.com. See \
             HOTPATH_HOSTS docstring before re-adding."
        );
    }
}
