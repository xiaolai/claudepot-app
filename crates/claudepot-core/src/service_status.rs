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

// ---------------------------------------------------------------------------
// First-run diagnosis — coarser than HostLatency, structured for the
// network-detection panel. See `dev-docs/network-detection-panel.md`.
// ---------------------------------------------------------------------------

/// Coarse classification of why the primary Anthropic host is
/// unreachable. The exact error message that produced the variant is
/// not load-bearing: the variant tells the panel which copy to show
/// and which remediation to highlight, the message stays available
/// for diagnostics-pane display only.
///
/// Heuristics are conservative: `Unknown` is the right fallback when
/// the error string doesn't match a known signature, never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Diagnosis {
    /// `api.anthropic.com` returned an HTTP response (any status —
    /// even 401 means the path is alive).
    Reachable,
    /// DNS resolution failed. Common GFW symptom (DNS poisoning) and
    /// also the result of a broken resolver / captive portal that
    /// hijacks DNS.
    DnsFailure,
    /// TCP connection refused or reset. The path resolves but a
    /// downstream router or firewall is actively dropping traffic.
    ConnectionRefused,
    /// TLS handshake failed — certificate validation, version
    /// mismatch, or interception. Often the signature of an
    /// inspecting middlebox.
    TlsError,
    /// Connection timed out — could be saturation, blocked path, or
    /// a route black-hole. Indistinguishable from "blocked" without
    /// more probes; the panel says "couldn't reach in time" and
    /// surfaces the same remediation set as `ConnectionRefused`.
    Timeout,
    /// Reached an HTTP responder but it wasn't Anthropic's API (or
    /// Anthropic returned 5xx). Distinct from unreachability — this
    /// is a service-side issue, not a network one.
    HttpError,
    /// None of the heuristics matched. The panel surfaces the raw
    /// (redacted) message and the generic remediation set.
    Unknown,
}

impl Diagnosis {
    /// True iff the diagnosis indicates Anthropic is reachable from
    /// the user's network. Used as the panel's gate.
    pub fn reachable(self) -> bool {
        matches!(self, Self::Reachable)
    }
}

/// Classify a redacted error message into a coarse [`Diagnosis`].
/// Pure string heuristics — no network calls. Returns
/// [`Diagnosis::Unknown`] when no signature matches.
///
/// Keep the patterns lowercase-insensitive and substring-based: the
/// underlying `reqwest::Error::Display` shape varies across hyper /
/// rustls versions, and matching loosely is more robust than
/// matching the exact phrasing of any one library version.
pub fn classify_error(message: &str) -> Diagnosis {
    let m = message.to_lowercase();
    // DNS first — "dns error" / "failed to lookup address" / "name
    // or service not known" / "no such host".
    if m.contains("dns")
        || m.contains("lookup address")
        || m.contains("no such host")
        || m.contains("name or service not known")
        || m.contains("nodename nor servname")
    {
        return Diagnosis::DnsFailure;
    }
    // TLS — handshake, certificate, alert, invalid record.
    if m.contains("tls")
        || m.contains("certificate")
        || m.contains("handshake")
        || m.contains("ssl")
    {
        return Diagnosis::TlsError;
    }
    // Connection refused / reset / network unreachable.
    if m.contains("refused")
        || m.contains("reset by peer")
        || m.contains("network is unreachable")
        || m.contains("no route to host")
    {
        return Diagnosis::ConnectionRefused;
    }
    Diagnosis::Unknown
}

/// First-run reachability summary for the network-detection panel.
/// Probes `api.anthropic.com` only — the panel's question is "can the
/// user use Anthropic from this network", which is answered by the
/// canonical hot-path host alone. Probing all three HOTPATH hosts
/// would muddy the signal: `claude.ai` reachability is a separate
/// concern (auth flow), and `platform.claude.com` failures while
/// `api.anthropic.com` is up almost never happen in practice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicDiagnosis {
    pub diagnosis: Diagnosis,
    /// Round-trip in milliseconds when reachable. `None` for any
    /// non-`Reachable` outcome.
    pub latency_ms: Option<u32>,
    /// Redacted error string for the diagnostics pane. `None` when
    /// reachable. The panel itself never displays this — copy is
    /// driven by the `diagnosis` variant — but the Settings →
    /// Network pane includes it for users debugging.
    pub message: Option<String>,
}

/// Map an HTTP status to a [`Diagnosis`]. 5xx is a service-side
/// failure (api.anthropic.com is up but degraded — Statuspage
/// territory); everything else (2xx, 3xx, 4xx including the
/// expected unauthenticated-HEAD 401) means the network path is
/// alive. Pulled out so the mapping is unit-testable without a
/// real HTTP client.
pub fn diagnose_status(status: u16) -> Diagnosis {
    if (500..600).contains(&status) {
        Diagnosis::HttpError
    } else {
        Diagnosis::Reachable
    }
}

/// Single-host probe against `api.anthropic.com` with structured
/// diagnosis. Distinct from [`probe_one`] because it inspects the
/// HTTP status to populate [`Diagnosis::HttpError`] for 5xx
/// responses — the audit-flagged case where Anthropic is reachable
/// but degraded would otherwise collapse to "Reachable" silently.
pub async fn diagnose_anthropic() -> AnthropicDiagnosis {
    let host = HOTPATH_HOSTS[0]; // api.anthropic.com — see HOTPATH_HOSTS docstring.
    debug_assert_eq!(host.name, "api.anthropic.com");
    let client = build_client();
    let start = Instant::now();
    match client.head(host.url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let diagnosis = diagnose_status(status);
            let message = if matches!(diagnosis, Diagnosis::HttpError) {
                Some(format!("HTTP {status} from api.anthropic.com"))
            } else {
                None
            };
            AnthropicDiagnosis {
                diagnosis,
                latency_ms: Some(ms),
                message,
            }
        }
        Err(e) if e.is_timeout() => AnthropicDiagnosis {
            diagnosis: Diagnosis::Timeout,
            latency_ms: None,
            message: Some("connection timed out".to_string()),
        },
        Err(e) => {
            let message = redact(&e.to_string());
            AnthropicDiagnosis {
                diagnosis: classify_error(&message),
                latency_ms: None,
                message: Some(message),
            }
        }
    }
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

    // -----------------------------------------------------------------
    // First-run diagnosis
    // -----------------------------------------------------------------

    #[test]
    fn classify_error_dns_signatures() {
        // Common shapes across reqwest / hyper / rustls versions.
        for s in [
            "dns error: failed to lookup address",
            "DNS error",
            "no such host (os error 8)",
            "Name or service not known",
            "nodename nor servname provided",
        ] {
            assert_eq!(
                classify_error(s),
                Diagnosis::DnsFailure,
                "expected DnsFailure for: {s}"
            );
        }
    }

    #[test]
    fn classify_error_tls_signatures() {
        for s in [
            "tls handshake eof",
            "Certificate verify failed",
            "ssl alert: bad certificate",
            "handshake timed out",
        ] {
            assert_eq!(
                classify_error(s),
                Diagnosis::TlsError,
                "expected TlsError for: {s}"
            );
        }
    }

    #[test]
    fn classify_error_connection_refused_signatures() {
        for s in [
            "connection refused",
            "Connection reset by peer",
            "Network is unreachable",
            "no route to host",
        ] {
            assert_eq!(
                classify_error(s),
                Diagnosis::ConnectionRefused,
                "expected ConnectionRefused for: {s}"
            );
        }
    }

    #[test]
    fn classify_error_unknown_falls_through() {
        // Generic / unrecognized shape → Unknown, not a guess.
        assert_eq!(classify_error("something went wrong"), Diagnosis::Unknown);
        assert_eq!(classify_error(""), Diagnosis::Unknown);
    }

    #[test]
    fn diagnosis_reachable_predicate() {
        assert!(Diagnosis::Reachable.reachable());
        assert!(!Diagnosis::DnsFailure.reachable());
        assert!(!Diagnosis::Timeout.reachable());
        assert!(!Diagnosis::ConnectionRefused.reachable());
        assert!(!Diagnosis::TlsError.reachable());
        assert!(!Diagnosis::HttpError.reachable());
        assert!(!Diagnosis::Unknown.reachable());
    }

    #[test]
    fn diagnose_anthropic_probes_first_hotpath_host() {
        // The function relies on HOTPATH_HOSTS[0] being
        // api.anthropic.com. Lock that ordering down so a future
        // reordering doesn't silently change which host the panel
        // probes.
        assert_eq!(HOTPATH_HOSTS[0].name, "api.anthropic.com");
    }

    #[test]
    fn diagnose_status_maps_5xx_to_http_error() {
        // The audit-flagged case: Anthropic reachable but degraded.
        // Without the status check, this would collapse to Reachable
        // and the user would see "everything's fine" when it isn't.
        assert_eq!(diagnose_status(500), Diagnosis::HttpError);
        assert_eq!(diagnose_status(502), Diagnosis::HttpError);
        assert_eq!(diagnose_status(503), Diagnosis::HttpError);
        assert_eq!(diagnose_status(599), Diagnosis::HttpError);
    }

    #[test]
    fn diagnose_status_maps_success_redirect_and_4xx_to_reachable() {
        // 2xx — fine. 3xx — fine (we don't follow). 4xx — fine
        // (401 is the expected unauthenticated-HEAD response).
        assert_eq!(diagnose_status(200), Diagnosis::Reachable);
        assert_eq!(diagnose_status(204), Diagnosis::Reachable);
        assert_eq!(diagnose_status(301), Diagnosis::Reachable);
        assert_eq!(diagnose_status(401), Diagnosis::Reachable);
        assert_eq!(diagnose_status(404), Diagnosis::Reachable);
        assert_eq!(diagnose_status(429), Diagnosis::Reachable);
    }
}
