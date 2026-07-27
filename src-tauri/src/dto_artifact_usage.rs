//! DTOs for artifact-usage telemetry — counts and outcomes for
//! invocations of installed CC artifacts (skills, hooks, agents,
//! slash commands).
//!
//! Distinct from `dto_usage`, which is account / rate-limit usage.
//! Naming: `Artifact*` prefix keeps the noun unambiguous in the JS
//! bridge.

use claudepot_core::artifact_usage::{ArtifactKind, UsageListRow, UsageStats};
use serde::Serialize;

/// Mirrors `claudepot_core::artifact_usage::UsageStats` with the same
/// field names so the JS side gets a stable shape.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArtifactUsageStatsDto {
    pub count_24h: u64,
    pub count_7d: u64,
    pub count_30d: u64,
    pub error_count_30d: u64,
    pub last_seen_ms: Option<i64>,
    pub p50_ms_24h: Option<u64>,
    pub avg_ms_30d: Option<u64>,
}

impl From<UsageStats> for ArtifactUsageStatsDto {
    fn from(s: UsageStats) -> Self {
        Self {
            count_24h: s.count_24h,
            count_7d: s.count_7d,
            count_30d: s.count_30d,
            error_count_30d: s.error_count_30d,
            last_seen_ms: s.last_seen_ms,
            p50_ms_24h: s.p50_ms_24h,
            avg_ms_30d: s.avg_ms_30d,
        }
    }
}

/// One row of the "Usage" rollup table, used by both the Activity
/// subview (Slice 3) and the per-artifact filter API.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactUsageRowDto {
    /// "skill" / "hook" / "agent" / "command".
    pub kind: String,
    pub artifact_key: String,
    pub plugin_id: Option<String>,
    pub stats: ArtifactUsageStatsDto,
}

impl From<UsageListRow> for ArtifactUsageRowDto {
    fn from(r: UsageListRow) -> Self {
        Self {
            kind: r.kind.as_str().to_string(),
            artifact_key: r.artifact_key,
            plugin_id: r.plugin_id,
            stats: r.stats.into(),
        }
    }
}

/// Wire shape for `artifact_usage_batch` — the Config-tree path uses
/// it to fetch many keys in one round-trip. The result is
/// position-aligned with the input.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactUsageBatchEntryDto {
    pub kind: String,
    pub artifact_key: String,
    pub stats: ArtifactUsageStatsDto,
}

/// One row of the durable "ever observed" ledger — the set the Unused
/// view subtracts from the installed inventory.
///
/// Deliberately carries no counts. The question this answers is
/// binary ("has this ever fired?"); adding a count would invite the
/// caller to re-derive a windowed answer from a table that does not
/// support one.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactEverFiredDto {
    pub kind: String,
    pub artifact_key: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

/// One row of the Unused view.
#[derive(Debug, Clone, Serialize)]
pub struct UnusedArtifactDto {
    pub kind: String,
    /// Config-tree node id — the UI navigates to Global → Config with
    /// `subRoute = "node:<node_id>"`.
    pub node_id: String,
    pub artifact_key: String,
    pub plugin_id: Option<String>,
    pub label: String,
    pub abs_path: String,
    /// Filesystem **modification** time in ms — not install time.
    pub modified_ms: i64,
}

/// Unused-view payload. Carries the suppression counts so the pane's
/// summary line reconciles instead of silently dropping rows.
#[derive(Debug, Clone, Serialize)]
pub struct UnusedReportDto {
    pub rows: Vec<UnusedArtifactDto>,
    pub installed_count: usize,
    pub suppressed_recent: usize,
    pub suppressed_disabled: usize,
    /// Grace window in days, so the UI states the same number core used
    /// rather than hard-coding a second copy.
    pub grace_days: i64,
}

impl From<claudepot_core::artifact_usage::unused::UnusedReport> for UnusedReportDto {
    fn from(r: claudepot_core::artifact_usage::unused::UnusedReport) -> Self {
        Self {
            rows: r
                .rows
                .into_iter()
                .map(|a| UnusedArtifactDto {
                    kind: a.kind,
                    node_id: a.node_id,
                    artifact_key: a.artifact_key,
                    plugin_id: a.plugin_id,
                    label: a.label,
                    abs_path: a.abs_path,
                    modified_ms: a.modified_ms,
                })
                .collect(),
            installed_count: r.installed_count,
            suppressed_recent: r.suppressed_recent,
            suppressed_disabled: r.suppressed_disabled,
            grace_days: claudepot_core::artifact_usage::unused::RECENTLY_MODIFIED_GRACE_DAYS,
        }
    }
}

/// Parse a wire `kind` string into the core enum. Returns a Tauri
/// command-friendly `Result` so handlers can short-circuit invalid
/// inputs without a panic.
pub fn parse_kind(s: &str) -> Result<ArtifactKind, String> {
    ArtifactKind::parse(s).ok_or_else(|| format!("unknown artifact kind: {s}"))
}
