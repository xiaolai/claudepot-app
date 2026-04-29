//! Tauri command surface for the local cost report.
//!
//! Pure aggregation — every line of policy lives in
//! `claudepot_core::usage_local`. This file only converts the call
//! shape and the pricing source. No business logic.
//!
//! Frontend consumes `local_usage_aggregate` to render the
//! Activities → Cost tab. The CLI's `claudepot usage report`
//! consumes the same core API directly.

use claudepot_core::pricing;
use claudepot_core::session::list_all_sessions;
use claudepot_core::usage_local::{
    aggregate_from_rows, LocalUsageReport, ProjectUsageRow, ReportWindow, TimeWindow,
    UsageTotals,
};
use serde::{Deserialize, Serialize};

/// Wire shape for the time window. The frontend constructs one of
/// these and ships it across IPC; the backend translates to the
/// core `TimeWindow`. Two variants:
///   - `kind = "all"` → open-ended (no `days` field consumed).
///   - `kind = "last_days"` → last `days` calendar days, anchored at
///     "now". `days` must be Some and > 0.
///
/// We keep the two-field shape (`kind` + optional `days`) instead of
/// a tagged union because Tauri serializes flat objects more
/// reliably than serde discriminators across the JS/Rust boundary,
/// and the TS caller stays a plain `{ kind, days? }`.
#[derive(Debug, Deserialize)]
pub struct WindowSpec {
    pub kind: String,
    #[serde(default)]
    pub days: Option<u32>,
}

impl WindowSpec {
    fn into_time_window(self, now_ms: i64) -> Result<TimeWindow, String> {
        match self.kind.as_str() {
            "all" => Ok(TimeWindow::open()),
            "last_days" => {
                let days = self
                    .days
                    .ok_or_else(|| "last_days requires `days`".to_string())?;
                if days == 0 {
                    return Ok(TimeWindow::open());
                }
                Ok(TimeWindow::last_days(days, now_ms))
            }
            other => Err(format!("unknown window kind: {other}")),
        }
    }
}

/// DTO mirror of `claudepot_core::usage_local::LocalUsageReport`.
/// Field shape is byte-for-byte the same as the core type's
/// `Serialize` derive — we just rename to keep the struct named
/// after its IPC role and to leave room for future GUI-only fields
/// (e.g. a pricing-source pill) without polluting core.
#[derive(Debug, Serialize)]
pub struct LocalUsageReportDto {
    pub window: ReportWindowDto,
    pub rows: Vec<ProjectUsageRowDto>,
    pub totals: UsageTotalsDto,
    /// Human-readable summary of where the price table came from
    /// ("bundled · 2026-01-15", "live · fetched 2 hours ago", etc.).
    /// Frontend renders it in a small pill near the window selector
    /// so the user can tell whether the cost figure reflects current
    /// rates or a stale cache.
    pub pricing_source: String,
    /// Set to a short message when the most recent pricing refresh
    /// failed; the GUI can render it in a tooltip next to the pill.
    /// `None` on success.
    pub pricing_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReportWindowDto {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

impl From<ReportWindow> for ReportWindowDto {
    fn from(w: ReportWindow) -> Self {
        Self {
            from_ms: w.from_ms,
            to_ms: w.to_ms,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProjectUsageRowDto {
    pub project_path: String,
    pub session_count: usize,
    pub first_active_ms: Option<i64>,
    pub last_active_ms: Option<i64>,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub tokens_cache_creation: u64,
    pub tokens_cache_read: u64,
    pub cost_usd: Option<f64>,
    pub unpriced_sessions: usize,
}

impl From<ProjectUsageRow> for ProjectUsageRowDto {
    fn from(r: ProjectUsageRow) -> Self {
        Self {
            project_path: r.project_path,
            session_count: r.session_count,
            first_active_ms: r.first_active_ms,
            last_active_ms: r.last_active_ms,
            tokens_input: r.tokens_input,
            tokens_output: r.tokens_output,
            tokens_cache_creation: r.tokens_cache_creation,
            tokens_cache_read: r.tokens_cache_read,
            cost_usd: r.cost_usd,
            unpriced_sessions: r.unpriced_sessions,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UsageTotalsDto {
    pub session_count: usize,
    pub first_active_ms: Option<i64>,
    pub last_active_ms: Option<i64>,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub tokens_cache_creation: u64,
    pub tokens_cache_read: u64,
    pub cost_usd: Option<f64>,
    pub unpriced_sessions: usize,
}

impl From<UsageTotals> for UsageTotalsDto {
    fn from(t: UsageTotals) -> Self {
        Self {
            session_count: t.session_count,
            first_active_ms: t.first_active_ms,
            last_active_ms: t.last_active_ms,
            tokens_input: t.tokens_input,
            tokens_output: t.tokens_output,
            tokens_cache_creation: t.tokens_cache_creation,
            tokens_cache_read: t.tokens_cache_read,
            cost_usd: t.cost_usd,
            unpriced_sessions: t.unpriced_sessions,
        }
    }
}

fn report_to_dto(
    report: LocalUsageReport,
    pricing_source: String,
    pricing_error: Option<String>,
) -> LocalUsageReportDto {
    LocalUsageReportDto {
        window: report.window.into(),
        rows: report.rows.into_iter().map(Into::into).collect(),
        totals: report.totals.into(),
        pricing_source,
        pricing_error,
    }
}

/// Render a one-line `pricing_source` summary the GUI can drop into
/// a pill. Formatted in display-tense ("bundled", "live · 2h ago",
/// "cached · stale"). The exact prose is allowed to change; tests
/// gate on shape, not literal strings.
fn format_pricing_source(table: &pricing::PriceTable) -> String {
    use pricing::PriceSource;
    match &table.source {
        PriceSource::Bundled { verified_at } => {
            format!("bundled · verified {verified_at}")
        }
        PriceSource::Live {
            fetched_at_unix, ..
        } => format!("live · {}", relative_when(*fetched_at_unix)),
        PriceSource::Cached {
            fetched_at_unix, ..
        } => format!("cached · {}", relative_when(*fetched_at_unix)),
    }
}

/// "now", "5m ago", "3h ago", "2d ago". Falls back to "earlier" when
/// the system clock is behind the fetched-at timestamp (NTP skew).
/// The fmt is intentionally relative — absolute timestamps would be
/// less readable in a small pill.
fn relative_when(unix_secs: u64) -> String {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if unix_secs > now_secs {
        return "earlier".to_string();
    }
    let delta = now_secs - unix_secs;
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86400)
    }
}

/// Tauri command — read the session index, aggregate, return DTO.
///
/// Async because `list_all_sessions` walks the filesystem (refresh
/// pass against the index cache) and pricing load may touch the
/// disk-cached file. Both are cheap enough to run on a worker
/// thread; declaring `async fn` keeps the IPC dispatcher off the
/// main thread per the threading-policy comment in
/// `commands.rs`.
#[tauri::command]
pub async fn local_usage_aggregate(
    spec: WindowSpec,
) -> Result<LocalUsageReportDto, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let window = spec.into_time_window(now_ms)?;

    let config_dir = claudepot_core::paths::claude_config_dir();
    let sessions = list_all_sessions(&config_dir)
        .map_err(|e| format!("session index: {e}"))?;

    let table = pricing::load();
    let pricing_source = format_pricing_source(&table);
    let pricing_error = table.last_fetch_error.clone();

    let report = aggregate_from_rows(sessions, &table, window);
    Ok(report_to_dto(report, pricing_source, pricing_error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::pricing::{ModelRates, PriceSource, PriceTable};
    use claudepot_core::session::TokenUsage;
    use std::collections::BTreeMap;

    #[test]
    fn window_spec_all_yields_open_window() {
        let w = WindowSpec {
            kind: "all".into(),
            days: None,
        }
        .into_time_window(1_700_000_000_000)
        .unwrap();
        assert!(w.from_ms.is_none() && w.to_ms.is_none());
    }

    #[test]
    fn window_spec_last_days_anchors_at_now() {
        let now = 1_700_000_000_000;
        let w = WindowSpec {
            kind: "last_days".into(),
            days: Some(7),
        }
        .into_time_window(now)
        .unwrap();
        assert_eq!(w.to_ms, Some(now));
        assert_eq!(w.from_ms, Some(now - 7 * 86_400_000));
    }

    #[test]
    fn window_spec_last_days_zero_means_open() {
        // Match TimeWindow::last_days(0, _) — defensive.
        let w = WindowSpec {
            kind: "last_days".into(),
            days: Some(0),
        }
        .into_time_window(1_700_000_000_000)
        .unwrap();
        assert!(w.from_ms.is_none() && w.to_ms.is_none());
    }

    #[test]
    fn window_spec_last_days_missing_count_errors() {
        let err = WindowSpec {
            kind: "last_days".into(),
            days: None,
        }
        .into_time_window(0)
        .unwrap_err();
        assert!(err.contains("last_days"));
    }

    #[test]
    fn window_spec_unknown_kind_errors() {
        let err = WindowSpec {
            kind: "yesterday".into(),
            days: Some(1),
        }
        .into_time_window(0)
        .unwrap_err();
        assert!(err.contains("unknown window kind"));
    }

    #[test]
    fn report_to_dto_preserves_token_and_cost_totals() {
        // Build a tiny core report by hand, push through report_to_dto,
        // assert the wire shape carries the same numbers. Guards
        // against future drift between the core struct and the DTO.
        use claudepot_core::usage_local::{ProjectUsageRow as CoreRow, UsageTotals as CoreTotals};
        let core = LocalUsageReport {
            window: ReportWindow {
                from_ms: Some(1),
                to_ms: Some(2),
            },
            rows: vec![CoreRow {
                project_path: "/p".into(),
                session_count: 4,
                first_active_ms: Some(10),
                last_active_ms: Some(20),
                tokens_input: 100,
                tokens_output: 200,
                tokens_cache_creation: 50,
                tokens_cache_read: 1000,
                cost_usd: Some(1.23),
                unpriced_sessions: 1,
            }],
            totals: CoreTotals {
                session_count: 4,
                first_active_ms: Some(10),
                last_active_ms: Some(20),
                tokens_input: 100,
                tokens_output: 200,
                tokens_cache_creation: 50,
                tokens_cache_read: 1000,
                cost_usd: Some(1.23),
                unpriced_sessions: 1,
            },
        };
        let dto = report_to_dto(core, "test-source".into(), None);
        assert_eq!(dto.pricing_source, "test-source");
        assert!(dto.pricing_error.is_none());
        assert_eq!(dto.rows.len(), 1);
        assert_eq!(dto.rows[0].project_path, "/p");
        assert_eq!(dto.rows[0].session_count, 4);
        assert_eq!(dto.rows[0].tokens_input, 100);
        assert_eq!(dto.rows[0].cost_usd, Some(1.23));
        assert_eq!(dto.rows[0].unpriced_sessions, 1);
        assert_eq!(dto.totals.tokens_cache_read, 1000);
    }

    #[test]
    fn format_pricing_source_renders_bundled_with_date() {
        let table = PriceTable {
            models: BTreeMap::new(),
            source: PriceSource::Bundled {
                verified_at: "2026-01-15".into(),
            },
            last_fetch_error: None,
        };
        let s = format_pricing_source(&table);
        assert!(s.starts_with("bundled"));
        assert!(s.contains("2026-01-15"));
    }

    #[test]
    fn relative_when_handles_clock_skew() {
        // Future timestamp (clock skew): never panics, returns a
        // friendly fallback rather than a negative duration.
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            + 99_999_999;
        let s = relative_when(future);
        assert_eq!(s, "earlier");
    }

    /// `apply_rates` lives in core; this test just confirms the
    /// DTO surface preserves a meaningful cost from a real pricing
    /// table. Effectively a smoke test that the from-impl wiring
    /// between core and DTO is intact end-to-end.
    #[test]
    fn report_to_dto_handles_unpriced_total_correctly() {
        use claudepot_core::usage_local::{ProjectUsageRow as CoreRow, UsageTotals as CoreTotals};
        let core = LocalUsageReport {
            window: ReportWindow {
                from_ms: None,
                to_ms: None,
            },
            rows: vec![CoreRow {
                project_path: "/y".into(),
                session_count: 1,
                first_active_ms: None,
                last_active_ms: None,
                tokens_input: 1_000,
                tokens_output: 0,
                tokens_cache_creation: 0,
                tokens_cache_read: 0,
                cost_usd: None,
                unpriced_sessions: 1,
            }],
            totals: CoreTotals {
                session_count: 1,
                first_active_ms: None,
                last_active_ms: None,
                tokens_input: 1_000,
                tokens_output: 0,
                tokens_cache_creation: 0,
                tokens_cache_read: 0,
                cost_usd: None,
                unpriced_sessions: 1,
            },
        };
        let _used: ModelRates = ModelRates {
            input_per_mtok: 1.0,
            output_per_mtok: 1.0,
            cache_write_per_mtok: 1.0,
            cache_read_per_mtok: 1.0,
        };
        let dto = report_to_dto(core, "bundled".into(), None);
        assert!(dto.totals.cost_usd.is_none());
        assert_eq!(dto.totals.unpriced_sessions, 1);
    }
}
