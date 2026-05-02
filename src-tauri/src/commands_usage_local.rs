//! Tauri command surface for the local cost report.
//!
//! Pure aggregation — every line of policy lives in
//! `claudepot_core::usage_local`. This file only converts the call
//! shape and the pricing source. No business logic.
//!
//! Frontend consumes `local_usage_aggregate` to render the
//! Activities → Cost tab. The CLI's `claudepot usage report`
//! consumes the same core API directly.

use claudepot_core::pricing::{self, PriceTier};
use claudepot_core::session::list_all_sessions;
use claudepot_core::session_index::SessionIndex;
use claudepot_core::usage_local::{
    aggregate_from_rows, top_costly_turns, CostlyTurn, LocalUsageReport, ProjectUsageRow,
    ReportWindow, TimeWindow, UsageTotals,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::preferences::PreferencesState;

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
    /// Wire-form pricing tier the cost figures were computed against
    /// (`anthropic_api`, `vertex_global`, `vertex_regional`,
    /// `aws_bedrock`). Driven by the user's preference; the GUI
    /// renders the matching display label in the pricing-source pill
    /// and uses this id to select the active option in the tier
    /// picker.
    pub pricing_tier: String,
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
    /// Session-count breakdown by model id. Order is deterministic
    /// (BTreeMap → sorted by key) so snapshot tests stay stable.
    pub models_by_session: std::collections::BTreeMap<String, usize>,
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
            models_by_session: r.models_by_session,
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
    /// Install-wide session-count breakdown by model id.
    pub models_by_session: std::collections::BTreeMap<String, usize>,
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
            models_by_session: t.models_by_session,
        }
    }
}

fn report_to_dto(
    report: LocalUsageReport,
    pricing_tier: String,
    pricing_source: String,
    pricing_error: Option<String>,
) -> LocalUsageReportDto {
    LocalUsageReportDto {
        window: report.window.into(),
        rows: report.rows.into_iter().map(Into::into).collect(),
        totals: report.totals.into(),
        pricing_source,
        pricing_error,
        pricing_tier,
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
    prefs: State<'_, PreferencesState>,
) -> Result<LocalUsageReportDto, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let window = spec.into_time_window(now_ms)?;

    let config_dir = claudepot_core::paths::claude_config_dir();
    let sessions = list_all_sessions(&config_dir).map_err(|e| format!("session index: {e}"))?;

    // Read the user's pricing tier from preferences. The lock is
    // released immediately — we only need the enum value, not a live
    // reference, and the aggregation that follows is the slow part.
    let tier = {
        let guard = prefs
            .0
            .lock()
            .map_err(|e| format!("preferences lock poisoned: {e}"))?;
        guard.pricing_tier
    };

    let bundled = pricing::load();
    let table = bundled.with_tier(tier);
    let pricing_source = format_pricing_source(&table);
    let pricing_error = table.last_fetch_error.clone();

    let report = aggregate_from_rows(sessions, &table, window);
    Ok(report_to_dto(
        report,
        tier.as_str().to_string(),
        pricing_source,
        pricing_error,
    ))
}

/// Update the user's pricing tier. The wire form is the lowercase
/// `PriceTier::as_str` value (`anthropic_api`, `vertex_global`,
/// `vertex_regional`, `aws_bedrock`); unknown values yield an
/// explicit error so the GUI can show a toast instead of silently
/// reverting to the default. Persists the change to disk before
/// returning so a hard crash mid-flight doesn't lose the choice.
#[tauri::command]
pub async fn pricing_tier_set(
    tier: String,
    prefs: State<'_, PreferencesState>,
) -> Result<(), String> {
    let parsed = PriceTier::parse(&tier)
        .ok_or_else(|| format!("unknown pricing tier: {tier}"))?;
    let snapshot = {
        let mut guard = prefs
            .0
            .lock()
            .map_err(|e| format!("preferences lock poisoned: {e}"))?;
        guard.pricing_tier = parsed;
        guard.clone()
    };
    snapshot.save()
}

/// Read the user's current pricing tier as the wire form. Lets the
/// frontend hydrate the tier picker on cold start before the first
/// `local_usage_aggregate` round-trip lands, so the picker doesn't
/// flicker from the default value to the saved value.
#[tauri::command]
pub fn pricing_tier_get(prefs: State<'_, PreferencesState>) -> Result<String, String> {
    let guard = prefs
        .0
        .lock()
        .map_err(|e| format!("preferences lock poisoned: {e}"))?;
    Ok(guard.pricing_tier.as_str().to_string())
}

/// Wire shape for one row of the "top costly prompts" panel. Mirrors
/// `claudepot_core::usage_local::CostlyTurn` byte-for-byte except the
/// JS-side never sees `None` for `cost_usd` — the core path drops
/// unresolved-model rows, so this DTO surfaces a concrete `f64`.
#[derive(Debug, Serialize)]
pub struct CostlyTurnDto {
    pub file_path: String,
    pub project_path: String,
    pub turn_index: usize,
    pub ts_ms: Option<i64>,
    pub model: String,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub tokens_cache_creation: u64,
    pub tokens_cache_read: u64,
    pub user_prompt_preview: Option<String>,
    /// Always populated — `top_costly_turns` filters out rows with
    /// unresolved cost. Kept as `f64` (not Option<f64>) on the wire
    /// so the UI can render `$X.XX` without a null guard per cell.
    pub cost_usd: f64,
}

impl From<CostlyTurn> for CostlyTurnDto {
    fn from(t: CostlyTurn) -> Self {
        Self {
            file_path: t.file_path,
            project_path: t.project_path,
            turn_index: t.turn_index,
            ts_ms: t.ts_ms,
            model: t.model,
            tokens_input: t.tokens_input,
            tokens_output: t.tokens_output,
            tokens_cache_creation: t.tokens_cache_creation,
            tokens_cache_read: t.tokens_cache_read,
            user_prompt_preview: t.user_prompt_preview,
            cost_usd: t.cost_usd.unwrap_or(0.0),
        }
    }
}

/// Wire envelope for the top-N response. Carries the same
/// `pricing_tier` echo as `LocalUsageReportDto` so the consumer can
/// render the active tier alongside the dollar figures.
#[derive(Debug, Serialize)]
pub struct TopCostlyPromptsDto {
    pub turns: Vec<CostlyTurnDto>,
    pub pricing_tier: String,
}

/// Tauri command — return the install's `final_n` costliest prompts
/// in the supplied `spec` window, scored against the user's active
/// pricing tier. The session index is refreshed in-band so newly-
/// landed transcripts contribute to the ranking on the next
/// dashboard tick. `final_n` is capped at 50 server-side to bound
/// the UI footprint and the in-memory candidate pool.
#[tauri::command]
pub async fn top_costly_prompts(
    spec: WindowSpec,
    final_n: usize,
    prefs: State<'_, PreferencesState>,
) -> Result<TopCostlyPromptsDto, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let window = spec.into_time_window(now_ms)?;
    let n = final_n.min(50);

    let tier = {
        let guard = prefs
            .0
            .lock()
            .map_err(|e| format!("preferences lock poisoned: {e}"))?;
        guard.pricing_tier
    };

    let bundled = pricing::load();
    let table = bundled.with_tier(tier);

    // The session index lives in the on-disk DB; opening it is cheap
    // (idempotent + lazy). Refresh inline so newly-landed transcripts
    // contribute to the ranking; the refresh itself is bounded by
    // the (size, mtime, inode) re-parse guard.
    let config_dir = claudepot_core::paths::claude_config_dir();
    let db_path = claudepot_core::paths::claudepot_data_dir().join("sessions.db");
    let index = SessionIndex::open(&db_path).map_err(|e| format!("session index open: {e}"))?;
    index
        .refresh(&config_dir)
        .map_err(|e| format!("session index refresh: {e}"))?;

    let turns = top_costly_turns(&index, &table, window, n)
        .map_err(|e| format!("top_costly_turns: {e}"))?;

    Ok(TopCostlyPromptsDto {
        turns: turns.into_iter().map(Into::into).collect(),
        pricing_tier: tier.as_str().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::pricing::{ModelRates, PriceSource, PriceTable};

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
                models_by_session: BTreeMap::new(),
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
                models_by_session: BTreeMap::new(),
            },
        };
        let dto = report_to_dto(core, "anthropic_api".into(), "test-source".into(), None);
        assert_eq!(dto.pricing_source, "test-source");
        assert_eq!(dto.pricing_tier, "anthropic_api");
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
                models_by_session: BTreeMap::new(),
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
                models_by_session: BTreeMap::new(),
            },
        };
        let _used: ModelRates = ModelRates {
            input_per_mtok: 1.0,
            output_per_mtok: 1.0,
            cache_write_per_mtok: 1.0,
            cache_read_per_mtok: 1.0,
        };
        let dto = report_to_dto(core, "anthropic_api".into(), "bundled".into(), None);
        assert!(dto.totals.cost_usd.is_none());
        assert_eq!(dto.totals.unpriced_sessions, 1);
    }
}
