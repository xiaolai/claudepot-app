//! Local cost-and-token aggregation derived from CC's on-disk session
//! transcripts. Mirrors what CC's own `/usage` report does for the
//! "this install" totals — token counts and dollar costs computed
//! locally, with no extra network call.
//!
//! # Why this lives in core
//!
//! CC writes per-message `usage` blocks into every `.jsonl` transcript;
//! `session_index` already captures session-aggregate token counts as
//! part of its scan/refresh cycle. This module is a *consumer* of that
//! index plus the `pricing` module: it joins per-session token totals
//! against the model-rate catalog and rolls up by project.
//!
//! Multi-account attribution is out of scope. Claudepot doesn't keep a
//! swap-event log, so there's no reliable way to bind a JSONL written
//! at time T to the account that owned the CLI slot at T. The
//! aggregation is per-project (and per-install) only — the "who paid"
//! question is intentionally not answered. Adding swap history is a
//! separate piece of infrastructure.
//!
//! # Cost approximation
//!
//! `session_index` stores token totals per *session*, not per *(session,
//! model)*. A session that mixed Opus and Sonnet has one row of
//! aggregate token counts and a `models_json` array of every model
//! seen. We pick a "dominant" model by alphabetical order from that
//! list and apply its rate to all tokens. In practice CC sessions rarely
//! mix models, so this is exact for the common case and an approximation
//! for the long tail. A schema bump to `tokens_input_by_model` would
//! make it exact at all times; that change is reserved for a follow-up.
//!
//! Sessions whose model isn't in the price table (or whose `models_json`
//! is empty — e.g. user-only sessions with no assistant turn yet) are
//! reported with `tokens=…, cost_usd=None`. The caller decides how to
//! render: claudepot's CLI renders `n/a` for the cost column and the
//! token columns still tell the truth.

use crate::pricing::{resolve_model_rates, ModelRates, PriceTable};
use crate::session::SessionRow;
use crate::session_index::error::SessionIndexError;
use crate::session_index::SessionIndex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Inclusive [from, to] window on `last_ts_ms` (ms-since-epoch). `None`
/// at either end means open-ended on that side. Used for "last 7 days"
/// / "last 30 days" / "all time" filters from the CLI.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeWindow {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

impl TimeWindow {
    pub fn open() -> Self {
        Self::default()
    }

    /// Last `days` days, anchored at `now_ms`. Convenience for CLI
    /// flags like `--window 7d`. Returns an open-ended window if
    /// `days == 0` (interpreted as "all time").
    pub fn last_days(days: u32, now_ms: i64) -> Self {
        if days == 0 {
            return Self::open();
        }
        let span_ms: i64 = (days as i64) * 24 * 60 * 60 * 1000;
        Self {
            from_ms: Some(now_ms - span_ms),
            to_ms: Some(now_ms),
        }
    }

    /// True iff the supplied last-ts ms-since-epoch falls inside the
    /// window. A session with no `last_ts_ms` (empty transcript) is
    /// excluded — there's nothing meaningful to attribute its tokens
    /// to time-wise.
    pub fn contains(&self, ts_ms: Option<i64>) -> bool {
        let Some(ts) = ts_ms else { return false };
        if let Some(f) = self.from_ms {
            if ts < f {
                return false;
            }
        }
        if let Some(t) = self.to_ms {
            if ts > t {
                return false;
            }
        }
        true
    }
}

/// One row of the per-project aggregation. Fields are designed so the
/// CLI can render a wide table directly, and the GUI (when it lands)
/// can drop fields it doesn't surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsageRow {
    /// Absolute project path (CWD). Identical key shape to the rest of
    /// the project surface so the row joins cleanly with `project_show`
    /// and friends.
    pub project_path: String,
    /// Number of session transcripts contributing to this row.
    pub session_count: usize,
    /// Earliest `last_ts_ms` across the contributing sessions, in
    /// ms-since-epoch. `None` if every session has a missing last_ts.
    pub first_active_ms: Option<i64>,
    /// Latest `last_ts_ms` across the contributing sessions.
    pub last_active_ms: Option<i64>,
    /// Sum of every contributing session's `tokens.input`.
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub tokens_cache_creation: u64,
    pub tokens_cache_read: u64,
    /// Sum of dollar costs across contributing sessions. `None` only
    /// when *every* contributing session lacked a model that the price
    /// table could resolve (e.g. all user-only sessions). One unmatched
    /// session does NOT zero out the total — the caller can compare
    /// `tokens_*` against `cost_usd` to detect partial matches.
    pub cost_usd: Option<f64>,
    /// Sessions whose models couldn't be priced. Never zero when
    /// `cost_usd` is None and tokens are non-zero.
    pub unpriced_sessions: usize,
}

/// Total roll-up across every project row. Same fields as a row,
/// minus `project_path`. Useful for the CLI footer and the GUI summary
/// strip.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageTotals {
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

/// Output of `aggregate_local_usage`. Rows sort newest-first by
/// `last_active_ms` so a glance at the head of the list shows where
/// recent spend went.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalUsageReport {
    pub window: ReportWindow,
    pub rows: Vec<ProjectUsageRow>,
    pub totals: UsageTotals,
}

/// Mirror of the `TimeWindow` used at call time, but with both ends
/// always populated for the CLI's "Window: from … to …" header. `None`
/// is rendered as "—" by the CLI, "Open" by the GUI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReportWindow {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

impl From<TimeWindow> for ReportWindow {
    fn from(w: TimeWindow) -> Self {
        Self {
            from_ms: w.from_ms,
            to_ms: w.to_ms,
        }
    }
}

/// Build the per-project local usage report. Reads the session index,
/// filters by `window.last_active_ms`, groups by `project_path`, and
/// joins each session's dominant model against `prices` to compute
/// cost. The `config_dir` argument is forwarded to `SessionIndex::list_all`
/// so the index gets refreshed against any newly-written transcripts —
/// the call is therefore idempotent: running it twice in a row is a
/// no-op on the second call.
///
/// Returns `Ok(report)` even when the index is empty (zero rows). Errors
/// only on the underlying I/O / SQL paths.
pub fn aggregate_local_usage(
    index: &SessionIndex,
    config_dir: &Path,
    prices: &PriceTable,
    window: TimeWindow,
) -> Result<LocalUsageReport, SessionIndexError> {
    let sessions = index.list_all(config_dir)?;
    Ok(aggregate_from_rows(sessions, prices, window))
}

/// Pure variant of [`aggregate_local_usage`] that takes pre-loaded
/// rows. Exposed so tests can drive the aggregation without touching
/// disk and so future callers (e.g. a GUI surface that already holds
/// the rows in memory) can avoid a second `list_all`.
pub fn aggregate_from_rows(
    sessions: Vec<SessionRow>,
    prices: &PriceTable,
    window: TimeWindow,
) -> LocalUsageReport {
    let mut by_project: BTreeMap<String, ProjectAccumulator> = BTreeMap::new();
    let mut totals = UsageTotals::default();

    for s in sessions {
        let last_ms = s.last_ts.map(|t| t.timestamp_millis());
        if !window.contains(last_ms) {
            continue;
        }

        let session_cost = compute_session_cost(&s, prices);

        let acc = by_project.entry(s.project_path.clone()).or_default();
        acc.session_count += 1;
        acc.tokens_input += s.tokens.input;
        acc.tokens_output += s.tokens.output;
        acc.tokens_cache_creation += s.tokens.cache_creation;
        acc.tokens_cache_read += s.tokens.cache_read;
        merge_min(&mut acc.first_active_ms, last_ms);
        merge_max(&mut acc.last_active_ms, last_ms);
        match session_cost {
            Some(c) => *acc.cost_usd.get_or_insert(0.0) += c,
            None => acc.unpriced_sessions += 1,
        }

        totals.session_count += 1;
        totals.tokens_input += s.tokens.input;
        totals.tokens_output += s.tokens.output;
        totals.tokens_cache_creation += s.tokens.cache_creation;
        totals.tokens_cache_read += s.tokens.cache_read;
        merge_min(&mut totals.first_active_ms, last_ms);
        merge_max(&mut totals.last_active_ms, last_ms);
        match session_cost {
            Some(c) => *totals.cost_usd.get_or_insert(0.0) += c,
            None => totals.unpriced_sessions += 1,
        }
    }

    let mut rows: Vec<ProjectUsageRow> = by_project
        .into_iter()
        .map(|(path, acc)| ProjectUsageRow {
            project_path: path,
            session_count: acc.session_count,
            first_active_ms: acc.first_active_ms,
            last_active_ms: acc.last_active_ms,
            tokens_input: acc.tokens_input,
            tokens_output: acc.tokens_output,
            tokens_cache_creation: acc.tokens_cache_creation,
            tokens_cache_read: acc.tokens_cache_read,
            cost_usd: acc.cost_usd,
            unpriced_sessions: acc.unpriced_sessions,
        })
        .collect();
    // Newest-first by recent activity; ties broken by project path so
    // the order is deterministic across runs (matters for snapshot
    // tests + golden CLI output).
    rows.sort_by(|a, b| {
        b.last_active_ms
            .cmp(&a.last_active_ms)
            .then_with(|| a.project_path.cmp(&b.project_path))
    });

    LocalUsageReport {
        window: window.into(),
        rows,
        totals,
    }
}

#[derive(Default)]
struct ProjectAccumulator {
    session_count: usize,
    first_active_ms: Option<i64>,
    last_active_ms: Option<i64>,
    tokens_input: u64,
    tokens_output: u64,
    tokens_cache_creation: u64,
    tokens_cache_read: u64,
    cost_usd: Option<f64>,
    unpriced_sessions: usize,
}

/// Compute USD cost for one session's aggregate token totals using its
/// dominant model. Returns `None` when:
///   - the session has no models recorded (zero assistant turns), or
///   - the dominant model can't be resolved against the price table.
fn compute_session_cost(s: &SessionRow, prices: &PriceTable) -> Option<f64> {
    let model = dominant_model(&s.models)?;
    let rates = resolve_model_rates(prices, model)?;
    Some(apply_rates(&s.tokens, rates))
}

/// Pick the dominant model from a session's recorded model list. CC
/// sessions rarely mix models, so the typical input is a single-element
/// list. When several are present, we pick the alphabetically-first
/// for determinism — there's no per-message token attribution at the
/// session-row level to pick a "majority" by, so any deterministic
/// tiebreak is as good as another. Empty list → None.
fn dominant_model(models: &[String]) -> Option<&str> {
    models.iter().min().map(|s| s.as_str())
}

fn apply_rates(tokens: &crate::session::TokenUsage, r: &ModelRates) -> f64 {
    let m = 1_000_000.0;
    (tokens.input as f64 / m) * r.input_per_mtok
        + (tokens.output as f64 / m) * r.output_per_mtok
        + (tokens.cache_creation as f64 / m) * r.cache_write_per_mtok
        + (tokens.cache_read as f64 / m) * r.cache_read_per_mtok
}

fn merge_min(slot: &mut Option<i64>, candidate: Option<i64>) {
    let Some(c) = candidate else { return };
    *slot = Some(slot.map_or(c, |existing| existing.min(c)));
}

fn merge_max(slot: &mut Option<i64>, candidate: Option<i64>) {
    let Some(c) = candidate else { return };
    *slot = Some(slot.map_or(c, |existing| existing.max(c)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::{ModelRates, PriceSource, PriceTable};
    use crate::session::{SessionRow, TokenUsage};
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn rates_for_test() -> PriceTable {
        let mut models = std::collections::BTreeMap::new();
        models.insert(
            "claude-opus-4-7".to_string(),
            ModelRates {
                input_per_mtok: 15.0,
                output_per_mtok: 75.0,
                cache_write_per_mtok: 18.75,
                cache_read_per_mtok: 1.5,
            },
        );
        models.insert(
            "claude-sonnet-4-6".to_string(),
            ModelRates {
                input_per_mtok: 3.0,
                output_per_mtok: 15.0,
                cache_write_per_mtok: 3.75,
                cache_read_per_mtok: 0.3,
            },
        );
        PriceTable {
            models,
            source: PriceSource::Bundled {
                verified_at: "test".into(),
            },
            last_fetch_error: None,
        }
    }

    fn row(project: &str, last_ts_ms: i64, models: Vec<&str>, tokens: TokenUsage) -> SessionRow {
        SessionRow {
            session_id: "sess".into(),
            slug: "slug".into(),
            file_path: PathBuf::from("/tmp/x.jsonl"),
            file_size_bytes: 0,
            last_modified: None,
            project_path: project.into(),
            project_from_transcript: true,
            first_ts: None,
            last_ts: Some(chrono::Utc.timestamp_millis_opt(last_ts_ms).unwrap()),
            event_count: 0,
            message_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            first_user_prompt: None,
            models: models.into_iter().map(String::from).collect(),
            tokens,
            git_branch: None,
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    #[test]
    fn time_window_contains_handles_open_ends_and_inside() {
        let w = TimeWindow {
            from_ms: Some(100),
            to_ms: Some(200),
        };
        assert!(w.contains(Some(150)));
        assert!(w.contains(Some(100))); // inclusive lower
        assert!(w.contains(Some(200))); // inclusive upper
        assert!(!w.contains(Some(99)));
        assert!(!w.contains(Some(201)));
        assert!(!w.contains(None));

        // Open lower
        let w = TimeWindow {
            from_ms: None,
            to_ms: Some(200),
        };
        assert!(w.contains(Some(50)));
        assert!(!w.contains(Some(201)));

        // Fully open
        let w = TimeWindow::open();
        assert!(w.contains(Some(0)));
        assert!(!w.contains(None)); // None still excluded
    }

    #[test]
    fn last_days_zero_means_all_time() {
        let w = TimeWindow::last_days(0, 1_700_000_000_000);
        assert!(w.from_ms.is_none() && w.to_ms.is_none());
    }

    #[test]
    fn last_days_n_anchors_at_now() {
        let now = 1_700_000_000_000;
        let w = TimeWindow::last_days(7, now);
        assert_eq!(w.to_ms, Some(now));
        assert_eq!(w.from_ms, Some(now - 7 * 86_400_000));
    }

    #[test]
    fn aggregate_groups_by_project_and_sums_tokens() {
        let prices = rates_for_test();
        let rows = vec![
            row(
                "/work/foo",
                1_000,
                vec!["claude-sonnet-4-6"],
                TokenUsage {
                    input: 1_000_000,
                    output: 500_000,
                    cache_creation: 100_000,
                    cache_read: 2_000_000,
                },
            ),
            row(
                "/work/foo",
                2_000,
                vec!["claude-sonnet-4-6"],
                TokenUsage {
                    input: 100_000,
                    output: 50_000,
                    cache_creation: 0,
                    cache_read: 200_000,
                },
            ),
            row(
                "/work/bar",
                3_000,
                vec!["claude-opus-4-7"],
                TokenUsage {
                    input: 200_000,
                    output: 100_000,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
        ];
        let r = aggregate_from_rows(rows, &prices, TimeWindow::open());

        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.totals.session_count, 3);
        assert_eq!(r.totals.tokens_input, 1_300_000);
        assert_eq!(r.totals.tokens_output, 650_000);
        assert_eq!(r.totals.tokens_cache_read, 2_200_000);
        assert!(r.totals.cost_usd.is_some());

        // Newest-first: /work/bar (last 3000) before /work/foo (last 2000).
        assert_eq!(r.rows[0].project_path, "/work/bar");
        assert_eq!(r.rows[0].session_count, 1);
        assert_eq!(r.rows[1].project_path, "/work/foo");
        assert_eq!(r.rows[1].session_count, 2);

        // Spot-check Sonnet cost for /work/foo:
        //   total: input  1.1M  × $3/M  = $3.30
        //          output 0.55M × $15/M = $8.25
        //          cwrite 0.1M  × $3.75 = $0.375
        //          cread  2.2M  × $0.30 = $0.66
        //   sum ≈ $12.585
        let foo_cost = r.rows[1].cost_usd.unwrap();
        assert!(
            (foo_cost - 12.585).abs() < 0.01,
            "expected ~12.585 USD, got {foo_cost}"
        );
    }

    #[test]
    fn unpriced_sessions_keep_token_totals_truthful() {
        let prices = rates_for_test();
        let rows = vec![
            row(
                "/x",
                1_000,
                vec!["claude-sonnet-4-6"],
                TokenUsage {
                    input: 1_000_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            // Unknown model — should not poison cost, but tokens still counted.
            row(
                "/x",
                2_000,
                vec!["claude-future-9000"],
                TokenUsage {
                    input: 500_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            // No models at all — same treatment.
            row(
                "/x",
                3_000,
                vec![],
                TokenUsage {
                    input: 250_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
        ];
        let r = aggregate_from_rows(rows, &prices, TimeWindow::open());
        assert_eq!(r.rows.len(), 1);
        let x = &r.rows[0];
        assert_eq!(x.session_count, 3);
        assert_eq!(x.tokens_input, 1_750_000);
        assert_eq!(x.unpriced_sessions, 2);
        // Cost should reflect the one priced session only:
        // 1M × $3/M = $3.00
        let cost = x.cost_usd.unwrap();
        assert!((cost - 3.0).abs() < 1e-9);
    }

    #[test]
    fn fully_unpriced_project_returns_none_cost() {
        let prices = rates_for_test();
        let rows = vec![row(
            "/y",
            1_000,
            vec![],
            TokenUsage {
                input: 1_000,
                output: 0,
                cache_creation: 0,
                cache_read: 0,
            },
        )];
        let r = aggregate_from_rows(rows, &prices, TimeWindow::open());
        assert_eq!(r.rows[0].cost_usd, None);
        assert_eq!(r.rows[0].unpriced_sessions, 1);
        // Totals follow the same rule.
        assert_eq!(r.totals.cost_usd, None);
        assert_eq!(r.totals.unpriced_sessions, 1);
    }

    #[test]
    fn time_window_filters_outside_sessions() {
        let prices = rates_for_test();
        let rows = vec![
            row(
                "/a",
                500,
                vec!["claude-sonnet-4-6"],
                TokenUsage {
                    input: 1,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            row(
                "/a",
                1_500,
                vec!["claude-sonnet-4-6"],
                TokenUsage {
                    input: 1,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
        ];
        let w = TimeWindow {
            from_ms: Some(1_000),
            to_ms: Some(2_000),
        };
        let r = aggregate_from_rows(rows, &prices, w);
        // Only the second row falls inside.
        assert_eq!(r.totals.session_count, 1);
        assert_eq!(r.totals.tokens_input, 1);
    }

    #[test]
    fn dominant_model_picks_alphabetically_first_for_determinism() {
        // Same payload, two model orderings → identical cost.
        let prices = rates_for_test();
        let tokens = TokenUsage {
            input: 1_000_000,
            output: 0,
            cache_creation: 0,
            cache_read: 0,
        };
        let r1 = aggregate_from_rows(
            vec![row(
                "/p",
                1,
                vec!["claude-sonnet-4-6", "claude-opus-4-7"],
                tokens.clone(),
            )],
            &prices,
            TimeWindow::open(),
        );
        let r2 = aggregate_from_rows(
            vec![row(
                "/p",
                1,
                vec!["claude-opus-4-7", "claude-sonnet-4-6"],
                tokens,
            )],
            &prices,
            TimeWindow::open(),
        );
        assert_eq!(r1.rows[0].cost_usd, r2.rows[0].cost_usd);
        // And it picked Opus (alphabetically before Sonnet) → 1M × $15/M = $15.
        let cost = r1.rows[0].cost_usd.unwrap();
        assert!((cost - 15.0).abs() < 1e-9);
    }
}
