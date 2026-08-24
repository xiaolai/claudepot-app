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

use crate::pricing::{PriceBook, PricedCost};
use crate::session::SessionRow;
use crate::session_index::error::SessionIndexError;
use crate::session_index::{SessionIndex, TurnCandidate};
use crate::session_live::pricing::{self, RateConfidence};
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
    /// Sessions priced from their *family's* rate because the exact
    /// model isn't in the rate table. Their dollars are included in
    /// `cost_usd`; this count is what lets the UI mark the figure as
    /// partly estimated instead of quoting it flat.
    pub estimated_sessions: usize,
    /// Session-count breakdown by model. Each session contributes
    /// once per *distinct* model it used, so a session that mixed
    /// Opus and Sonnet adds 1 to both buckets — the sum of values is
    /// ≥ `session_count`. Sessions with zero recorded models (no
    /// assistant turn yet) contribute nothing here. Used by the GUI
    /// to render the "model mix" badge column.
    pub models_by_session: BTreeMap<String, usize>,
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
    /// Install-wide count of sessions priced from a family estimate.
    pub estimated_sessions: usize,
    /// Install-wide session-count breakdown by model — mirrors
    /// `ProjectUsageRow.models_by_session` aggregated across every
    /// row in the report.
    pub models_by_session: BTreeMap<String, usize>,
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
    prices: &PriceBook,
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
    prices: &PriceBook,
    window: TimeWindow,
) -> LocalUsageReport {
    let mut by_project: BTreeMap<String, ProjectAccumulator> = BTreeMap::new();
    let mut totals = UsageTotals::default();

    for s in sessions {
        let last_ms = s.last_ts.map(|t| t.timestamp_millis());
        if !window.contains(last_ms) {
            continue;
        }

        let session_cost = compute_session_cost(&s, prices, last_ms);

        let acc = by_project.entry(s.project_path.clone()).or_default();
        acc.session_count += 1;
        acc.tokens_input += s.tokens.input;
        acc.tokens_output += s.tokens.output;
        acc.tokens_cache_creation += s.tokens.cache_creation;
        acc.tokens_cache_read += s.tokens.cache_read;
        merge_min(&mut acc.first_active_ms, last_ms);
        merge_max(&mut acc.last_active_ms, last_ms);
        match session_cost {
            Some(c) => {
                *acc.cost_usd.get_or_insert(0.0) += c.usd;
                if c.confidence == RateConfidence::FamilyEstimate {
                    acc.estimated_sessions += 1;
                }
            }
            None => acc.unpriced_sessions += 1,
        }
        // Distinct-models-per-session: dedupe at the session boundary
        // so a single session that emitted three Opus turns and one
        // Sonnet turn adds 1 to each bucket, not 4. Sessions with no
        // recorded models simply don't contribute.
        //
        // `<synthetic>` is excluded for the same reason it is excluded
        // from pricing: it is CC's placeholder for a locally-generated
        // turn, and listing it in a "models used" breakdown invents a
        // model the user never chose.
        let mut seen = std::collections::HashSet::new();
        for m in s.models.iter().filter(|m| !pricing::is_synthetic_model(m)) {
            if seen.insert(m.as_str()) {
                *acc.models_by_session.entry(m.clone()).or_insert(0) += 1;
                *totals.models_by_session.entry(m.clone()).or_insert(0) += 1;
            }
        }

        totals.session_count += 1;
        totals.tokens_input += s.tokens.input;
        totals.tokens_output += s.tokens.output;
        totals.tokens_cache_creation += s.tokens.cache_creation;
        totals.tokens_cache_read += s.tokens.cache_read;
        merge_min(&mut totals.first_active_ms, last_ms);
        merge_max(&mut totals.last_active_ms, last_ms);
        match session_cost {
            Some(c) => {
                *totals.cost_usd.get_or_insert(0.0) += c.usd;
                if c.confidence == RateConfidence::FamilyEstimate {
                    totals.estimated_sessions += 1;
                }
            }
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
            estimated_sessions: acc.estimated_sessions,
            models_by_session: acc.models_by_session,
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
    estimated_sessions: usize,
    models_by_session: BTreeMap<String, usize>,
}

/// One assistant turn ranked among the install's costliest prompts,
/// with its computed dollar cost and surrounding context. Returned
/// by [`top_costly_turns`] for the GUI's "Top costly prompts" panel
/// and the future CLI surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostlyTurn {
    /// Absolute path to the `.jsonl` transcript that produced this
    /// turn. Stable identifier — joins back to the `sessions` row
    /// for further drill-down.
    pub file_path: String,
    /// Project the session belonged to. Echoed here so the consumer
    /// surface can render the column without a round-trip.
    pub project_path: String,
    /// 0-based assistant-turn ordinal in the transcript.
    pub turn_index: usize,
    /// Server-side timestamp (`None` when the source line lacked a
    /// usable `timestamp`).
    pub ts_ms: Option<i64>,
    /// Model id stamped on the turn (`""` when missing).
    pub model: String,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub tokens_cache_creation: u64,
    pub tokens_cache_read: u64,
    /// Truncated copy of the user prompt that drove this turn.
    /// Already `sk-ant-`-redacted at write time.
    pub user_prompt_preview: Option<String>,
    /// Dollar cost of this turn at the rate in force on its own
    /// timestamp. `None` when the turn's model belongs to no family
    /// the book prices — the row is then dropped from the consumer's
    /// ranking but would survive a verbose mode.
    pub cost_usd: Option<f64>,
    /// True when `cost_usd` came from the model's *family* rate rather
    /// than a rate recorded for that exact model. The UI marks these
    /// so an estimate is never read as a quote.
    #[serde(default)]
    pub cost_is_estimated: bool,
}

/// Return the install's `final_n` costliest turns within `window`,
/// scored against `prices`. Two-stage ranking:
///
/// 1. SQL pulls a coarse `final_n × 50` candidates ordered by
///    total token count (cost-proxy that's fast to evaluate in
///    SQLite without a per-row model JOIN).
/// 2. Rust resolves each candidate's model against `prices`,
///    computes the precise dollar cost, drops unresolved-model
///    rows, then sorts by cost descending.
///
/// The proxy can re-order across model families (Opus tokens are
/// ~20× more expensive than Haiku tokens), so the SQL pre-sort is
/// only a reduction. The pool size of `50×` is generous for
/// realistic cross-model variance — adjust if benchmarks show a
/// tighter bound holds.
///
/// Returns an empty Vec when no turns match (e.g. fresh install
/// with sessions on disk but no re-scan yet to populate the turn
/// table). Errors only on underlying SQL paths.
pub fn top_costly_turns(
    index: &SessionIndex,
    prices: &PriceBook,
    window: TimeWindow,
    final_n: usize,
) -> Result<Vec<CostlyTurn>, SessionIndexError> {
    if final_n == 0 {
        return Ok(Vec::new());
    }
    let pool_limit = final_n.saturating_mul(50).max(50);
    let candidates = index.turn_candidates(window.from_ms, window.to_ms, pool_limit)?;
    Ok(rank_candidates(candidates, prices, final_n))
}

/// Pure variant of [`top_costly_turns`] that takes pre-fetched
/// candidates. Exposed for tests + future callers that already
/// hold a candidate slice (e.g. a per-project drill-down that
/// pre-filters by `file_path`).
pub fn rank_candidates(
    candidates: Vec<TurnCandidate>,
    prices: &PriceBook,
    final_n: usize,
) -> Vec<CostlyTurn> {
    if final_n == 0 {
        return Vec::new();
    }
    let mut scored: Vec<CostlyTurn> = candidates
        .into_iter()
        .filter_map(|c| {
            // Each turn is priced at the rate in force on its own day,
            // so a ranking that spans a price change compares what the
            // turns actually cost rather than re-scoring them all at
            // today's rate.
            let cost = prices.cost_at_ms(&c.model, c.ts_ms, &c.tokens)?;
            Some(CostlyTurn {
                file_path: c.file_path,
                project_path: c.project_path,
                turn_index: c.turn_index,
                ts_ms: c.ts_ms,
                model: c.model,
                tokens_input: c.tokens.input,
                tokens_output: c.tokens.output,
                tokens_cache_creation: c.tokens.cache_creation,
                tokens_cache_read: c.tokens.cache_read,
                user_prompt_preview: c.user_prompt_preview,
                cost_usd: Some(cost.usd),
                cost_is_estimated: cost.confidence == RateConfidence::FamilyEstimate,
            })
        })
        .collect();
    // Descending by cost; ties broken by file_path then turn_index so
    // output is deterministic even when two different transcripts
    // produced equally-priced turns at the same ordinal. Without the
    // file_path tiebreak, the order between equal-cost cross-file
    // turns depended on input ordering, which jittered the GUI's
    // stable-row-key strategy across re-fetches.
    scored.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.turn_index.cmp(&b.turn_index))
    });
    scored.truncate(final_n);
    scored
}

/// Compute USD cost for one session's aggregate token totals using its
/// dominant model, at the rate in force when the session was last
/// active. Returns `None` when:
///   - the session has no models recorded (zero assistant turns), or
///   - the dominant model belongs to no family the book prices.
///
/// A session that spans a rate change is scored entirely at its
/// last-active rate. Splitting it would need per-turn attribution the
/// session row doesn't carry; `top_costly_turns` does score per turn.
fn compute_session_cost(
    s: &SessionRow,
    prices: &PriceBook,
    last_ms: Option<i64>,
) -> Option<PricedCost> {
    let model = dominant_model(&s.models)?;
    prices.cost_at_ms(model, last_ms, &s.tokens)
}

/// Pick the dominant model from a session's recorded model list. CC
/// sessions rarely mix models, so the typical input is a single-element
/// list. When several are present, we pick the alphabetically-first
/// for determinism — there's no per-message token attribution at the
/// session-row level to pick a "majority" by, so any deterministic
/// tiebreak is as good as another. Empty list → None.
///
/// `<synthetic>` is dropped first, and the ordering above is exactly
/// why that matters: it is not a model (see
/// [`pricing::is_synthetic_model`]), and `<` sorts before every
/// lowercase letter, so it won the alphabetical tiebreak in every
/// session that contained one and took the whole session's cost out
/// of the report with it.
fn dominant_model(models: &[String]) -> Option<&str> {
    models
        .iter()
        .map(|s| s.as_str())
        .filter(|m| !pricing::is_synthetic_model(m))
        .min()
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
    use crate::session::{SessionRow, TokenUsage};
    use chrono::TimeZone;
    use std::path::PathBuf;

    /// The real bundled rate history, with no observed changes.
    ///
    /// These tests used to build a synthetic `PriceTable`, which meant
    /// they passed while the shipped table said something else — the
    /// Opus row in that fixture was still the retired $15/$75 long
    /// after the bundled rates moved to $5/$25. Driving them off the
    /// real book keeps the fixtures honest.
    fn rates_for_test() -> PriceBook {
        PriceBook::bundled_only()
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

    /// #91's real cause. `<synthetic>` is not a model, and because
    /// `SessionRow::models` arrives sorted it led the list on every
    /// session that ever hit an API error — winning the alphabetical
    /// tiebreak and taking that session's whole cost out of the report.
    /// On the reference machine that was 186 of 1,817 sessions and the
    /// majority of the spend, which reads from outside as "the bundled
    /// price table doesn't know my models".
    #[test]
    fn a_synthetic_turn_does_not_make_a_whole_session_unpriced() {
        let prices = rates_for_test();
        // Sorted exactly the way a BTreeSet serializes it: `<` (0x3C)
        // before `c` (0x63).
        let row = row(
            "/p",
            1_800_000_000_000,
            vec!["<synthetic>", "claude-opus-5"],
            TokenUsage {
                input: 1_000_000,
                output: 1_000_000,
                cache_creation: 0,
                cache_read: 0,
            },
        );
        assert!(
            row.models[0] < row.models[1],
            "the fixture must reproduce the ordering that causes the bug"
        );

        let out = aggregate_from_rows(vec![row], &prices, TimeWindow::default());
        assert_eq!(
            out.totals.unpriced_sessions, 0,
            "the session has a real model and must be priced"
        );
        let usd = out.totals.cost_usd.expect("priced");
        assert!(usd > 0.0, "cost must be non-zero, got {usd}");

        // And the placeholder must not appear as a model the user used.
        assert!(
            !out.totals.models_by_session.contains_key("<synthetic>"),
            "models breakdown: {:?}",
            out.totals.models_by_session
        );
        assert_eq!(out.totals.models_by_session.get("claude-opus-5"), Some(&1));
    }

    #[test]
    fn a_session_of_only_synthetic_turns_stays_unpriced() {
        // Nothing was billed — CC generated those turns locally and
        // every one carries zero tokens — so "unpriced" is the honest
        // answer here, not a bug to paper over.
        let prices = rates_for_test();
        let row = row(
            "/p",
            1_800_000_000_000,
            vec!["<synthetic>"],
            TokenUsage::default(),
        );
        let out = aggregate_from_rows(vec![row], &prices, TimeWindow::default());
        assert_eq!(out.totals.unpriced_sessions, 1);
        assert!(out.totals.models_by_session.is_empty());
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

    /// Epoch-ms for a UTC date, so date-sensitive tests read as dates.
    fn ms_on(date: &str) -> i64 {
        format!("{date}T12:00:00Z")
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn a_session_is_priced_at_the_rate_in_force_when_it_ran() {
        // Same tokens, same model, two sides of Sonnet 5's
        // introductory-window boundary. Before the dated book both
        // scored identically, which silently rewrote the older figure
        // every time a price moved.
        let prices = rates_for_test();
        let tokens = TokenUsage {
            input: 1_000_000,
            output: 0,
            cache_creation: 0,
            cache_read: 0,
        };
        let during = aggregate_from_rows(
            vec![row(
                "/p",
                ms_on("2026-08-15"),
                vec!["claude-sonnet-5"],
                tokens.clone(),
            )],
            &prices,
            TimeWindow::open(),
        );
        let after = aggregate_from_rows(
            vec![row(
                "/p",
                ms_on("2026-09-15"),
                vec!["claude-sonnet-5"],
                tokens,
            )],
            &prices,
            TimeWindow::open(),
        );
        assert!((during.rows[0].cost_usd.unwrap() - 2.0).abs() < 1e-9);
        assert!((after.rows[0].cost_usd.unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_family_estimated_session_is_counted_but_still_priced() {
        // An unreleased Opus point release resolves through the family
        // fallback: its dollars count toward the total, and the row
        // reports how many sessions were estimated so the UI can mark
        // the figure.
        let prices = rates_for_test();
        let r = aggregate_from_rows(
            vec![row(
                "/p",
                1_000,
                vec!["claude-opus-9"],
                TokenUsage {
                    input: 1_000_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            )],
            &prices,
            TimeWindow::open(),
        );
        assert_eq!(r.rows[0].unpriced_sessions, 0);
        assert_eq!(r.rows[0].estimated_sessions, 1);
        assert!((r.rows[0].cost_usd.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(r.totals.estimated_sessions, 1);
    }

    #[test]
    fn an_exactly_priced_session_is_not_counted_as_estimated() {
        let prices = rates_for_test();
        let r = aggregate_from_rows(
            vec![row(
                "/p",
                1_000,
                vec!["claude-opus-5"],
                TokenUsage {
                    input: 1_000_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            )],
            &prices,
            TimeWindow::open(),
        );
        assert_eq!(r.rows[0].estimated_sessions, 0);
        assert_eq!(r.totals.estimated_sessions, 0);
    }

    #[test]
    fn a_turn_carries_its_own_estimated_flag() {
        let prices = rates_for_test();
        let top = rank_candidates(
            vec![
                make_candidate(
                    "/p",
                    0,
                    "claude-opus-9",
                    TokenUsage {
                        input: 1_000_000,
                        output: 0,
                        cache_creation: 0,
                        cache_read: 0,
                    },
                ),
                make_candidate(
                    "/p",
                    1,
                    "claude-opus-5",
                    TokenUsage {
                        input: 900_000,
                        output: 0,
                        cache_creation: 0,
                        cache_read: 0,
                    },
                ),
            ],
            &prices,
            2,
        );
        assert!(
            top[0].cost_is_estimated,
            "claude-opus-9 is a family estimate"
        );
        assert!(!top[1].cost_is_estimated, "claude-opus-5 is priced exactly");
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
    fn models_by_session_counts_distinct_models_per_session() {
        let prices = rates_for_test();
        let rows = vec![
            // Two sessions on /a, one Opus, one mixed Opus+Sonnet.
            row(
                "/a",
                1_000,
                vec!["claude-opus-4-7"],
                TokenUsage {
                    input: 1,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            row(
                "/a",
                2_000,
                vec!["claude-opus-4-7", "claude-sonnet-4-6"],
                TokenUsage {
                    input: 1,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            // One session on /b, Sonnet only.
            row(
                "/b",
                3_000,
                vec!["claude-sonnet-4-6"],
                TokenUsage {
                    input: 1,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            // No-models session — should not contribute to mix.
            row(
                "/b",
                4_000,
                vec![],
                TokenUsage {
                    input: 1,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
        ];
        let r = aggregate_from_rows(rows, &prices, TimeWindow::open());
        // Rows are sorted newest-first by last_active_ms, so /b appears
        // before /a (last 4_000 vs last 2_000).
        let b = r.rows.iter().find(|r| r.project_path == "/b").unwrap();
        let a = r.rows.iter().find(|r| r.project_path == "/a").unwrap();

        // /a: opus appears in 2 sessions, sonnet in 1.
        assert_eq!(a.models_by_session.get("claude-opus-4-7"), Some(&2));
        assert_eq!(a.models_by_session.get("claude-sonnet-4-6"), Some(&1));
        // /b: sonnet in 1 session; the empty-models session contributes nothing.
        assert_eq!(b.models_by_session.get("claude-sonnet-4-6"), Some(&1));
        assert_eq!(b.models_by_session.len(), 1);

        // Totals: opus 2, sonnet 2 across the two projects.
        assert_eq!(r.totals.models_by_session.get("claude-opus-4-7"), Some(&2));
        assert_eq!(
            r.totals.models_by_session.get("claude-sonnet-4-6"),
            Some(&2)
        );
    }

    #[test]
    fn models_by_session_dedupes_within_a_session() {
        // Defensive: SessionRow.models is already a deduped Vec on the
        // scan path, but a future change might pass duplicates. The
        // aggregator must dedupe at the session boundary so the mix
        // count is "sessions that used model X", not "rows mentioning X".
        let prices = rates_for_test();
        let mut row = row(
            "/p",
            1,
            vec![],
            TokenUsage {
                input: 1,
                output: 0,
                cache_creation: 0,
                cache_read: 0,
            },
        );
        row.models = vec![
            "claude-opus-4-7".into(),
            "claude-opus-4-7".into(),
            "claude-opus-4-7".into(),
        ];
        let r = aggregate_from_rows(vec![row], &prices, TimeWindow::open());
        let p = &r.rows[0];
        // Three duplicates collapse to one; same on totals.
        assert_eq!(p.models_by_session.get("claude-opus-4-7"), Some(&1));
        assert_eq!(r.totals.models_by_session.get("claude-opus-4-7"), Some(&1));
    }

    fn make_candidate(
        project: &str,
        turn_index: usize,
        model: &str,
        tokens: TokenUsage,
    ) -> TurnCandidate {
        TurnCandidate {
            file_path: format!("/p/{turn_index}.jsonl"),
            project_path: project.into(),
            turn_index,
            ts_ms: Some(1_700_000_000_000 + (turn_index as i64) * 1_000),
            model: model.into(),
            tokens,
            user_prompt_preview: Some(format!("prompt {turn_index}")),
        }
    }

    #[test]
    fn rank_candidates_returns_top_n_by_cost() {
        let prices = rates_for_test();
        // Costs (Opus $5/$25, Sonnet $3/$15):
        //   c0 Opus   3M in/0 out → $15
        //   c1 Sonnet 5M in/0 out → $15   (deliberate tie with c0)
        //   c2 Opus 0.5M in/0 out → $2.50
        let candidates = vec![
            make_candidate(
                "/p",
                0,
                "claude-opus-4-7",
                TokenUsage {
                    input: 3_000_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            make_candidate(
                "/p",
                1,
                "claude-sonnet-4-6",
                TokenUsage {
                    input: 5_000_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            make_candidate(
                "/p",
                2,
                "claude-opus-4-7",
                TokenUsage {
                    input: 500_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
        ];
        let top = rank_candidates(candidates, &prices, 2);
        assert_eq!(top.len(), 2);
        // Tie at $15; tiebreak ascending turn_index → c1 (0) first.
        assert_eq!(top[0].turn_index, 0);
        assert!((top[0].cost_usd.unwrap() - 15.0).abs() < 1e-6);
        assert_eq!(top[1].turn_index, 1);
        assert!((top[1].cost_usd.unwrap() - 15.0).abs() < 1e-6);
    }

    #[test]
    fn rank_candidates_drops_unresolved_models() {
        let prices = rates_for_test();
        let candidates = vec![
            make_candidate(
                "/p",
                0,
                "claude-future-9000",
                TokenUsage {
                    input: 100_000_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
            make_candidate(
                "/p",
                1,
                "claude-opus-4-7",
                TokenUsage {
                    input: 1_000,
                    output: 0,
                    cache_creation: 0,
                    cache_read: 0,
                },
            ),
        ];
        let top = rank_candidates(candidates, &prices, 5);
        assert_eq!(top.len(), 1, "unresolved-model turn must be dropped");
        assert_eq!(top[0].turn_index, 1);
    }

    #[test]
    fn rank_candidates_zero_n_returns_empty() {
        let prices = rates_for_test();
        let candidates = vec![make_candidate(
            "/p",
            0,
            "claude-opus-4-7",
            TokenUsage {
                input: 1_000_000,
                output: 0,
                cache_creation: 0,
                cache_read: 0,
            },
        )];
        assert!(rank_candidates(candidates, &prices, 0).is_empty());
    }

    #[test]
    fn rank_candidates_preserves_prompt_preview() {
        let prices = rates_for_test();
        let candidates = vec![make_candidate(
            "/p",
            0,
            "claude-opus-4-7",
            TokenUsage {
                input: 1_000_000,
                output: 0,
                cache_creation: 0,
                cache_read: 0,
            },
        )];
        let top = rank_candidates(candidates, &prices, 1);
        assert_eq!(top[0].user_prompt_preview.as_deref(), Some("prompt 0"));
    }

    #[test]
    fn rank_candidates_breaks_cost_ties_on_turn_index() {
        // Same model, same tokens → identical cost; the consumer's
        // stable-row-key strategy depends on a deterministic fallback.
        let prices = rates_for_test();
        let tokens = TokenUsage {
            input: 1_000_000,
            output: 0,
            cache_creation: 0,
            cache_read: 0,
        };
        let candidates = vec![
            make_candidate("/p", 7, "claude-opus-4-7", tokens.clone()),
            make_candidate("/p", 3, "claude-opus-4-7", tokens.clone()),
            make_candidate("/p", 5, "claude-opus-4-7", tokens),
        ];
        let top = rank_candidates(candidates, &prices, 3);
        assert_eq!(top[0].turn_index, 3);
        assert_eq!(top[1].turn_index, 5);
        assert_eq!(top[2].turn_index, 7);
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
        // And it picked Opus (alphabetically before Sonnet) → 1M × $5/M = $5.
        let cost = r1.rows[0].cost_usd.unwrap();
        assert!((cost - 5.0).abs() < 1e-9, "expected $5, got {cost}");
    }
}
