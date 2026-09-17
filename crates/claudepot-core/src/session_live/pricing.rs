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
//! and [`resolve_rates_on`] takes the day the usage happened, so a
//! price change re-scores only the usage after it. Scoring everything
//! at today's rate silently rewrites the past every time Anthropic
//! changes a price. No bundled model has more than one period today —
//! Sonnet 5's announced 2026-09-01 increase was cancelled (see its
//! entry) — but observed changes from [`crate::pricing::history`]
//! land as later periods, and those are resolved the same way.
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
/// published "standard" tier. See [`RATE_TIERS`] for the values and the
/// date they were verified.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelRates {
    pub input_per_million_usd: f64,
    pub output_per_million_usd: f64,
    pub cache_read_per_million_usd: f64,
    /// Five-minute cache writes (1.25× input).
    pub cache_write_per_million_usd: f64,
    /// One-hour cache writes (2× input).
    pub cache_write_1h_per_million_usd: f64,
}

/// A server-side web search, per request, for every model (CC 2.1.274's
/// `webSearchRequests: 0.01` in every tier).
pub const WEB_SEARCH_USD_PER_REQUEST: f64 = 0.01;

/// US-only inference (`usage.inference_geo == "us"`) bills token cost at
/// 1.1× — CC's `Wfe`, and Anthropic's data-residency pricing. Web
/// searches are not multiplied.
pub const US_GEO_MULTIPLIER: f64 = 1.1;

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

/// Cache rates for `model` at an input rate of `input_per_million_usd`,
/// as `(five-minute write, one-hour write, read)`.
///
/// For sources that see only input and output — the live pricing
/// scrape, and the history observations it writes. The multipliers
/// are a property of the model, not a constant: Fable 5.1 and Mythos
/// 5.1 read cache at 0.025× input where the rest read at 0.1×, and a
/// fixed 0.1× turned a scrape of Fable 5.1's unchanged $10 input into
/// a recorded "change" to a $1 cache read. So a listed model is scaled
/// from its own bundled entry, and only an unlisted one gets
/// Anthropic's standard 1.25× / 0.1×.
///
/// Scaled as `bundled × (input / bundled_input)` rather than through a
/// ratio, so an unchanged input returns the bundled figures
/// bit-for-bit — history dedup compares with `==`.
pub fn derived_cache_rates(model: &str, input_per_million_usd: f64) -> (f64, f64, f64) {
    let input = input_per_million_usd;
    let key = canonicalize_model_id(model);
    let listed = periods_for_id(&key)
        .and_then(|p| p.last())
        .map(|p| p.rates)
        .filter(|r| r.input_per_million_usd > 0.0);
    match listed {
        Some(r) => {
            let scale = input / r.input_per_million_usd;
            (
                r.cache_write_per_million_usd * scale,
                r.cache_write_1h_per_million_usd * scale,
                r.cache_read_per_million_usd * scale,
            )
        }
        None => (input * 1.25, input * 2.0, input * 0.10),
    }
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
/// group the moment one member's price diverges.
///
/// Periods are listed oldest-first and must stay that way;
/// `periods_are_sorted_oldest_first` enforces it.
///
/// Rates verified against Anthropic's published model pricing
/// (platform.claude.com/docs/en/about-claude/pricing) and Claude Code
/// 2.1.274's own `pricing_tiers` table on 2026-09-17. USD per million
/// tokens; `cache_write` is the 5-minute write (1.25× input), and
/// `cache_read` is 0.1× input **except** on Fable 5.1 and Mythos 5.1,
/// where it is 0.025×.
///
/// Claude 3.x ids (`claude-3-5-haiku`, `claude-3-7-sonnet`, …) are
/// deliberately absent. Their `claude-<generation>-<family>` naming
/// puts no family where [`FAMILY_CURRENT`] looks for one, so listing
/// them would need a fake `claude-3-` family; unlisted, they render
/// `—`, which is honest for models retired from the first-party API.
///
/// Fast mode is priced from [`FAST_RATE_TIERS`]: transcripts have
/// carried `usage.speed` since the field was added, which closed what
/// this note used to call a known gap.
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
                cache_write_1h_per_million_usd: 10.0,
            },
        }],
    ),
    // Opus 4.1 / Opus 4 — the retired $15 / $75 tier. Transcripts
    // from their era are still on disk, and scoring them at the
    // current Opus rate understates them 3×. `claude-opus-4` is what
    // `claude-opus-4-20250514` canonicalizes to; `claude-opus-4-0` is
    // the alias Claude Code keys its own table on.
    (
        &["claude-opus-4-1", "claude-opus-4", "claude-opus-4-0"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 15.0,
                output_per_million_usd: 75.0,
                cache_read_per_million_usd: 1.5,
                cache_write_per_million_usd: 18.75,
                cache_write_1h_per_million_usd: 30.0,
            },
        }],
    ),
    // Sonnet 5 — $2 / $10, flat. It launched calling that an
    // introductory price with an increase to $3 / $15 on 2026-09-01;
    // Anthropic made $2 / $10 the standard price instead, and the
    // pricing page says the increase "will not occur". This table
    // carried the cancelled period for a while, over-reporting every
    // Sonnet 5 session after that date by half.
    // `sonnet_5_keeps_its_price_past_the_cancelled_increase` holds it.
    (
        &["claude-sonnet-5"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 2.0,
                output_per_million_usd: 10.0,
                cache_read_per_million_usd: 0.2,
                cache_write_per_million_usd: 2.5,
                cache_write_1h_per_million_usd: 4.0,
            },
        }],
    ),
    // Sonnet 4.6 / 4.5 / 4 — $3 / $15 throughout. `claude-sonnet-4`
    // and `claude-sonnet-4-0` as for Opus 4 above.
    (
        &[
            "claude-sonnet-4-6",
            "claude-sonnet-4-5",
            "claude-sonnet-4",
            "claude-sonnet-4-0",
        ],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 3.0,
                output_per_million_usd: 15.0,
                cache_read_per_million_usd: 0.3,
                cache_write_per_million_usd: 3.75,
                cache_write_1h_per_million_usd: 6.0,
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
                cache_write_1h_per_million_usd: 20.0,
            },
        }],
    ),
    // Fable 5.1 / Mythos 5.1 — the same $10 / $50, but cache reads at
    // $0.25 (0.025× input, not 0.1×). A group of its own because only
    // the cache-read rate differs, which is exactly the difference an
    // input-only comparison cannot see: before this entry, 5.1 fell
    // back to Fable 5 as a family estimate and every cache read — the
    // bulk of a long session's tokens — was scored at 4× its price.
    (
        &["claude-fable-5-1", "claude-mythos-5-1"],
        &[RatePeriod {
            starts: None,
            rates: ModelRates {
                input_per_million_usd: 10.0,
                output_per_million_usd: 50.0,
                cache_read_per_million_usd: 0.25,
                cache_write_per_million_usd: 12.5,
                cache_write_1h_per_million_usd: 20.0,
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
                cache_write_1h_per_million_usd: 2.0,
            },
        }],
    ),
];

/// Fast-mode rates (`usage.speed == "fast"`), from CC 2.1.274's cost
/// function: Opus 5 and 4.8 bill at 2× their standard rates, Opus 4.7
/// and 4.6 at 6× — the research-preview price those two carried while
/// fast mode ran on them. Cache multipliers apply on top, as they do at
/// the standard rate. A model not listed here bills fast-mode tokens at
/// its standard rate, which is what CC does too.
const FAST_RATE_TIERS: &[(&[&str], ModelRates)] = &[
    (
        &["claude-opus-5", "claude-opus-4-8"],
        ModelRates {
            input_per_million_usd: 10.0,
            output_per_million_usd: 50.0,
            cache_read_per_million_usd: 1.0,
            cache_write_per_million_usd: 12.5,
            cache_write_1h_per_million_usd: 20.0,
        },
    ),
    (
        &["claude-opus-4-7", "claude-opus-4-6"],
        ModelRates {
            input_per_million_usd: 30.0,
            output_per_million_usd: 150.0,
            cache_read_per_million_usd: 3.0,
            cache_write_per_million_usd: 37.5,
            cache_write_1h_per_million_usd: 60.0,
        },
    ),
];

/// Fast-mode rates for a canonical model id, if it has its own.
pub fn fast_rates_for(canonical_id: &str) -> Option<ModelRates> {
    FAST_RATE_TIERS
        .iter()
        .find(|(ids, _)| ids.contains(&canonical_id))
        .map(|(_, r)| *r)
}

/// Every model with its own fast-mode rates, for transport.
pub fn fast_rate_entries() -> impl Iterator<Item = (&'static str, ModelRates)> {
    FAST_RATE_TIERS
        .iter()
        .flat_map(|(ids, r)| ids.iter().map(move |id| (*id, *r)))
}

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
    // Fable 5.1 has been the default Fable model since Claude Code
    // 2.1.257, so an unlisted Fable is priced like it.
    ("claude-fable-", "claude-fable-5-1"),
    // Mythos tracks Fable's pricing generation for generation, and has
    // its own family prefix.
    ("claude-mythos-", "claude-mythos-5-1"),
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

/// Token cost of `u` at `r` — the one place the per-token arithmetic
/// lives, so every surface weights the token classes identically.
///
/// One-hour cache writes bill at their own rate and the rest of the
/// writes at the five-minute rate, with the one-hour count capped at
/// the write total (CC's `jfe`). Web searches are not tokens; see
/// [`crate::pricing::PriceBook::cost`].
pub fn price_tokens(r: &ModelRates, u: &crate::session::TokenUsage) -> f64 {
    let million = 1_000_000.0;
    let write_1h = u.cache_creation_1h.min(u.cache_creation);
    let write_5m = u.cache_creation - write_1h;
    (u.input as f64 / million) * r.input_per_million_usd
        + (u.output as f64 / million) * r.output_per_million_usd
        + (u.cache_read as f64 / million) * r.cache_read_per_million_usd
        + (write_5m as f64 / million) * r.cache_write_per_million_usd
        + (write_1h as f64 / million) * r.cache_write_1h_per_million_usd
}

/// Compute estimated cost in USD at today's rates. Returns `None` when
/// the model belongs to no family we price — callers should not invent
/// a number. Historical usage must go through
/// [`crate::pricing::PriceBook`] with its own date instead.
pub fn estimate_cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let r = rates_for(model)?;
    Some(price_tokens(
        &r,
        &crate::session::TokenUsage {
            input: input_tokens,
            output: output_tokens,
            cache_read: cache_read_tokens,
            cache_creation: cache_write_tokens,
            ..Default::default()
        },
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

    /// An ordinary day to resolve flat rates on.
    const DAY: Ymd = (2026, 7, 25);
    /// The day Sonnet 5's cancelled increase was scheduled to start.
    const CANCELLED_INCREASE: Ymd = (2026, 9, 1);

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
        let opus = exact_on("claude-opus-5", DAY);
        assert_eq!(opus.input_per_million_usd, 5.0);
        assert_eq!(opus.output_per_million_usd, 25.0);
        for id in [
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
        ] {
            assert_eq!(exact_on(id, DAY), opus);
        }

        let son = exact_on("claude-sonnet-4-6", DAY);
        assert_eq!(son.input_per_million_usd, 3.0);
        assert_eq!(son.output_per_million_usd, 15.0);
        for id in ["claude-sonnet-4-5", "claude-sonnet-4", "claude-sonnet-4-0"] {
            assert_eq!(exact_on(id, DAY), son);
        }

        let fable = exact_on("claude-fable-5", DAY);
        assert_eq!(fable.input_per_million_usd, 10.0);
        assert_eq!(fable.output_per_million_usd, 50.0);
        assert_eq!(fable.cache_read_per_million_usd, 1.0);
        assert_eq!(exact_on("claude-mythos-5", DAY), fable);

        let hai = exact_on("claude-haiku-4-5", DAY);
        assert_eq!(hai.input_per_million_usd, 1.0);
        assert_eq!(hai.output_per_million_usd, 5.0);
    }

    #[test]
    fn fable_5_1_reads_cache_at_a_quarter_dollar() {
        // Same input and output as Fable 5, so only the cache-read
        // rate tells them apart — and it is the rate a long session
        // spends most of its tokens on.
        let fable_5_1 = exact_on("claude-fable-5-1", DAY);
        assert_eq!(fable_5_1.input_per_million_usd, 10.0);
        assert_eq!(fable_5_1.output_per_million_usd, 50.0);
        assert_eq!(fable_5_1.cache_read_per_million_usd, 0.25);
        assert_eq!(fable_5_1.cache_write_per_million_usd, 12.5);
        assert_eq!(exact_on("claude-mythos-5-1", DAY), fable_5_1);
        assert_ne!(fable_5_1, exact_on("claude-fable-5", DAY));
    }

    #[test]
    fn derived_cache_rates_follow_each_models_own_multipliers() {
        // An unchanged input reproduces the bundled figures exactly —
        // not approximately — for every listed model, because history
        // dedup compares with `==`.
        for id in priced_model_ids() {
            let r = exact_on(id, DAY);
            assert_eq!(
                derived_cache_rates(id, r.input_per_million_usd),
                (
                    r.cache_write_per_million_usd,
                    r.cache_write_1h_per_million_usd,
                    r.cache_read_per_million_usd,
                ),
                "{id}"
            );
        }
        // Fable 5.1 keeps its quarter-rate reads when its input moves.
        assert_eq!(
            derived_cache_rates("claude-fable-5-1", 20.0),
            (25.0, 40.0, 0.5)
        );
        // An unlisted model gets the standard multipliers.
        assert_eq!(
            derived_cache_rates("claude-fable-9", 10.0),
            (12.5, 20.0, 1.0)
        );
    }

    #[test]
    fn retired_opus_keeps_its_own_higher_rate() {
        // Opus 4.1 and Opus 4 billed at $15 / $75. Folding them into
        // the current Opus tier would understate every transcript from
        // their era 3×.
        let old = exact_on("claude-opus-4-1", DAY);
        assert_eq!(old.input_per_million_usd, 15.0);
        assert_eq!(old.output_per_million_usd, 75.0);
        let current = exact_on("claude-opus-5", DAY);
        assert_ne!(old, current);
        // The API's dated id for Opus 4 canonicalizes onto this tier
        // rather than falling through to the current Opus estimate.
        assert_eq!(exact_on("claude-opus-4-20250514", DAY), old);
        assert_eq!(exact_on("claude-opus-4-0", DAY), old);
    }

    #[test]
    fn dated_model_id_resolves_to_family_rate() {
        let a = rates_for("claude-haiku-4-5-20251001").unwrap();
        let b = rates_for("claude-haiku-4-5").unwrap();
        assert_eq!(a, b, "dated id must canonicalize to undated");
    }

    // ── Price history ──────────────────────────────────────────────

    #[test]
    fn sonnet_5_keeps_its_price_past_the_cancelled_increase() {
        // Launched as "introductory $2/$10 through 2026-08-31", then
        // made the standard price; the $3/$15 increase never happened.
        // The table carried it anyway and scored every later Sonnet 5
        // session at 1.5× — this pins the day before, the day of, and
        // well after.
        let before = exact_on("claude-sonnet-5", (2026, 8, 31));
        assert_eq!(before.input_per_million_usd, 2.0);
        assert_eq!(before.output_per_million_usd, 10.0);
        assert_eq!(exact_on("claude-sonnet-5", CANCELLED_INCREASE), before);
        assert_eq!(exact_on("claude-sonnet-5", (2027, 3, 14)), before);
    }

    /// Two periods with a change on 2026-10-01, shaped like an
    /// observed rate change merged over a bundled opening period.
    fn dated_periods() -> [RatePeriod; 2] {
        let at = |input: f64| ModelRates {
            cache_write_1h_per_million_usd: (input) * 2.0,
            input_per_million_usd: input,
            output_per_million_usd: input * 5.0,
            cache_read_per_million_usd: input / 10.0,
            cache_write_per_million_usd: input * 1.25,
        };
        [
            RatePeriod {
                starts: None,
                rates: at(5.0),
            },
            RatePeriod {
                starts: Some((2026, 10, 1)),
                rates: at(7.0),
            },
        ]
    }

    #[test]
    fn the_period_boundary_is_the_first_day_of_the_new_rate() {
        // 2026-09-30 is the last old day; 2026-10-01 is the first new
        // one. An off-by-one here misprices a whole day of usage.
        let periods = dated_periods();
        let input_on = |on| rate_on(&periods, on).unwrap().input_per_million_usd;
        assert_eq!(input_on((2026, 9, 30)), 5.0);
        assert_eq!(input_on((2026, 10, 1)), 7.0);
        // ...and every day after, not just the boundary day.
        assert_eq!(input_on((2027, 3, 14)), 7.0);
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
                cache_write_1h_per_million_usd: 2.0,
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
        let future = estimated_on("claude-opus-4-9", DAY);
        assert_eq!(future, exact_on("claude-opus-5", DAY));
    }

    #[test]
    fn future_generation_falls_back_to_family() {
        // The dateless `claude-<family>-<N>` shape (Opus 5's shape) has
        // to fall back too — this is what broke when Opus 5 shipped and
        // only `claude-opus-4-*` ids were listed.
        assert_eq!(
            estimated_on("claude-opus-6", DAY),
            exact_on("claude-opus-5", DAY)
        );
        assert_eq!(
            estimated_on("claude-sonnet-6", DAY),
            exact_on("claude-sonnet-5", DAY)
        );
    }

    #[test]
    fn fable_and_mythos_estimate_from_their_current_generation() {
        // The current Fable is 5.1, whose cache reads cost a quarter of
        // Fable 5's. Estimating an unlisted Fable from Fable 5 would
        // put the old cache rate on the newest model.
        assert_eq!(
            estimated_on("claude-fable-6", DAY),
            exact_on("claude-fable-5-1", DAY)
        );
        // Mythos has its own family prefix, so it needs its own
        // FAMILY_CURRENT entry.
        assert_eq!(
            estimated_on("claude-mythos-6", DAY),
            exact_on("claude-mythos-5-1", DAY)
        );
    }

    #[test]
    fn a_retired_model_does_not_capture_its_family_fallback() {
        // `claude-opus-` spans the current tier and retired Opus 4.1.
        // An unlisted Opus must land on the current $5/$25 rate, not on
        // whichever tier happens to be declared first.
        let est = estimated_on("claude-opus-7", DAY);
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
            assert!(resolve_rates_on(id, DAY).is_none());
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
