//! Model pricing — canonicalize CC-reported model ids and look up the
//! per-million-token rates that were in force when the tokens were
//! spent.
//!
//! Baked table, not network-fetched. Per plan §11 decision 2, we
//! prefer predictable (bound at release time) to "accurate" (requires
//! an internet round-trip in a hot path). Observed rate changes are
//! layered on top at runtime by [`crate::pricing::history`].
//!
//! # Rates are dated
//!
//! Each model carries a list of [`RatePeriod`]s rather than one rate,
//! and [`resolve_rates_on`] takes the day the usage happened. A
//! session from the Sonnet 5 introductory window scores at $2/$10; the
//! same session's tokens spent in September score at $3/$15. Scoring
//! everything at today's rate silently rewrites the past every time
//! Anthropic changes a price.
//!
//! # Resolution
//!
//! The canonicalizer strips CC's dated suffixes (e.g.
//! `claude-haiku-4-5-20251001` → `claude-haiku-4-5`) before lookup so
//! every release-series-compatible id hits the same row. An id we
//! don't list falls back to its family's current model and is marked
//! [`RateConfidence::FamilyEstimate`] so the UI can flag the figure
//! rather than passing a guess off as a quote. An id from no family we
//! price returns `None`; the UI renders `—`, not `$0.00`.

use once_cell::sync::Lazy;
use regex::Regex;

/// Rates in US dollars per million tokens, matching Anthropic's
/// published "standard" tier. All four token classes are priced here;
/// see [`RATE_TIERS`] for the values and the date they were verified.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelRates {
    pub input_per_million_usd: f64,
    pub output_per_million_usd: f64,
    pub cache_read_per_million_usd: f64,
    pub cache_write_per_million_usd: f64,
}

/// Canonicalize a CC-reported model id to its release-series form.
/// Rules, in order:
///   1. Trim trailing ` -YYYYMMDD` date suffix (common on Haiku ids).
///   2. Lowercase.
///   3. Strip a trailing `-preview` / `-latest` / `-experimental`
///      marker that CC sometimes appends for alias resolution.
pub fn canonicalize_model_id(raw: &str) -> String {
    static DATE_SUFFIX_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"-\d{8}$").expect("static regex"));
    static ALIAS_SUFFIX_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"-(preview|latest|experimental)$").expect("static regex"));
    let lower = raw.to_ascii_lowercase();
    let no_date = DATE_SUFFIX_RE.replace(&lower, "").into_owned();
    ALIAS_SUFFIX_RE.replace(&no_date, "").into_owned()
}

/// A calendar day as `(year, month, day)`.
///
/// Tuple ordering is lexicographic, which for this field order is also
/// chronological — so `<=` on a `Ymd` is a date comparison, with no
/// timezone or leap-second subtlety to get wrong. Rates change on
/// calendar-day boundaries in Anthropic's announcements, so a day is
/// the right resolution.
pub type Ymd = (i32, u32, u32);

/// A rate that took effect on a day and held until the next period
/// began.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatePeriod {
    /// First day this rate applied. `None` means "in force since
    /// before Claudepot tracked this model" — the opening period.
    pub starts: Option<Ymd>,
    pub rates: ModelRates,
}

/// How well a resolved rate matches the model it was asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateConfidence {
    /// The model id is listed in the table and the date falls inside a
    /// recorded period. The figure is as good as our published-rate
    /// knowledge.
    Exact,
    /// No entry for this id, so the rate is its family's current one.
    /// Correct often enough to be worth showing, wrong often enough
    /// that the UI must mark it.
    FamilyEstimate,
}

/// A rate plus how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedRates {
    pub rates: ModelRates,
    pub confidence: RateConfidence,
}

/// Rate lookup for a model on a given day.
///
/// Two stages, and the confidence marker says which one answered:
///
/// 1. **Exact** — the canonicalized id is in [`RATE_TIERS`]; pick the
///    period covering `on`.
/// 2. **Family estimate** — the id isn't listed, but its family is;
///    use that family's current model's rate for `on`.
///
/// `None` only for ids from no family we price, which the UI renders
/// as `—` rather than `$0.00`.
///
/// **Bundled rates only.** This is the low-level lookup over
/// [`RATE_TIERS`]; it does not consult this install's observed rate
/// history and does not apply the billing tier's multiplier. Cost
/// figures must go through [`crate::pricing::PriceBook::resolve`],
/// which layers both on top — `src/costs.ts` mirrors *that*, and the
/// shared fixture in `testdata/rate-resolution-vectors.json` locks
/// the two together. Kept crate-visible so `PriceBook` and the table
/// integrity tests can reach it without re-opening a second public
/// resolver.
pub(crate) fn resolve_rates_on(model: &str, on: Ymd) -> Option<ResolvedRates> {
    let key = canonicalize_model_id(model);
    if let Some(rates) = periods_for_id(&key).and_then(|p| rate_on(p, on)) {
        return Some(ResolvedRates {
            rates,
            confidence: RateConfidence::Exact,
        });
    }
    let current = family_current_id(&key)?;
    let rates = periods_for_id(current).and_then(|p| rate_on(p, on))?;
    Some(ResolvedRates {
        rates,
        confidence: RateConfidence::FamilyEstimate,
    })
}

/// Rate lookup for a model *right now*.
///
/// Correct for the live Activity strip, where the tokens are being
/// spent as we read them. Anything scoring historical usage must call
/// [`resolve_rates_on`] with that usage's own date, or a session from
/// before a price change is re-scored at today's rate.
pub fn rates_for(model: &str) -> Option<ModelRates> {
    resolve_rates_on(model, today_utc()).map(|r| r.rates)
}

/// Today in UTC. Anthropic publishes rate-change dates without a
/// timezone; UTC is the least surprising reading and keeps the
/// boundary from moving with the user's locale.
pub fn today_utc() -> Ymd {
    use chrono::Datelike;
    let d = chrono::Utc::now().date_naive();
    (d.year(), d.month(), d.day())
}

/// Convert an epoch-millisecond timestamp to a UTC calendar day.
/// Returns `None` for a timestamp outside the representable range.
pub fn ymd_from_ms(ts_ms: i64) -> Option<Ymd> {
    use chrono::Datelike;
    let d = chrono::DateTime::from_timestamp_millis(ts_ms)?.date_naive();
    Some((d.year(), d.month(), d.day()))
}

/// Every model id the bundled table prices, with the rate periods each
/// has moved through.
///
/// **This is the single source of truth for the bundled rates.**
/// [`resolve_rates_on`] resolves against it and
/// [`crate::pricing::bundled`] seeds its dashboard table from
/// [`priced_model_ids`], so adding a model here is the only edit a new
/// model release needs. Before this was one table, the id list lived
/// in two files and silently drifted — a model present in one and
/// absent from the other priced correctly in the Activity strip and
/// showed as "unpriced" in the Cost dashboard.
///
/// Ids inside a group share a rate *and its whole history*. Split a
/// group the moment one member's price diverges: Sonnet 5 and Sonnet
/// 4.6 shared a group until Sonnet 5's introductory window made their
/// histories differ.
///
/// Periods are listed oldest-first and must stay that way;
/// `periods_are_sorted_oldest_first` enforces it.
///
/// Rates verified against Anthropic's published model pricing on
/// 2026-07-25 (USD per million tokens; `cache_read` = 0.1× input,
/// `cache_write` = 1.25× input, per Anthropic's cache multipliers).
///
/// **Known gap:** fast mode bills Opus 5 and Opus 4.8 at $10/$50
/// rather than $5/$25, and CC's transcripts carry no fast-mode marker,
/// so a fast-mode session is under-reported here. Recording it would
/// need a signal we don't have; see `docs` on the fast-mode toggle.
const RATE_TIERS: &[(&[&str], &[RatePeriod])] = &[
    // Opus 5 / 4.8 / 4.7 / 4.6 — the standard Opus tier ($5 / $25).
    // NOT the old $15 / $75 tier: Anthropic dropped Opus pricing with
    // the 4.5+ generation, and the bundled table had never been
    // updated (every Opus cost was inflated 3×). Opus 5 shipped at the
    // same $5 / $25, so it joins the tier rather than opening a new one.
    (
        &[
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
        ],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 5.0,
                output_per_million_usd: 25.0,
                cache_read_per_million_usd: 0.5,
                cache_write_per_million_usd: 6.25,
            },
        }],
    ),
    // Opus 4.1 — the retired $15 / $75 tier. Deprecated, retiring
    // 2026-08-05, but transcripts from its era are still on disk and
    // scoring them at the current Opus rate understates them 3×.
    (
        &["claude-opus-4-1"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 15.0,
                output_per_million_usd: 75.0,
                cache_read_per_million_usd: 1.5,
                cache_write_per_million_usd: 18.75,
            },
        }],
    ),
    // Sonnet 5 — introductory $2 / $10 through 2026-08-31, standard
    // $3 / $15 from 2026-09-01. This is the price history the dated
    // table exists for: before it, we baked the standard rate so the
    // figure wouldn't under-report *later*, which meant over-reporting
    // every session run during the introductory window.
    (
        &["claude-sonnet-5"],
        &[
            RatePeriod {
                starts: None,
                rates: ModelRates {
                    input_per_million_usd: 2.0,
                    output_per_million_usd: 10.0,
                    cache_read_per_million_usd: 0.2,
                    cache_write_per_million_usd: 2.5,
                },
            },
            RatePeriod {
                starts: Some((2026, 9, 1)),
                rates: ModelRates {
                    input_per_million_usd: 3.0,
                    output_per_million_usd: 15.0,
                    cache_read_per_million_usd: 0.3,
                    cache_write_per_million_usd: 3.75,
                },
            },
        ],
    ),
    // Sonnet 4.6 / 4.5 — $3 / $15 throughout, no introductory window.
    (
        &["claude-sonnet-4-6", "claude-sonnet-4-5"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 3.0,
                output_per_million_usd: 15.0,
                cache_read_per_million_usd: 0.3,
                cache_write_per_million_usd: 3.75,
            },
        }],
    ),
    // Fable 5 / Mythos 5 — most capable tier, above Opus ($10 / $50).
    (
        &["claude-fable-5", "claude-mythos-5"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 10.0,
                output_per_million_usd: 50.0,
                cache_read_per_million_usd: 1.0,
                cache_write_per_million_usd: 12.5,
            },
        }],
    ),
    // Haiku 4.5 — $1 / $5.
    (
        &["claude-haiku-4-5"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 1.0,
                output_per_million_usd: 5.0,
                cache_read_per_million_usd: 0.1,
                cache_write_per_million_usd: 1.25,
            },
        }],
    ),
];

/// The model whose rate stands in for an unlisted member of a family.
///
/// Explicit rather than "the first tier mentioning this family",
/// because a family can span tiers — `claude-opus-` covers both the
/// current $5/$25 tier and retired Opus 4.1's $15/$75 — and an
/// order-dependent scan would silently pick whichever happened to be
/// declared first. `family_targets_are_priced` checks every target
/// resolves.
const FAMILY_CURRENT: &[(&str, &str)] = &[
    ("claude-opus-", "claude-opus-5"),
    ("claude-sonnet-", "claude-sonnet-5"),
    ("claude-haiku-", "claude-haiku-4-5"),
    ("claude-fable-", "claude-fable-5"),
    // Mythos shares Fable's specs and pricing but has its own family.
    ("claude-mythos-", "claude-fable-5"),
];

/// Claude Code's placeholder in a transcript's `model` field for an
/// assistant turn **it generated locally**, not one the API produced:
/// `API Error: 403 Request not allowed`, `No response requested.`,
/// interruption notices. Verified against real transcripts — every
/// such turn carries `usage` with all four token counts at zero,
/// because nothing was ever sent.
///
/// It is not a model, and treating it as one is expensive rather than
/// merely untidy. `SessionRow::models` is a `BTreeSet`, so it
/// serializes sorted, and `<` (0x3C) sorts before every lowercase
/// letter — so `["<synthetic>", "claude-opus-5"]` puts the
/// placeholder FIRST. Both cost estimators pick the leading element
/// (Rust takes the minimum, `src/costs.ts` takes `models[0]`), so
/// every session that ever hit one API error resolved to an unpriced
/// "model" and dropped out of the cost total entirely — tokens still
/// counted, dollars silently missing.
///
/// Measured on this repo's reference machine: 186 of 1,817 sessions,
/// carrying 130 billion cache-read and 444 million output tokens —
/// the majority of the machine's spend. It reads as "the price table
/// is missing my newest models" (issue #91), which is why the
/// distinction is written down here rather than fixed silently.
///
/// Filtered where the value is READ rather than where the index is
/// written, so existing `sessions.db` files report correct costs with
/// no rebuild.
pub fn is_synthetic_model(id: &str) -> bool {
    id == SYNTHETIC_MODEL
}

/// CC's literal placeholder. Exact match only — a real model id will
/// never contain angle brackets, and a substring test would be a
/// silent over-filter.
pub const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Every canonical model id the bundled table prices, in tier order.
/// [`crate::pricing::bundled`] iterates this to build the dashboard's
/// price table, so the two can't disagree about which models exist.
pub fn priced_model_ids() -> impl Iterator<Item = &'static str> {
    RATE_TIERS.iter().flat_map(|(ids, _)| ids.iter().copied())
}

/// The family-fallback map as `(family_prefix, current_model_id)`
/// pairs, for transport to surfaces that resolve rates themselves.
pub fn family_current_map() -> impl Iterator<Item = (&'static str, &'static str)> {
    FAMILY_CURRENT.iter().copied()
}

/// The rate periods recorded for a canonical id, if it's listed.
pub fn periods_for_id(id: &str) -> Option<&'static [RatePeriod]> {
    RATE_TIERS
        .iter()
        .find(|(ids, _)| ids.contains(&id))
        .map(|(_, periods)| *periods)
}

/// The last period that had begun on or before `on`. `None` when every
/// recorded period starts after `on` — a model priced only from a
/// later date, which must not be scored at a rate that didn't exist.
///
/// Requires `periods` to be ordered oldest-first, which
/// [`RATE_TIERS`] guarantees and
/// [`crate::pricing::history::HistoryFile::effective_periods`]
/// preserves when it merges observations in.
pub fn rate_on(periods: &[RatePeriod], on: Ymd) -> Option<ModelRates> {
    periods
        .iter()
        .rev()
        .find(|p| p.starts.is_none_or(|start| start <= on))
        .map(|p| p.rates)
}

/// The stand-in model for `id`'s family, per [`FAMILY_CURRENT`].
/// `None` when the id belongs to no family we price.
pub fn family_current_id(id: &str) -> Option<&'static str> {
    let family = family_prefix(id)?;
    FAMILY_CURRENT
        .iter()
        .find(|(prefix, _)| *prefix == family)
        .map(|(_, current)| *current)
}

/// `claude-opus-5` → `claude-opus-`; `claude-haiku-4-5` →
/// `claude-haiku-`. `None` for anything not shaped like
/// `claude-<family>-…`, so a non-Claude id falls out cleanly instead
/// of matching a family it has nothing to do with.
fn family_prefix(id: &str) -> Option<&str> {
    const CLAUDE: &str = "claude-";
    let rest = id.strip_prefix(CLAUDE)?;
    let dash = rest.find('-')?;
    Some(&id[..CLAUDE.len() + dash + 1])
}

/// Apply rates to token counts. The one place the cost arithmetic
/// lives, so every surface weights the four token classes identically.
pub fn apply_rates(
    r: &ModelRates,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> f64 {
    let million = 1_000_000.0;
    (input_tokens as f64 / million) * r.input_per_million_usd
        + (output_tokens as f64 / million) * r.output_per_million_usd
        + (cache_read_tokens as f64 / million) * r.cache_read_per_million_usd
        + (cache_write_tokens as f64 / million) * r.cache_write_per_million_usd
}

/// Compute estimated cost in USD at today's rates. Returns `None` when
/// the model belongs to no family we price — callers should not invent
/// a number. Historical usage must use [`resolve_rates_on`] +
/// [`apply_rates`] with its own date instead.
pub fn estimate_cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let r = rates_for(model)?;
    Some(apply_rates(
        &r,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Canonicalization ───────────────────────────────────────────

    #[test]
    fn strips_date_suffix() {
        assert_eq!(
            canonicalize_model_id("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn strips_alias_suffix() {
        assert_eq!(
            canonicalize_model_id("claude-sonnet-4-6-preview"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            canonicalize_model_id("claude-opus-4-7-latest"),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn canonicalize_is_lowercase() {
        assert_eq!(canonicalize_model_id("Claude-Opus-4-7"), "claude-opus-4-7");
    }

    #[test]
    fn canonicalize_strips_date_before_alias() {
        // CC doesn't produce this combination but the regex order
        // should tolerate it anyway.
        assert_eq!(
            canonicalize_model_id("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn canonicalize_passes_unknown_through() {
        // An arbitrary non-Claude id should survive so the rates
        // lookup can return None cleanly.
        assert_eq!(canonicalize_model_id("gpt-4"), "gpt-4");
    }

    // ── Exact rate lookup ──────────────────────────────────────────

    /// A day inside the Sonnet 5 introductory window.
    const DURING_INTRO: Ymd = (2026, 7, 25);
    /// The first day the Sonnet 5 standard rate applies.
    const AFTER_INTRO: Ymd = (2026, 9, 1);

    fn exact_on(model: &str, on: Ymd) -> ModelRates {
        let r = resolve_rates_on(model, on).unwrap();
        assert_eq!(
            r.confidence,
            RateConfidence::Exact,
            "{model} should resolve exactly"
        );
        r.rates
    }

    #[test]
    fn exact_rates_for_known_ids() {
        // Current-generation Opus is $5 / $25 (the standard tier), not
        // the retired $15 / $75.
        let opus = exact_on("claude-opus-5", DURING_INTRO);
        assert_eq!(opus.input_per_million_usd, 5.0);
        assert_eq!(opus.output_per_million_usd, 25.0);
        for id in [
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
        ] {
            assert_eq!(exact_on(id, DURING_INTRO), opus);
        }

        let son = exact_on("claude-sonnet-4-6", DURING_INTRO);
        assert_eq!(son.input_per_million_usd, 3.0);
        assert_eq!(son.output_per_million_usd, 15.0);
        assert_eq!(exact_on("claude-sonnet-4-5", DURING_INTRO), son);

        let fable = exact_on("claude-fable-5", DURING_INTRO);
        assert_eq!(fable.input_per_million_usd, 10.0);
        assert_eq!(fable.output_per_million_usd, 50.0);
        assert_eq!(exact_on("claude-mythos-5", DURING_INTRO), fable);

        let hai = exact_on("claude-haiku-4-5", DURING_INTRO);
        assert_eq!(hai.input_per_million_usd, 1.0);
        assert_eq!(hai.output_per_million_usd, 5.0);
    }

    #[test]
    fn retired_opus_keeps_its_own_higher_rate() {
        // Opus 4.1 billed at $15 / $75. Folding it into the current
        // Opus tier would understate every transcript from its era 3×.
        let old = exact_on("claude-opus-4-1", DURING_INTRO);
        assert_eq!(old.input_per_million_usd, 15.0);
        assert_eq!(old.output_per_million_usd, 75.0);
        let current = exact_on("claude-opus-5", DURING_INTRO);
        assert_ne!(old, current);
    }

    #[test]
    fn dated_model_id_resolves_to_family_rate() {
        let a = rates_for("claude-haiku-4-5-20251001").unwrap();
        let b = rates_for("claude-haiku-4-5").unwrap();
        assert_eq!(a, b, "dated id must canonicalize to undated");
    }

    // ── Price history ──────────────────────────────────────────────

    #[test]
    fn sonnet_5_prices_at_the_introductory_rate_inside_the_window() {
        let intro = exact_on("claude-sonnet-5", DURING_INTRO);
        assert_eq!(intro.input_per_million_usd, 2.0);
        assert_eq!(intro.output_per_million_usd, 10.0);
    }

    #[test]
    fn sonnet_5_prices_at_the_standard_rate_after_the_window() {
        let standard = exact_on("claude-sonnet-5", AFTER_INTRO);
        assert_eq!(standard.input_per_million_usd, 3.0);
        assert_eq!(standard.output_per_million_usd, 15.0);
        // ...and every day after, not just the boundary day.
        assert_eq!(exact_on("claude-sonnet-5", (2027, 3, 14)), standard);
    }

    #[test]
    fn the_period_boundary_is_the_first_day_of_the_new_rate() {
        // 2026-08-31 is the last introductory day; 2026-09-01 is the
        // first standard day. An off-by-one here misprices a whole day
        // of usage.
        assert_eq!(
            exact_on("claude-sonnet-5", (2026, 8, 31)).input_per_million_usd,
            2.0
        );
        assert_eq!(
            exact_on("claude-sonnet-5", (2026, 9, 1)).input_per_million_usd,
            3.0
        );
    }

    #[test]
    fn a_flat_rate_model_is_unaffected_by_date() {
        let early = exact_on("claude-opus-5", (2025, 1, 1));
        let late = exact_on("claude-opus-5", (2030, 12, 31));
        assert_eq!(early, late);
    }

    #[test]
    fn rate_on_returns_none_before_the_first_recorded_period() {
        // A model we only start pricing from a given date must not be
        // scored at a rate that didn't exist yet.
        let periods = [RatePeriod {
            starts: Some((2026, 6, 1)),
            rates: ModelRates {
                input_per_million_usd: 1.0,
                output_per_million_usd: 2.0,
                cache_read_per_million_usd: 0.1,
                cache_write_per_million_usd: 1.25,
            },
        }];
        assert!(rate_on(&periods, (2026, 5, 31)).is_none());
        assert!(rate_on(&periods, (2026, 6, 1)).is_some());
    }

    #[test]
    fn ymd_from_ms_converts_epoch_millis_to_a_utc_day() {
        assert_eq!(ymd_from_ms(0), Some((1970, 1, 1)));
        // 2026-07-25T00:00:00Z, cross-checked against chrono's own
        // parser rather than a hand-written constant.
        let midnight = "2026-07-25T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            .timestamp_millis();
        assert_eq!(ymd_from_ms(midnight), Some((2026, 7, 25)));
        // Last millisecond of the same UTC day stays on that day.
        assert_eq!(
            ymd_from_ms(midnight + 86_400_000 - 1),
            Some((2026, 7, 25)),
            "23:59:59.999Z must not roll into the next day"
        );
        assert_eq!(ymd_from_ms(midnight + 86_400_000), Some((2026, 7, 26)));
    }

    // ── Tier table integrity ───────────────────────────────────────

    #[test]
    fn every_priced_id_resolves() {
        // `pricing::bundled()` builds its dashboard table by calling
        // `rates_for` on each of these. An id advertised here that
        // doesn't resolve would land in that table as a silent hole.
        for id in priced_model_ids() {
            assert!(
                rates_for(id).is_some(),
                "{id} is advertised by priced_model_ids but has no rate"
            );
        }
    }

    #[test]
    fn priced_ids_are_unique_across_tiers() {
        // An id listed twice resolves by declaration order, which is
        // exactly the ambiguity splitting Sonnet 5 out was meant to end.
        let ids: Vec<&str> = priced_model_ids().collect();
        let mut seen = std::collections::BTreeSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "{id} appears in more than one tier");
        }
    }

    #[test]
    fn priced_ids_are_already_canonical() {
        // The ids seed a table that's looked up by canonicalized key,
        // so a tier entry carrying a date or alias suffix would be
        // unreachable.
        for id in priced_model_ids() {
            assert_eq!(canonicalize_model_id(id), id, "{id} is not canonical");
        }
    }

    #[test]
    fn periods_are_sorted_oldest_first() {
        // `rate_on` scans in reverse and takes the first period that
        // has begun, which is only the *latest* such period when the
        // list is ordered. An out-of-order entry silently misprices.
        for (ids, periods) in RATE_TIERS {
            let mut prev: Option<Ymd> = None;
            for (i, p) in periods.iter().enumerate() {
                if i == 0 {
                    prev = p.starts;
                    continue;
                }
                let start = p.starts.unwrap_or_else(|| {
                    panic!("{ids:?}: only the opening period may have starts: None")
                });
                if let Some(prev_start) = prev {
                    assert!(prev_start < start, "{ids:?}: periods out of order");
                }
                prev = Some(start);
            }
        }
    }

    #[test]
    fn family_targets_are_priced() {
        // Every family fallback must name a model the table actually
        // prices, or an unlisted model in that family resolves to None
        // instead of an estimate.
        for (family, current) in FAMILY_CURRENT {
            assert!(
                periods_for_id(current).is_some(),
                "family {family} falls back to unpriced {current}"
            );
        }
    }

    #[test]
    fn every_priced_family_has_a_fallback() {
        // A family in the table but missing from FAMILY_CURRENT means
        // its next release resolves to None rather than an estimate.
        for id in priced_model_ids() {
            let family = family_prefix(id).expect("tier ids are claude-<family>-…");
            assert!(
                FAMILY_CURRENT.iter().any(|(f, _)| *f == family),
                "family {family} (from {id}) has no FAMILY_CURRENT entry"
            );
        }
    }

    // ── Prefix fallback ────────────────────────────────────────────

    fn estimated_on(model: &str, on: Ymd) -> ModelRates {
        let r = resolve_rates_on(model, on).unwrap();
        assert_eq!(
            r.confidence,
            RateConfidence::FamilyEstimate,
            "{model} should resolve as an estimate"
        );
        r.rates
    }

    #[test]
    fn future_point_release_falls_back_to_family() {
        // `claude-opus-4-9` isn't in the table yet; the family fallback
        // returns the current Opus rate until a release updates the
        // baked table — and marks it as an estimate.
        let future = estimated_on("claude-opus-4-9", DURING_INTRO);
        assert_eq!(future, exact_on("claude-opus-5", DURING_INTRO));
    }

    #[test]
    fn future_generation_falls_back_to_family() {
        // The dateless `claude-<family>-<N>` shape (Opus 5's shape) has
        // to fall back too — this is what broke when Opus 5 shipped and
        // only `claude-opus-4-*` ids were listed.
        assert_eq!(
            estimated_on("claude-opus-6", DURING_INTRO),
            exact_on("claude-opus-5", DURING_INTRO)
        );
        assert_eq!(
            estimated_on("claude-sonnet-6", DURING_INTRO),
            exact_on("claude-sonnet-5", DURING_INTRO)
        );
    }

    #[test]
    fn a_family_estimate_is_still_dated() {
        // The estimate borrows the stand-in model's rate *for that
        // day*, not its rate today — otherwise an estimate would leak
        // today's price into a historical figure.
        assert_eq!(
            estimated_on("claude-sonnet-6", DURING_INTRO).input_per_million_usd,
            2.0
        );
        assert_eq!(
            estimated_on("claude-sonnet-6", AFTER_INTRO).input_per_million_usd,
            3.0
        );
    }

    #[test]
    fn mythos_falls_back_to_the_fable_tier() {
        // Mythos shares Fable's specs and pricing but has its own
        // family prefix, so it needs its own FAMILY_CURRENT entry.
        assert_eq!(
            estimated_on("claude-mythos-6", DURING_INTRO),
            exact_on("claude-fable-5", DURING_INTRO)
        );
    }

    #[test]
    fn a_retired_model_does_not_capture_its_family_fallback() {
        // `claude-opus-` spans the current tier and retired Opus 4.1.
        // An unlisted Opus must land on the current $5/$25 rate, not on
        // whichever tier happens to be declared first.
        let est = estimated_on("claude-opus-7", DURING_INTRO);
        assert_eq!(est.input_per_million_usd, 5.0);
    }

    #[test]
    fn unknown_family_returns_none() {
        for id in [
            "gpt-4",
            "",
            "unknown-thing",
            // Shaped like a Claude id but not a family we price.
            "claude-bogus-9",
            // No family segment at all — must not match a tier.
            "claude-opus",
        ] {
            assert!(rates_for(id).is_none(), "{id} should not resolve");
            assert!(resolve_rates_on(id, DURING_INTRO).is_none());
        }
    }

    // ── Cost estimation ────────────────────────────────────────────

    #[test]
    fn estimate_zero_tokens_is_zero_cost() {
        let c = estimate_cost_usd("claude-opus-4-7", 0, 0, 0, 0).unwrap();
        assert!((c - 0.0).abs() < 1e-9);
    }

    #[test]
    fn estimate_opus_million_in_million_out() {
        // 1M in at $5 + 1M out at $25 = $30 (current Opus tier).
        let c = estimate_cost_usd("claude-opus-4-8", 1_000_000, 1_000_000, 0, 0).unwrap();
        assert!((c - 30.0).abs() < 1e-6);
    }

    #[test]
    fn estimate_cache_read_is_dramatically_cheaper() {
        // 1M cache-read at $0.50 vs 1M input at $5 — 10× savings.
        let read = estimate_cost_usd("claude-opus-4-8", 0, 0, 1_000_000, 0).unwrap();
        let raw_in = estimate_cost_usd("claude-opus-4-8", 1_000_000, 0, 0, 0).unwrap();
        assert!(read * 10.0 > raw_in * 0.99 && read * 10.0 < raw_in * 1.01);
    }

    #[test]
    fn estimate_unknown_model_returns_none() {
        assert!(estimate_cost_usd("gpt-4", 1, 1, 0, 0).is_none());
    }

    #[test]
    fn estimate_is_additive_across_token_classes() {
        let split =
            estimate_cost_usd("claude-sonnet-4-6", 100_000, 50_000, 25_000, 10_000).unwrap();
        let sum = estimate_cost_usd("claude-sonnet-4-6", 100_000, 0, 0, 0).unwrap()
            + estimate_cost_usd("claude-sonnet-4-6", 0, 50_000, 0, 0).unwrap()
            + estimate_cost_usd("claude-sonnet-4-6", 0, 0, 25_000, 0).unwrap()
            + estimate_cost_usd("claude-sonnet-4-6", 0, 0, 0, 10_000).unwrap();
        assert!((split - sum).abs() < 1e-9);
    }
}
