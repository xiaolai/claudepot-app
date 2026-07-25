//! The one rate-resolution surface: bundled price history plus this
//! install's observed rate changes, resolved for a given day.
//!
//! # Why this exists
//!
//! Rate resolution used to be implemented three times — once in
//! `session_live::pricing` (guessed a family rate for unknown models),
//! once in `pricing::resolve_model_rates` (refused to guess), and once
//! in `src/costs.ts` (also refused). The same session could be priced
//! by one and reported unpriced by another. [`PriceBook`] is the single
//! algorithm; `src/costs.ts` mirrors it against the shared vectors in
//! `crates/claudepot-core/testdata/rate-resolution-vectors.json`.
//!
//! # What it resolves
//!
//! 1. **Exact** — the canonicalized id is priced; take the period
//!    covering the requested day, with observed changes merged in.
//! 2. **Family estimate** — the id isn't priced but its family is;
//!    borrow the family's current model's rate *for that same day*.
//!    Marked [`RateConfidence::FamilyEstimate`] so no surface passes a
//!    guess off as a quote.
//! 3. **Unpriced** — no family match; `None`, rendered `—`.

use crate::pricing::history::HistoryFile;
use crate::session::TokenUsage;
use crate::session_live::pricing::{
    self as rates, apply_rates, canonicalize_model_id, ModelRates, RateConfidence, RatePeriod,
    ResolvedRates, Ymd,
};

/// Bundled rate history overlaid with observed rate changes.
///
/// Cheap to clone-free borrow but not free to build — it owns the
/// loaded history file — so build one per rollup rather than per row.
#[derive(Debug, Clone, Default)]
pub struct PriceBook {
    history: HistoryFile,
    /// Scales every resolved rate onto the billing platform's rate
    /// card. Applied at resolution rather than baked into the periods,
    /// so switching tiers never rewrites recorded history. See
    /// [`crate::pricing::PriceTier`] for why a scalar is the right
    /// shape.
    tier_multiplier: f64,
}

/// A priced figure plus how much to trust the rate behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PricedCost {
    pub usd: f64,
    pub confidence: RateConfidence,
}

impl PriceBook {
    /// Bundled history only — no observed changes. Used by tests and
    /// by callers that must not touch the filesystem.
    pub fn bundled_only() -> Self {
        Self {
            history: HistoryFile::default(),
            tier_multiplier: 1.0,
        }
    }

    /// Bundled history overlaid with the supplied observations.
    pub fn with_history(history: HistoryFile) -> Self {
        Self {
            history,
            tier_multiplier: 1.0,
        }
    }

    /// Load the observed-change file from the standard location. A
    /// missing or corrupt file degrades to bundled-only rather than
    /// failing: a lost observation costs accuracy, not correctness.
    pub fn load() -> Self {
        match crate::pricing::history::load() {
            Ok(history) => Self::with_history(history),
            Err(e) => {
                tracing::warn!(error = %e, "pricing history unreadable; using bundled rates");
                Self::bundled_only()
            }
        }
    }

    /// Scale every resolved rate onto `tier`'s rate card.
    ///
    /// Applied at resolution rather than folded into the stored
    /// periods, so switching tiers re-prices the display without
    /// touching the recorded history.
    pub fn with_tier(mut self, tier: crate::pricing::PriceTier) -> Self {
        self.tier_multiplier = tier.rate_multiplier();
        self
    }

    /// Fold an in-memory current-rates table into the book as
    /// observations dated `on`, for any model whose rate differs from
    /// what the book already resolves for that day.
    ///
    /// This exists because the two are otherwise only *usually* in
    /// sync. A live scrape records its changes to `pricing-history.json`
    /// and the book reads them back — but that write is best-effort, and
    /// a failed one leaves the app displaying a fresh rate in the chip
    /// while pricing sessions from the stale book. Folding the table in
    /// makes a book built alongside a table agree with it by
    /// construction rather than by the history write having succeeded.
    ///
    /// Only differences are folded, so the common case (nothing moved)
    /// allocates nothing and leaves the recorded history untouched.
    pub fn with_current_rates(
        mut self,
        current: &std::collections::BTreeMap<String, crate::pricing::ModelRates>,
        on: Ymd,
    ) -> Self {
        for (id, table_rates) in current {
            let believed = self.resolve(id, on).map(|r| r.rates);
            let live = crate::pricing::history::to_live_rates(table_rates);
            if believed == Some(live) {
                continue;
            }
            self.history.observe(id, table_rates, None, on);
        }
        self
    }

    /// The effective rate periods for a canonical id, bundled +
    /// observed. `None` when neither source knows the id.
    ///
    /// A model can be observed without being bundled: a scrape sees a
    /// model this build predates, and `record_scrape` logs it because
    /// we had no prior rate. Requiring a bundled entry here made those
    /// observations write-only — recorded on disk and never read back.
    fn periods(&self, canonical_id: &str) -> Option<Vec<RatePeriod>> {
        let bundled = rates::periods_for_id(canonical_id).unwrap_or(&[]);
        let merged = self.history.effective_periods(canonical_id, bundled);
        if merged.is_empty() {
            return None;
        }
        Some(merged)
    }

    /// Resolve the rate for `model` as it stood on `on`.
    pub fn resolve(&self, model: &str, on: Ymd) -> Option<ResolvedRates> {
        let key = canonicalize_model_id(model);
        if let Some(r) = self.periods(&key).and_then(|p| rates::rate_on(&p, on)) {
            return Some(ResolvedRates {
                rates: self.scaled(r),
                confidence: RateConfidence::Exact,
            });
        }
        // Falls through to the family when the id is unknown *or* when
        // it is known but has no period covering `on` — a model whose
        // first recorded rate postdates the usage. Borrowing the
        // family's rate for that day beats reporting the session
        // unpriced, and the `FamilyEstimate` marker keeps it honest.
        let current = rates::family_current_id(&key)?;
        let r = self.periods(current).and_then(|p| rates::rate_on(&p, on))?;
        Some(ResolvedRates {
            rates: self.scaled(r),
            confidence: RateConfidence::FamilyEstimate,
        })
    }

    fn scaled(&self, r: ModelRates) -> ModelRates {
        if self.tier_multiplier == 1.0 {
            return r;
        }
        let m = self.tier_multiplier;
        ModelRates {
            input_per_million_usd: r.input_per_million_usd * m,
            output_per_million_usd: r.output_per_million_usd * m,
            cache_read_per_million_usd: r.cache_read_per_million_usd * m,
            cache_write_per_million_usd: r.cache_write_per_million_usd * m,
        }
    }

    /// Resolve the rate for `model` as of an epoch-millisecond
    /// timestamp. `None` when the timestamp is unrepresentable *or*
    /// the model is unpriced.
    pub fn resolve_at_ms(&self, model: &str, ts_ms: i64) -> Option<ResolvedRates> {
        self.resolve(model, rates::ymd_from_ms(ts_ms)?)
    }

    /// Cost of `usage` for `model` on day `on`.
    pub fn cost(&self, model: &str, on: Ymd, usage: &TokenUsage) -> Option<PricedCost> {
        let r = self.resolve(model, on)?;
        Some(PricedCost {
            usd: apply_usage(&r.rates, usage),
            confidence: r.confidence,
        })
    }

    /// Cost of `usage` for `model` at an epoch-millisecond timestamp.
    ///
    /// `ts_ms: None` — a transcript line with no usable timestamp —
    /// falls back to today's rate. That is the same answer the old
    /// undated code gave, so an untimestamped row is no worse off than
    /// before, and the confidence marker still reflects the model
    /// match.
    pub fn cost_at_ms(
        &self,
        model: &str,
        ts_ms: Option<i64>,
        usage: &TokenUsage,
    ) -> Option<PricedCost> {
        let on = ts_ms
            .and_then(rates::ymd_from_ms)
            .unwrap_or_else(rates::today_utc);
        self.cost(model, on, usage)
    }
}

/// A serializable snapshot of the whole book: every model's effective
/// periods plus the family fallback map.
///
/// This exists so the renderer can run the *same* resolution the
/// backend runs, over the same data, instead of reimplementing a
/// simplified version against a flat current-rates map — which is how
/// `src/costs.ts` came to disagree with both Rust paths.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PriceBookSnapshot {
    /// Model id → effective periods, oldest first.
    pub models: std::collections::BTreeMap<String, Vec<PeriodSnapshot>>,
    /// `claude-<family>-` → the model id an unlisted member falls back to.
    pub family_current: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeriodSnapshot {
    /// `[year, month, day]`, or `null` for the opening period.
    pub starts: Option<[i32; 3]>,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_write_per_mtok: f64,
    pub cache_read_per_mtok: f64,
}

impl PriceBook {
    /// Snapshot every priced model's effective periods, tier scaling
    /// included, for transport to the renderer or a test fixture.
    pub fn snapshot(&self) -> PriceBookSnapshot {
        let mut models = std::collections::BTreeMap::new();
        // Bundled ids *plus* any model this install has only ever
        // observed. Iterating the bundled list alone would hand the
        // renderer a book that can't price a model the backend can.
        let ids: std::collections::BTreeSet<String> = rates::priced_model_ids()
            .map(str::to_string)
            .chain(self.history.observed_model_ids())
            .collect();
        for id in &ids {
            let Some(periods) = self.periods(id) else {
                continue;
            };
            let snapped: Vec<PeriodSnapshot> = periods
                .iter()
                .map(|p| {
                    let r = self.scaled(p.rates);
                    PeriodSnapshot {
                        starts: p.starts.map(|(y, m, d)| [y, m as i32, d as i32]),
                        input_per_mtok: r.input_per_million_usd,
                        output_per_mtok: r.output_per_million_usd,
                        cache_write_per_mtok: r.cache_write_per_million_usd,
                        cache_read_per_mtok: r.cache_read_per_million_usd,
                    }
                })
                .collect();
            models.insert(id.to_string(), snapped);
        }
        PriceBookSnapshot {
            models,
            family_current: rates::family_current_map()
                .map(|(f, c)| (f.to_string(), c.to_string()))
                .collect(),
        }
    }
}

/// Apply rates to a [`TokenUsage`], mapping its field names onto the
/// four priced token classes.
fn apply_usage(r: &ModelRates, usage: &TokenUsage) -> f64 {
    apply_rates(
        r,
        usage.input,
        usage.output,
        usage.cache_read,
        usage.cache_creation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::ModelRates as TableRates;

    const DURING_INTRO: Ymd = (2026, 7, 25);
    const AFTER_INTRO: Ymd = (2026, 9, 1);

    fn usage(input: u64, output: u64) -> TokenUsage {
        TokenUsage {
            input,
            output,
            cache_read: 0,
            cache_creation: 0,
        }
    }

    #[test]
    fn a_listed_model_resolves_exactly() {
        let book = PriceBook::bundled_only();
        let r = book.resolve("claude-opus-5", DURING_INTRO).unwrap();
        assert_eq!(r.confidence, RateConfidence::Exact);
        assert_eq!(r.rates.input_per_million_usd, 5.0);
    }

    #[test]
    fn an_unlisted_model_resolves_as_a_family_estimate() {
        let book = PriceBook::bundled_only();
        let r = book.resolve("claude-opus-7", DURING_INTRO).unwrap();
        assert_eq!(r.confidence, RateConfidence::FamilyEstimate);
        assert_eq!(r.rates.input_per_million_usd, 5.0);
    }

    #[test]
    fn a_model_from_no_priced_family_is_unpriced() {
        let book = PriceBook::bundled_only();
        assert!(book.resolve("gpt-4", DURING_INTRO).is_none());
        assert!(book.cost("gpt-4", DURING_INTRO, &usage(1, 1)).is_none());
    }

    #[test]
    fn the_same_session_prices_differently_across_a_rate_change() {
        // The whole point of the dated book: one million Sonnet 5
        // input tokens cost $2 during the introductory window and $3
        // after it, and neither figure rewrites the other.
        let book = PriceBook::bundled_only();
        let before = book
            .cost("claude-sonnet-5", DURING_INTRO, &usage(1_000_000, 0))
            .unwrap();
        let after = book
            .cost("claude-sonnet-5", AFTER_INTRO, &usage(1_000_000, 0))
            .unwrap();
        assert!((before.usd - 2.0).abs() < 1e-9);
        assert!((after.usd - 3.0).abs() < 1e-9);
    }

    #[test]
    fn an_observed_change_applies_only_from_the_day_it_was_seen() {
        let mut history = HistoryFile::default();
        history.observe(
            "claude-opus-5",
            &TableRates {
                input_per_mtok: 8.0,
                output_per_mtok: 40.0,
                cache_write_per_mtok: 10.0,
                cache_read_per_mtok: 0.8,
            },
            None,
            (2026, 10, 1),
        );
        let book = PriceBook::with_history(history);

        // Before the observation: the bundled rate still stands.
        let before = book.resolve("claude-opus-5", (2026, 9, 30)).unwrap();
        assert_eq!(before.rates.input_per_million_usd, 5.0);
        // On and after: the observed rate.
        let after = book.resolve("claude-opus-5", (2026, 10, 1)).unwrap();
        assert_eq!(after.rates.input_per_million_usd, 8.0);
    }

    #[test]
    fn a_family_estimate_follows_observed_changes_too() {
        // An unlisted Opus borrows the current Opus rate, so it must
        // also pick up an observed change to that model.
        let mut history = HistoryFile::default();
        history.observe(
            "claude-opus-5",
            &TableRates {
                input_per_mtok: 8.0,
                output_per_mtok: 40.0,
                cache_write_per_mtok: 10.0,
                cache_read_per_mtok: 0.8,
            },
            None,
            (2026, 10, 1),
        );
        let book = PriceBook::with_history(history);
        let r = book.resolve("claude-opus-9", (2026, 10, 2)).unwrap();
        assert_eq!(r.confidence, RateConfidence::FamilyEstimate);
        assert_eq!(r.rates.input_per_million_usd, 8.0);
    }

    #[test]
    fn cost_weights_every_token_class() {
        let book = PriceBook::bundled_only();
        let u = TokenUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_creation: 1_000_000,
        };
        // Opus 5: $5 in + $25 out + $0.50 cache-read + $6.25 cache-write.
        let c = book.cost("claude-opus-5", DURING_INTRO, &u).unwrap();
        assert!((c.usd - 36.75).abs() < 1e-9);
    }

    #[test]
    fn an_untimestamped_row_falls_back_to_todays_rate() {
        let book = PriceBook::bundled_only();
        let fallback = book
            .cost_at_ms("claude-opus-5", None, &usage(1_000_000, 0))
            .unwrap();
        let today = book
            .cost("claude-opus-5", rates::today_utc(), &usage(1_000_000, 0))
            .unwrap();
        assert_eq!(fallback, today);
    }

    // ── Cross-language vectors ─────────────────────────────────────

    /// Vectors both this module and `src/costs.ts` are checked against.
    ///
    /// Rate resolution has to exist in two languages: the rollups run
    /// in Rust, and the dashboard aggregates client-side over rows it
    /// already holds. Two implementations drift — that is how
    /// `costs.ts` ended up disagreeing with both Rust paths. The
    /// fixture is the contract; `src/costs.test.ts` reads the same
    /// file.
    #[derive(serde::Deserialize)]
    struct VectorFile {
        vectors: Vec<Vector>,
    }

    #[derive(serde::Deserialize)]
    struct Vector {
        name: String,
        model: String,
        on: [i32; 3],
        /// `"exact"`, `"family_estimate"`, or `"unpriced"`.
        expect: String,
        #[serde(default)]
        input_per_mtok: Option<f64>,
    }

    fn vectors_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("rate-resolution-vectors.json")
    }

    #[test]
    fn shared_vectors_match_this_implementation() {
        let raw = std::fs::read_to_string(vectors_path())
            .expect("rate-resolution-vectors.json must exist");
        let file: VectorFile = serde_json::from_str(&raw).expect("vectors must parse");
        assert!(!file.vectors.is_empty(), "fixture must not be empty");
        let book = PriceBook::bundled_only();
        for v in &file.vectors {
            let on = (v.on[0], v.on[1] as u32, v.on[2] as u32);
            let got = book.resolve(&v.model, on);
            match v.expect.as_str() {
                "unpriced" => assert!(got.is_none(), "{}: expected unpriced", v.name),
                "exact" | "family_estimate" => {
                    let got = got.unwrap_or_else(|| panic!("{}: expected a rate", v.name));
                    let want = if v.expect == "exact" {
                        RateConfidence::Exact
                    } else {
                        RateConfidence::FamilyEstimate
                    };
                    assert_eq!(got.confidence, want, "{}: confidence", v.name);
                    if let Some(expected) = v.input_per_mtok {
                        assert!(
                            (got.rates.input_per_million_usd - expected).abs() < 1e-9,
                            "{}: expected ${expected}/MTok in, got ${}",
                            v.name,
                            got.rates.input_per_million_usd
                        );
                    }
                }
                other => panic!("{}: unknown expect `{other}`", v.name),
            }
        }
    }

    #[test]
    fn the_snapshot_carries_every_priced_model_and_family() {
        // The renderer resolves against this snapshot, so a model
        // missing from it is a model the dashboard can't price.
        let snap = PriceBook::bundled_only().snapshot();
        for id in rates::priced_model_ids() {
            assert!(snap.models.contains_key(id), "{id} missing from snapshot");
        }
        for (family, current) in rates::family_current_map() {
            assert_eq!(
                snap.family_current.get(family).map(String::as_str),
                Some(current)
            );
        }
    }

    #[test]
    fn current_rates_are_folded_in_when_they_differ_from_the_book() {
        // The scrape recorded a change but the history write failed, so
        // the book alone still says $5. Folding the live table in makes
        // the DTO self-consistent anyway.
        let mut current = std::collections::BTreeMap::new();
        current.insert(
            "claude-opus-5".to_string(),
            TableRates {
                input_per_mtok: 7.0,
                output_per_mtok: 35.0,
                cache_write_per_mtok: 8.75,
                cache_read_per_mtok: 0.7,
            },
        );
        let book = PriceBook::bundled_only().with_current_rates(&current, DURING_INTRO);
        let r = book.resolve("claude-opus-5", DURING_INTRO).unwrap();
        assert_eq!(r.rates.input_per_million_usd, 7.0);
    }

    #[test]
    fn folding_in_matching_current_rates_changes_nothing() {
        let mut current = std::collections::BTreeMap::new();
        current.insert(
            "claude-opus-5".to_string(),
            TableRates {
                input_per_mtok: 5.0,
                output_per_mtok: 25.0,
                cache_write_per_mtok: 6.25,
                cache_read_per_mtok: 0.5,
            },
        );
        let folded = PriceBook::bundled_only().with_current_rates(&current, DURING_INTRO);
        assert_eq!(
            folded.snapshot(),
            PriceBook::bundled_only().snapshot(),
            "an unchanged rate must not add a period"
        );
    }

    #[test]
    fn folding_in_current_rates_leaves_earlier_days_alone() {
        // Only "from today" is claimed — a rate seen today says nothing
        // about what last month cost.
        let mut current = std::collections::BTreeMap::new();
        current.insert(
            "claude-opus-5".to_string(),
            TableRates {
                input_per_mtok: 7.0,
                output_per_mtok: 35.0,
                cache_write_per_mtok: 8.75,
                cache_read_per_mtok: 0.7,
            },
        );
        let book = PriceBook::bundled_only().with_current_rates(&current, (2026, 7, 25));
        assert_eq!(
            book.resolve("claude-opus-5", (2026, 7, 24))
                .unwrap()
                .rates
                .input_per_million_usd,
            5.0
        );
    }

    #[test]
    fn a_scraped_model_the_bundle_lacks_becomes_resolvable() {
        let mut current = std::collections::BTreeMap::new();
        current.insert(
            "claude-opus-9".to_string(),
            TableRates {
                input_per_mtok: 12.0,
                output_per_mtok: 60.0,
                cache_write_per_mtok: 15.0,
                cache_read_per_mtok: 1.2,
            },
        );
        let book = PriceBook::bundled_only().with_current_rates(&current, DURING_INTRO);
        let r = book.resolve("claude-opus-9", DURING_INTRO).unwrap();
        assert_eq!(r.confidence, RateConfidence::Exact);
        assert_eq!(r.rates.input_per_million_usd, 12.0);
        assert!(book.snapshot().models.contains_key("claude-opus-9"));
    }

    #[test]
    fn the_snapshot_applies_the_tier_multiplier() {
        let snap = PriceBook::bundled_only()
            .with_tier(crate::pricing::PriceTier::AnthropicApi)
            .snapshot();
        let opus = &snap.models["claude-opus-5"][0];
        assert_eq!(opus.input_per_mtok, 5.0);
    }

    #[test]
    fn resolve_at_ms_uses_the_timestamps_own_day() {
        let book = PriceBook::bundled_only();
        let intro_ms = "2026-08-15T12:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            .timestamp_millis();
        let after_ms = "2026-09-15T12:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            book.resolve_at_ms("claude-sonnet-5", intro_ms)
                .unwrap()
                .rates
                .input_per_million_usd,
            2.0
        );
        assert_eq!(
            book.resolve_at_ms("claude-sonnet-5", after_ms)
                .unwrap()
                .rates
                .input_per_million_usd,
            3.0
        );
    }
}
