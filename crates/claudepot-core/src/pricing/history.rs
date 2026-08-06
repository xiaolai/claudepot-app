//! Recorded price history — `~/.claudepot/pricing-history.json`.
//!
//! The bundled table in [`crate::session_live::pricing`] carries the
//! rate history Anthropic has *published*. This file carries the rate
//! changes this install has *observed*: every time a live scrape
//! reports a rate that differs from what we already believe, the new
//! rate is appended with the day we saw it.
//!
//! # Why append instead of overwrite
//!
//! The pricing cache overwrote rates in place, so the moment a price
//! changed, every historical cost figure silently re-scored at the new
//! rate — a January session would be re-priced at July's rate with no
//! trace that anything had moved. Appending keeps each rate bound to
//! the window it applied to, so past figures stay put.
//!
//! # What an observation actually claims
//!
//! An entry dated `D` means **"on day `D` we first saw this rate"**,
//! not "the price changed on day `D`". We only learn about a change
//! when a scrape happens to run, which may be days later, so the date
//! is an upper bound on when the change took effect.
//!
//! Given that imprecision, an observation only ever claims the window
//! **from its own day forward**: everything before it keeps the
//! bundled rate, and a bundled period scheduled for a later date still
//! takes over on schedule. See [`HistoryFile::effective_periods`].
//!
//! Thin wrapper over [`crate::json_store`] for the corruption policy
//! (timestamped rename-aside, atomic write). A corrupt history file
//! recovers to empty and the bundled periods carry on alone, so this
//! store never fails loud: losing observations degrades accuracy, it
//! does not strand an obligation the way a lost permission grant does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::json_store::{self, SaveError};
use crate::pricing::ModelRates;
use crate::session_live::pricing::{RatePeriod, Ymd};

/// Standard filename inside `claudepot_data_dir()`.
pub const HISTORY_FILENAME: &str = "pricing-history.json";

/// Store name used in log messages.
const STORE: &str = "pricing_history";

/// Current on-disk schema.
pub const SCHEMA_VERSION: u32 = 1;

/// Cap on retained observations. Rate changes are rare — a handful of
/// models × a few changes a year — so this is a runaway guard, not a
/// working limit. Oldest entries are dropped first, which is the safe
/// direction: the oldest observation is the one most likely already
/// covered by a bundled period.
pub const MAX_OBSERVATIONS: usize = 1000;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// `~/.claudepot/pricing-history.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn history_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(HISTORY_FILENAME)
}

/// One recorded sighting of a model's rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateObservation {
    /// Canonical model id, e.g. `claude-opus-5`.
    pub model_id: String,
    /// The day this rate was first observed, `[year, month, day]`.
    /// Stored as an array so the file stays hand-readable and sorts
    /// the same way the in-memory [`Ymd`] tuple does.
    pub observed_on: [i32; 3],
    pub rates: ModelRates,
}

impl RateObservation {
    /// The observation day as a comparable [`Ymd`]. Returns `None` for
    /// a date that isn't a real calendar day, which validation rejects.
    ///
    /// Range-checking month 1–12 and day 1–31 is not enough: it accepts
    /// `2026-02-31`, which would then sort between real February and
    /// March days and silently shift a rate boundary. `NaiveDate` does
    /// the real check, leap years included.
    pub fn day(&self) -> Option<Ymd> {
        let [y, m, d] = self.observed_on;
        let (m, d) = (u32::try_from(m).ok()?, u32::try_from(d).ok()?);
        chrono::NaiveDate::from_ymd_opt(y, m, d)?;
        Some((y, m, d))
    }
}

/// Top-level on-disk shape of `~/.claudepot/pricing-history.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Append-only, oldest first.
    #[serde(default)]
    pub observations: Vec<RateObservation>,
}

impl Default for HistoryFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            observations: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("unsupported schema_version {0} (this build understands {SCHEMA_VERSION})")]
    UnsupportedSchemaVersion(u32),
    #[error("observation {0} has an out-of-range date")]
    BadDate(usize),
    #[error("observation {0} has an empty model_id")]
    EmptyModelId(usize),
    #[error("observation {0} has a negative rate")]
    NegativeRate(usize),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// The module segment is `pricing_history`, not `pricing` — two sibling
/// modules (`permission::grants`, `rotation::rules`) also name an enum
/// `ValidationError`, and one shared namespace would make the three
/// indistinguishable to a translator.
impl crate::error_code::ErrorCode for ValidationError {
    fn code(&self) -> &'static str {
        match self {
            ValidationError::UnsupportedSchemaVersion(_) => {
                "pricing_history.unsupported_schema_version"
            }
            ValidationError::BadDate(_) => "pricing_history.bad_date",
            ValidationError::EmptyModelId(_) => "pricing_history.empty_model_id",
            ValidationError::NegativeRate(_) => "pricing_history.negative_rate",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            // The message interpolates the build's own `SCHEMA_VERSION`
            // as well as the file's, so both cross — a translator needs
            // the pair, not just the number that was wrong.
            ValidationError::UnsupportedSchemaVersion(found) => {
                serde_json::json!({ "found": found, "expected": SCHEMA_VERSION })
            }
            // The index of the offending observation in the file.
            ValidationError::BadDate(index)
            | ValidationError::EmptyModelId(index)
            | ValidationError::NegativeRate(index) => serde_json::json!({ "index": index }),
        }
    }
}

impl HistoryFile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        for (i, o) in self.observations.iter().enumerate() {
            if o.model_id.trim().is_empty() {
                return Err(ValidationError::EmptyModelId(i));
            }
            if o.day().is_none() {
                return Err(ValidationError::BadDate(i));
            }
            let r = &o.rates;
            if r.input_per_mtok < 0.0
                || r.output_per_mtok < 0.0
                || r.cache_read_per_mtok < 0.0
                || r.cache_write_per_mtok < 0.0
            {
                return Err(ValidationError::NegativeRate(i));
            }
        }
        Ok(())
    }

    /// Observations for one model, oldest first.
    fn for_model<'a>(
        &'a self,
        model_id: &str,
    ) -> impl DoubleEndedIterator<Item = &'a RateObservation> {
        let owned = model_id.to_string();
        self.observations
            .iter()
            .filter(move |o| o.model_id == owned)
    }

    /// The most recently observed rate for a model, if any.
    pub fn latest_rate(&self, model_id: &str) -> Option<&ModelRates> {
        self.for_model(model_id).next_back().map(|o| &o.rates)
    }

    /// Every model id this install has observed, deduped.
    ///
    /// The book's snapshot needs these on top of the bundled ids: a
    /// model first seen by a scrape has no bundled entry, and omitting
    /// it would ship the renderer a book that can't price something the
    /// backend can.
    pub fn observed_model_ids(&self) -> impl Iterator<Item = String> + '_ {
        let unique: std::collections::BTreeSet<String> = self
            .observations
            .iter()
            .map(|o| o.model_id.clone())
            .collect();
        unique.into_iter()
    }

    /// Append `rates` as today's observation for `model_id`, but only
    /// when it differs from what we already believe. `believed` is the
    /// caller's current rate for the model (bundled or previously
    /// observed); passing `None` means "we had no rate", which always
    /// records.
    ///
    /// Returns `true` when an entry was appended.
    pub fn observe(
        &mut self,
        model_id: &str,
        rates: &ModelRates,
        believed: Option<&ModelRates>,
        on: Ymd,
    ) -> bool {
        // Compare against the newest observation first — that is what
        // we currently believe — falling back to the caller's value.
        let current = self.latest_rate(model_id).or(believed);
        if let Some(c) = current {
            if rates_equal(c, rates) {
                return false;
            }
        }
        self.observations.push(RateObservation {
            model_id: model_id.to_string(),
            observed_on: [on.0, on.1 as i32, on.2 as i32],
            rates: rates.clone(),
        });
        if self.observations.len() > MAX_OBSERVATIONS {
            let excess = self.observations.len() - MAX_OBSERVATIONS;
            self.observations.drain(..excess);
        }
        true
    }

    /// Merge the bundled periods for `model_id` with everything this
    /// install has observed, producing one chronological period list.
    ///
    /// An observation applies **from its own day forward**, and a
    /// bundled period that starts later still takes over on its own
    /// date. So a scrape that spots a change today refines the window
    /// we're currently in without disturbing either the documented
    /// past or a documented future change.
    ///
    /// This used to drop any observation dated before the *last*
    /// bundled period start, meaning to protect documented windows from
    /// a scrape's imprecise date. It protected the wrong thing: a model
    /// with a **scheduled future** period — Sonnet 5's standard rate
    /// starting 2026-09-01 — put the floor in the future, so every
    /// observation about *today* was silently discarded. Recording a
    /// rate change and then ignoring it is worse than either trusting
    /// or refusing it outright.
    ///
    /// The residual risk is a misread scrape inserting a wrong period
    /// from its date forward. It is bounded: `observe` only records on
    /// change, everything before the observation keeps its bundled
    /// rate, and the next bundled period still takes over on schedule.
    pub fn effective_periods(&self, model_id: &str, bundled: &[RatePeriod]) -> Vec<RatePeriod> {
        let mut out: Vec<RatePeriod> = bundled.to_vec();
        for o in self.for_model(model_id) {
            let Some(day) = o.day() else { continue };
            let period = RatePeriod {
                starts: Some(day),
                rates: to_live_rates(&o.rates),
            };
            // An observation on the same day as an existing period
            // replaces it rather than creating an ambiguous duplicate.
            match out.iter().position(|p| p.starts == Some(day)) {
                Some(i) => out[i] = period,
                None => out.push(period),
            }
        }
        out.sort_by_key(|p| p.starts);
        out
    }
}

/// Exact equality is the right test here: both sides come from the
/// same parse-and-derive path, so a rate that "should" be equal is
/// bit-identical. An epsilon would silently swallow a real
/// sub-cent change.
fn rates_equal(a: &ModelRates, b: &ModelRates) -> bool {
    a == b
}

/// Bridge the dashboard's `*_per_mtok` shape to the rate table's
/// `*_per_million_usd` shape. Same numbers, two struct definitions
/// that predate each other.
pub(crate) fn to_live_rates(r: &ModelRates) -> crate::session_live::pricing::ModelRates {
    crate::session_live::pricing::ModelRates {
        input_per_million_usd: r.input_per_mtok,
        output_per_million_usd: r.output_per_mtok,
        cache_read_per_million_usd: r.cache_read_per_mtok,
        cache_write_per_million_usd: r.cache_write_per_mtok,
    }
}

impl json_store::Validate for HistoryFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        HistoryFile::validate(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// `Validation` does not delegate to the inner code — `Display` is the
/// wrapper sentence `"validation: {0}"`, not the inner one.
impl crate::error_code::ErrorCode for HistoryStoreError {
    fn code(&self) -> &'static str {
        match self {
            HistoryStoreError::Io(_) => "pricing_history_store.io",
            HistoryStoreError::Validation(_) => "pricing_history_store.validation",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            HistoryStoreError::Io(e) => serde_json::json!({ "detail": e.to_string() }),
            HistoryStoreError::Validation(e) => serde_json::json!({ "detail": e.to_string() }),
        }
    }
}

/// Load the history file. A missing file is an empty history; a
/// corrupt one is moved aside and reported as empty.
pub fn load_from(path: &Path) -> Result<HistoryFile, HistoryStoreError> {
    Ok(json_store::load::<HistoryFile>(path, STORE)?)
}

/// Load from the standard location.
pub fn load() -> Result<HistoryFile, HistoryStoreError> {
    load_from(&history_path())
}

pub fn save_to(path: &Path, file: &HistoryFile) -> Result<(), HistoryStoreError> {
    json_store::save(path, file).map_err(|e| match e {
        SaveError::Validation(v) => HistoryStoreError::Validation(v),
        SaveError::Io(io) => HistoryStoreError::Io(io),
        SaveError::Serde(s) => HistoryStoreError::Io(std::io::Error::other(s)),
    })
}

/// Save to the standard location.
pub fn save(file: &HistoryFile) -> Result<(), HistoryStoreError> {
    if let Some(dir) = history_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    save_to(&history_path(), file)
}

/// Record every rate in `scraped` that differs from what we believe,
/// returning the model ids that moved. `believed` supplies the
/// current rate per model (typically the bundled table).
pub fn record_scrape(
    file: &mut HistoryFile,
    scraped: &BTreeMap<String, ModelRates>,
    believed: &BTreeMap<String, ModelRates>,
    on: Ymd,
) -> Vec<String> {
    let mut changed = Vec::new();
    for (id, rates) in scraped {
        if file.observe(id, rates, believed.get(id), on) {
            changed.push(id.clone());
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rates(input: f64) -> ModelRates {
        ModelRates {
            input_per_mtok: input,
            output_per_mtok: input * 5.0,
            cache_write_per_mtok: input * 1.25,
            cache_read_per_mtok: input * 0.10,
        }
    }

    fn bundled_flat(input: f64) -> Vec<RatePeriod> {
        vec![RatePeriod {
            starts: None,
            rates: to_live_rates(&rates(input)),
        }]
    }

    // ── observe ────────────────────────────────────────────────────

    #[test]
    fn an_unchanged_rate_is_not_recorded() {
        // The scraper runs daily. Recording every run would bury the
        // actual changes in thousands of identical rows.
        let mut f = HistoryFile::default();
        assert!(!f.observe(
            "claude-opus-5",
            &rates(5.0),
            Some(&rates(5.0)),
            (2026, 7, 25)
        ));
        assert!(f.observations.is_empty());
    }

    #[test]
    fn a_changed_rate_is_recorded_with_the_day_it_was_seen() {
        let mut f = HistoryFile::default();
        assert!(f.observe(
            "claude-opus-5",
            &rates(6.0),
            Some(&rates(5.0)),
            (2026, 8, 1)
        ));
        assert_eq!(f.observations.len(), 1);
        assert_eq!(f.observations[0].model_id, "claude-opus-5");
        assert_eq!(f.observations[0].observed_on, [2026, 8, 1]);
        assert_eq!(f.observations[0].rates.input_per_mtok, 6.0);
    }

    #[test]
    fn a_first_sighting_with_no_prior_belief_is_recorded() {
        let mut f = HistoryFile::default();
        assert!(f.observe("claude-new-9", &rates(7.0), None, (2026, 8, 1)));
    }

    #[test]
    fn the_newest_observation_is_what_a_repeat_compares_against() {
        // After recording a change, seeing the same new rate again
        // must not append a second identical row — even though the
        // caller keeps passing the stale bundled rate as `believed`.
        let mut f = HistoryFile::default();
        assert!(f.observe(
            "claude-opus-5",
            &rates(6.0),
            Some(&rates(5.0)),
            (2026, 8, 1)
        ));
        assert!(!f.observe(
            "claude-opus-5",
            &rates(6.0),
            Some(&rates(5.0)),
            (2026, 8, 2)
        ));
        assert_eq!(f.observations.len(), 1);
    }

    #[test]
    fn observations_are_capped_dropping_the_oldest() {
        let mut f = HistoryFile::default();
        for i in 0..(MAX_OBSERVATIONS + 10) {
            // Alternate the rate so every call records.
            let r = rates(if i % 2 == 0 { 1.0 } else { 2.0 });
            f.observe("m", &r, None, (2026, 1, 1));
        }
        assert_eq!(f.observations.len(), MAX_OBSERVATIONS);
    }

    // ── effective_periods ──────────────────────────────────────────

    #[test]
    fn with_no_observations_the_bundled_periods_stand_alone() {
        let f = HistoryFile::default();
        let bundled = bundled_flat(5.0);
        assert_eq!(f.effective_periods("claude-opus-5", &bundled), bundled);
    }

    #[test]
    fn an_observation_appends_a_period_from_the_day_it_was_seen() {
        let mut f = HistoryFile::default();
        f.observe(
            "claude-opus-5",
            &rates(6.0),
            Some(&rates(5.0)),
            (2026, 8, 1),
        );
        let periods = f.effective_periods("claude-opus-5", &bundled_flat(5.0));
        assert_eq!(periods.len(), 2);
        assert_eq!(periods[0].starts, None);
        assert_eq!(periods[0].rates.input_per_million_usd, 5.0);
        assert_eq!(periods[1].starts, Some((2026, 8, 1)));
        assert_eq!(periods[1].rates.input_per_million_usd, 6.0);
    }

    #[test]
    fn an_observation_refines_the_current_window_without_touching_a_scheduled_change() {
        // Sonnet 5's shape: an open window now, a documented change on
        // 2026-09-01. An observation today must land between them —
        // the old "ignore anything before the last bundled start" rule
        // put the floor in the *future* and threw this away.
        let mut f = HistoryFile::default();
        f.observe("claude-sonnet-5", &rates(9.0), None, (2026, 8, 15));
        let bundled = vec![
            RatePeriod {
                starts: None,
                rates: to_live_rates(&rates(2.0)),
            },
            RatePeriod {
                starts: Some((2026, 9, 1)),
                rates: to_live_rates(&rates(3.0)),
            },
        ];
        let periods = f.effective_periods("claude-sonnet-5", &bundled);
        let starts: Vec<_> = periods.iter().map(|p| p.starts).collect();
        assert_eq!(
            starts,
            vec![None, Some((2026, 8, 15)), Some((2026, 9, 1))],
            "the observation slots in; the scheduled change survives"
        );
        // Before the observation: still the documented opening rate.
        assert_eq!(periods[0].rates.input_per_million_usd, 2.0);
        // From the observation: what we saw.
        assert_eq!(periods[1].rates.input_per_million_usd, 9.0);
        // From the scheduled date: the documented change, undisturbed.
        assert_eq!(periods[2].rates.input_per_million_usd, 3.0);
    }

    #[test]
    fn an_observation_on_an_existing_period_day_replaces_it() {
        let mut f = HistoryFile::default();
        f.observe("m", &rates(4.0), None, (2026, 9, 1));
        let bundled = vec![
            RatePeriod {
                starts: None,
                rates: to_live_rates(&rates(2.0)),
            },
            RatePeriod {
                starts: Some((2026, 9, 1)),
                rates: to_live_rates(&rates(3.0)),
            },
        ];
        let periods = f.effective_periods("m", &bundled);
        assert_eq!(periods.len(), 2, "no duplicate period for the same day");
        assert_eq!(periods[1].rates.input_per_million_usd, 4.0);
    }

    #[test]
    fn periods_come_back_in_chronological_order() {
        let mut f = HistoryFile::default();
        f.observe("m", &rates(6.0), None, (2026, 10, 1));
        f.observe("m", &rates(7.0), None, (2026, 8, 1));
        let periods = f.effective_periods("m", &bundled_flat(5.0));
        let starts: Vec<_> = periods.iter().map(|p| p.starts).collect();
        assert_eq!(starts, vec![None, Some((2026, 8, 1)), Some((2026, 10, 1))]);
    }

    #[test]
    fn observations_for_other_models_do_not_leak() {
        let mut f = HistoryFile::default();
        f.observe("claude-opus-5", &rates(6.0), None, (2026, 8, 1));
        let periods = f.effective_periods("claude-haiku-4-5", &bundled_flat(1.0));
        assert_eq!(periods.len(), 1);
    }

    // ── record_scrape ──────────────────────────────────────────────

    #[test]
    fn record_scrape_reports_only_the_models_that_moved() {
        let mut f = HistoryFile::default();
        let mut believed = BTreeMap::new();
        believed.insert("a".to_string(), rates(1.0));
        believed.insert("b".to_string(), rates(2.0));
        let mut scraped = BTreeMap::new();
        scraped.insert("a".to_string(), rates(1.0)); // unchanged
        scraped.insert("b".to_string(), rates(9.0)); // moved
        let changed = record_scrape(&mut f, &scraped, &believed, (2026, 8, 1));
        assert_eq!(changed, vec!["b".to_string()]);
    }

    // ── validation ─────────────────────────────────────────────────

    #[test]
    fn a_future_schema_version_is_rejected() {
        let f = HistoryFile {
            schema_version: SCHEMA_VERSION + 1,
            observations: Vec::new(),
        };
        assert_eq!(
            f.validate(),
            Err(ValidationError::UnsupportedSchemaVersion(
                SCHEMA_VERSION + 1
            ))
        );
    }

    #[test]
    fn an_out_of_range_date_is_rejected() {
        let f = HistoryFile {
            schema_version: SCHEMA_VERSION,
            observations: vec![RateObservation {
                model_id: "m".into(),
                observed_on: [2026, 13, 1],
                rates: rates(1.0),
            }],
        };
        assert_eq!(f.validate(), Err(ValidationError::BadDate(0)));
    }

    #[test]
    fn an_impossible_calendar_date_is_rejected() {
        // In range month-wise and day-wise, but not a real day. A plain
        // 1..=31 check would accept it, and it would then sort between
        // real February and March days.
        for bad in [[2026, 2, 31], [2026, 2, 30], [2027, 2, 29], [2026, 4, 31]] {
            let f = HistoryFile {
                schema_version: SCHEMA_VERSION,
                observations: vec![RateObservation {
                    model_id: "m".into(),
                    observed_on: bad,
                    rates: rates(1.0),
                }],
            };
            assert_eq!(
                f.validate(),
                Err(ValidationError::BadDate(0)),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn a_leap_day_is_accepted() {
        let f = HistoryFile {
            schema_version: SCHEMA_VERSION,
            observations: vec![RateObservation {
                model_id: "m".into(),
                observed_on: [2028, 2, 29],
                rates: rates(1.0),
            }],
        };
        assert_eq!(f.validate(), Ok(()));
    }

    #[test]
    fn observed_model_ids_are_unique() {
        let mut f = HistoryFile::default();
        f.observe("a", &rates(1.0), None, (2026, 1, 1));
        f.observe("a", &rates(2.0), None, (2026, 1, 2));
        f.observe("b", &rates(3.0), None, (2026, 1, 3));
        let ids: Vec<String> = f.observed_model_ids().collect();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn a_negative_rate_is_rejected() {
        let f = HistoryFile {
            schema_version: SCHEMA_VERSION,
            observations: vec![RateObservation {
                model_id: "m".into(),
                observed_on: [2026, 1, 1],
                rates: rates(-1.0),
            }],
        };
        assert_eq!(f.validate(), Err(ValidationError::NegativeRate(0)));
    }

    #[test]
    fn an_empty_model_id_is_rejected() {
        let f = HistoryFile {
            schema_version: SCHEMA_VERSION,
            observations: vec![RateObservation {
                model_id: "  ".into(),
                observed_on: [2026, 1, 1],
                rates: rates(1.0),
            }],
        };
        assert_eq!(f.validate(), Err(ValidationError::EmptyModelId(0)));
    }

    // ── persistence ────────────────────────────────────────────────

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILENAME);
        let mut f = HistoryFile::default();
        f.observe(
            "claude-opus-5",
            &rates(6.0),
            Some(&rates(5.0)),
            (2026, 8, 1),
        );
        save_to(&path, &f).unwrap();
        assert_eq!(load_from(&path).unwrap(), f);
    }

    #[test]
    fn a_missing_file_loads_as_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_from(&dir.path().join("nope.json")).unwrap();
        assert_eq!(loaded, HistoryFile::default());
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_loads_as_empty() {
        // Losing observations degrades accuracy but strands no
        // obligation, so recovery here is silent-and-usable rather
        // than fail-loud.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILENAME);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(load_from(&path).unwrap(), HistoryFile::default());
        assert!(!path.exists(), "corrupt file should have been renamed");
    }

    #[test]
    fn saving_an_invalid_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILENAME);
        let bad = HistoryFile {
            schema_version: SCHEMA_VERSION,
            observations: vec![RateObservation {
                model_id: "m".into(),
                observed_on: [2026, 99, 1],
                rates: rates(1.0),
            }],
        };
        assert!(save_to(&path, &bad).is_err());
        assert!(!path.exists(), "an invalid file must never land on disk");
    }
}
