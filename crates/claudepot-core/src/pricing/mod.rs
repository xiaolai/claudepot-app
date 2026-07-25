//! Anthropic API pricing table — bundled defaults, a 24-hour file
//! cache, and an opportunistic scraper of Anthropic's public pricing
//! docs for daily refreshes.
//!
//! Why a table in the app at all?
//!
//! Subscription users already pay a flat monthly fee for Pro / Max /
//! Team. The number that moves them is *"what pay-per-call would have
//! cost me"* — seeing that figure grow each day is the emotional
//! payload of being on a subscription. The table only needs to stay
//! accurate enough to make that comparison credible.
//!
//! # Freshness strategy
//!
//! 1. **Bundled defaults** — rates hardcoded at build time with a
//!    `RATES_VERIFIED_AT` date. Always available, even with no
//!    network. If the scraper fails or is never reached, the app
//!    still shows costs, marked `source: Bundled`.
//!
//! 2. **Cache file** — `$CLAUDEPOT_DATA_DIR/pricing-cache.json`
//!    holds the last successful fetch. Read at app start and used if
//!    less than [`CACHE_TTL_HOURS`] old.
//!
//! 3. **Opportunistic refresh** — on app start, if the cache is
//!    stale, a background task fetches Anthropic's pricing page,
//!    parses the model table, writes the cache, and returns. Never
//!    blocks the UI; if the fetch fails (network offline, page
//!    restructured), we fall through to bundled defaults.
//!
//! # Stability
//!
//! The scraper is best-effort. Anthropic's marketing pages change
//! shape. When parsing fails we surface that explicitly in the
//! returned table (`source: Bundled`, `last_fetch_error: Some(...)`)
//! so callers can show a "rates may be stale" indicator rather than
//! silently displaying old numbers.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod book;
pub mod history;
pub mod service;
mod tier;

pub use book::{PriceBook, PricedCost};
pub use service::{Fetcher, LiveFetcher, PricingCacheService};
pub use tier::PriceTier;

/// USD per *million tokens*. Kept in an "easy to eyeball" unit so
/// table edits don't drown in trailing zeros. Multiplication by
/// actual token counts happens one layer up.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_write_per_mtok: f64,
    pub cache_read_per_mtok: f64,
}

/// Where the in-memory rates came from. Surfaced to the UI so we can
/// label the figure ("Rates as of 2026-04-24 · from anthropic.com").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PriceSource {
    /// Hardcoded at build time.
    Bundled { verified_at: String },
    /// Live scrape of an Anthropic-controlled URL.
    Live { url: String, fetched_at_unix: u64 },
    /// Cache file, older than memory but younger than bundled.
    Cached {
        fetched_at_unix: u64,
        source_url: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceTable {
    /// Keyed by model identifier (e.g. `claude-opus-4-7`). Callers
    /// should pass the same id CC stamps into each event's
    /// `usage.model` field; see `resolve_model_rates` for the
    /// aliasing rules when versions don't match exactly.
    pub models: BTreeMap<String, ModelRates>,
    pub source: PriceSource,
    /// If the most recent refresh attempt failed, a short user-safe
    /// message explaining why. Never contains a stack trace — this
    /// string is OK to render in a tooltip.
    pub last_fetch_error: Option<String>,
}

/// Bundled rates verified against Anthropic's public pricing page on
/// this date. Bumped whenever the defaults are edited.
///
/// This also acts as a cache-freshness floor (see
/// [`rates_verified_at_unix`]), so bumping it is what makes a shipped
/// rate correction reach users who already hold a cache file. Editing
/// [`crate::session_live::pricing::RATE_TIERS`] without bumping this
/// leaves those users on the old numbers for up to a day.
const RATES_VERIFIED_AT: &str = "2026-07-25";

/// Cache TTL — how old a cached table can be before we trigger a
/// background refresh. Anthropic rate changes are infrequent and
/// announced; 24h is plenty responsive for a display figure.
pub const CACHE_TTL_HOURS: u64 = 24;

/// Filename inside `claudepot_data_dir()` where the last-good fetch
/// is mirrored. Plain JSON so a user can inspect / edit if needed.
const CACHE_FILENAME: &str = "pricing-cache.json";

/// Returns a PriceTable using the bundled defaults. Always succeeds;
/// used as the final fallback when cache + scraper both miss.
///
/// Rate values are sourced from `session_live::pricing` so this
/// module and the hot-path lookups used inside the live watcher
/// can't drift apart. Edits to the bundled table happen in that
/// module's match arms; this function just re-exposes them as a
/// generic `PriceTable` keyed for dashboard consumption.
pub fn bundled() -> PriceTable {
    use crate::session_live::pricing as sl;
    let mut models = BTreeMap::new();
    // The model list comes from `session_live::pricing` rather than
    // being restated here. It used to be a second hardcoded list, and
    // the two drifted: when Opus 5 shipped it was added to the rate
    // table but not to this list, so `resolve_model_rates` reported
    // every Opus 5 session as "unpriced" in the cost dashboard while
    // the Activity strip priced the same session correctly.
    for id in sl::priced_model_ids() {
        let Some(r) = sl::rates_for(id) else { continue };
        models.insert(
            id.to_string(),
            ModelRates {
                input_per_mtok: r.input_per_million_usd,
                output_per_mtok: r.output_per_million_usd,
                cache_write_per_mtok: r.cache_write_per_million_usd,
                cache_read_per_mtok: r.cache_read_per_million_usd,
            },
        );
    }
    PriceTable {
        models,
        source: PriceSource::Bundled {
            verified_at: RATES_VERIFIED_AT.to_string(),
        },
        last_fetch_error: None,
    }
}

// A `resolve_model_rates(&PriceTable, id)` used to live here: a flat
// current-rates lookup with its own date-suffix stripping. It was the
// second of three rate resolvers, and the one that answered
// differently from the others — no observed history, no family
// estimate, no notion of *when* the usage happened — which is how the
// Activity strip and the cost dashboard came to disagree about the
// same session. `PriceBook::resolve` is now the only resolver;
// `session_live::pricing::canonicalize_model_id` owns the suffix
// stripping. Do not reintroduce a shortcut past the book.

fn cache_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(CACHE_FILENAME)
}

/// Today in UTC, re-exported so pricing callers don't have to reach
/// into `session_live` for a calendar day.
pub use crate::session_live::pricing::today_utc;

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `RATES_VERIFIED_AT` as a unix timestamp (midnight UTC). Used as the
/// freshness floor for cached tables: a cache fetched *before* the
/// running build verified its bundled rates predates our current price
/// knowledge and must not shadow it. Falls back to `0` (no floor) if
/// the constant ever fails to parse — a malformed date should degrade
/// to "trust the cache", never panic or reject every cache.
fn rates_verified_at_unix() -> u64 {
    chrono::NaiveDate::parse_from_str(RATES_VERIFIED_AT, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp().max(0) as u64)
        .unwrap_or(0)
}

/// Read the on-disk cache. Returns `None` when the file is missing,
/// unreadable, corrupt, or older than the TTL. Parse errors on the
/// cache are intentionally silent — a bad cache is equivalent to no
/// cache, and bundled defaults cover.
pub fn load_cached() -> Option<PriceTable> {
    let path = cache_path();
    let bytes = std::fs::read(&path).ok()?;
    let table: PriceTable = serde_json::from_slice(&bytes).ok()?;
    let fetched_at = match &table.source {
        PriceSource::Cached {
            fetched_at_unix, ..
        } => *fetched_at_unix,
        PriceSource::Live {
            fetched_at_unix, ..
        } => *fetched_at_unix,
        // Bundled-sourced cache files are meaningless (nothing to
        // restore); treat as miss.
        PriceSource::Bundled { .. } => return None,
    };
    // Freshness floor: a cache fetched before this build verified its
    // bundled rates predates our current price knowledge. Discard it so
    // a shipped rate correction (e.g. the Opus $15→$5 fix) wins the
    // instant a user updates, instead of waiting out the 24h TTL or a
    // manual refresh. A cache fetched *after* the floor is a real live
    // scrape newer than our bundled snapshot — keep trusting it.
    if fetched_at < rates_verified_at_unix() {
        return None;
    }
    let age_secs = now_unix_secs().saturating_sub(fetched_at);
    if age_secs > CACHE_TTL_HOURS * 3600 {
        return None;
    }
    // Re-tag as Cached so callers don't mistake a file-loaded table
    // for a fresh live fetch.
    let url = match &table.source {
        PriceSource::Live { url, .. } => url.clone(),
        PriceSource::Cached { source_url, .. } => source_url.clone(),
        _ => String::new(),
    };
    Some(PriceTable {
        models: table.models,
        source: PriceSource::Cached {
            fetched_at_unix: fetched_at,
            source_url: url,
        },
        last_fetch_error: None,
    })
}

fn write_cache(table: &PriceTable) -> std::io::Result<()> {
    let dir = crate::paths::claudepot_data_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(CACHE_FILENAME);
    let bytes = serde_json::to_vec_pretty(table).map_err(std::io::Error::other)?;
    std::fs::write(path, bytes)
}

/// URL we scrape when refreshing. Kept as an internal constant so
/// tests can point at a fixture server without exposing it to every
/// caller.
const ANTHROPIC_PRICING_URL: &str = "https://www.anthropic.com/pricing";

/// Fetch + parse Anthropic's pricing page. Best-effort: on any
/// failure (network, HTML shape drift, missing known models), returns
/// `Err(short message)`. Callers treat errors as "keep using the
/// previous table, annotate it as stale".
///
/// Parsing strategy: the page renders each model as a row with a
/// predictable text shape — `"Claude <Model> <version?> ... Input
/// $X.XX / MTok ... Output $Y.YY / MTok ...`. We extract the model
/// id and the two base rates via a forgiving regex, then *derive*
/// cache-write / cache-read from Anthropic's fixed formulas
/// (input × 1.25 and input × 0.1). That derivation is authoritative
/// per Anthropic's own cache documentation and sidesteps brittle
/// parsing of the secondary rows.
/// Overlay scraped rates onto the bundled table as a floor: `bundled()`
/// provides an entry for every current model, and each scraped rate
/// overrides its bundled counterpart. The result therefore covers at
/// least the bundled model set — a partial or stale scrape can only
/// refresh/extend coverage, never drop a known model to "unpriced".
fn overlay_scrape_on_bundled(
    scraped: BTreeMap<String, ModelRates>,
) -> BTreeMap<String, ModelRates> {
    let mut models = bundled().models;
    for (id, rate) in scraped {
        models.insert(id, rate);
    }
    models
}

pub async fn fetch_live() -> Result<PriceTable, String> {
    let body = reqwest::get(ANTHROPIC_PRICING_URL)
        .await
        .map_err(|e| format!("fetch: {e}"))?
        .text()
        .await
        .map_err(|e| format!("read body: {e}"))?;

    let scraped = parse_pricing_html(&body).map_err(|e| format!("parse: {e}"))?;
    // A parse that found nothing means the page restructured past what
    // the heuristic scraper can read. Surface it as an error so the
    // caller keeps the existing cache / bundled table rather than
    // caching an empty "live" result.
    if scraped.is_empty() {
        return Err("scrape parsed no models (pricing page shape changed?)".to_string());
    }
    // Overlay the scrape onto the bundled table as a floor. The scraper
    // is best-effort — it reads one model per family (the highest
    // version it can find) and depends on marketing-page text staying
    // recognizable, so a raw scrape is routinely missing models the
    // bundled table knows. Starting from `bundled()` and letting scraped
    // rates override guarantees a live fetch can only *refresh or
    // extend* coverage, never regress a known model to "unpriced".
    let models = overlay_scrape_on_bundled(scraped);
    Ok(PriceTable {
        models,
        source: PriceSource::Live {
            url: ANTHROPIC_PRICING_URL.to_string(),
            fetched_at_unix: now_unix_secs(),
        },
        last_fetch_error: None,
    })
}

/// Every family the scraper looks for, as a bare `claude-<family>-`
/// prefix. The version segments are parsed by [`parse_family_id`],
/// which handles both id shapes Anthropic has shipped:
/// `claude-opus-4-8` (major-minor) and `claude-opus-5` (major only,
/// the dateless form introduced with the 4.6 generation).
///
/// These were once written as `claude-opus-4-` — the major version was
/// baked into the prefix. That made the whole 5 generation invisible to
/// the scraper, which by 2026-07 was three of the four current models.
const SCRAPED_FAMILIES: &[&str] = &[
    "claude-opus-",
    "claude-sonnet-",
    "claude-haiku-",
    "claude-fable-",
];

/// Very forgiving HTML parse. Looks for repeated patterns of
/// `claude-<family>-<version>` near `$X.XX` / MTok markers and
/// pairs them up. Doesn't require a specific DOM structure; survives
/// reasonable page restructures as long as the text is still present.
fn parse_pricing_html(html: &str) -> Result<BTreeMap<String, ModelRates>, String> {
    // Normalize whitespace so a multi-line cell collapses into
    // searchable text.
    let flat: String = html.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = BTreeMap::new();
    for family_prefix in SCRAPED_FAMILIES {
        if let Some(rates) = scrape_family(&flat, family_prefix) {
            out.insert(rates.0, rates.1);
        }
    }
    Ok(out)
}

/// Parse a model id off the head of `tail`, which is known to start
/// with `prefix`. Returns the canonical id and its `(major, minor)`
/// version for ordering.
///
/// Accepts `claude-opus-5` → `(5, 0)` and `claude-opus-4-8` → `(4, 8)`,
/// so the two id generations sort against each other correctly. A
/// trailing `-YYYYMMDD` snapshot stamp is excluded from the id — the
/// price table is keyed by the undated form — and is never mistaken
/// for a minor version, since minor versions are at most 7 digits.
fn parse_family_id(tail: &str, prefix: &str) -> Option<(String, (u32, u32))> {
    let after_prefix = tail.strip_prefix(prefix)?;
    let major_digits = leading_digits(after_prefix);
    if major_digits.is_empty() {
        return None;
    }
    let major: u32 = major_digits.parse().ok()?;
    let mut end = prefix.len() + major_digits.len();
    let mut minor = 0u32;
    if let Some(after_dash) = after_prefix[major_digits.len()..].strip_prefix('-') {
        let minor_digits = leading_digits(after_dash);
        if !minor_digits.is_empty() && minor_digits.len() < 8 {
            minor = minor_digits.parse().ok()?;
            end += 1 + minor_digits.len();
        }
    }
    Some((tail[..end].to_string(), (major, minor)))
}

fn leading_digits(s: &str) -> &str {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    &s[..end]
}

/// Pull the latest version of a family from a flattened HTML blob.
/// Returns `(canonical_id, rates)` when both the id and its two
/// rates can be found, otherwise None. Heuristic; deliberately
/// minimal to resist page churn.
///
/// When the page lists several versions of a family (e.g.
/// `claude-opus-4-8` is still mentioned alongside `claude-opus-5`
/// in historical tables), we scan every occurrence and pick the
/// highest version, then extract rates from that model's window.
fn scrape_family(flat: &str, prefix: &str) -> Option<(String, ModelRates)> {
    // Collect every id + offset pair that starts with the family
    // prefix followed by a version. (offset, id, (major, minor)).
    let mut candidates: Vec<(usize, String, (u32, u32))> = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = flat[search_from..].find(prefix) {
        let start = search_from + rel;
        if let Some((id, version)) = parse_family_id(&flat[start..], prefix) {
            candidates.push((start, id, version));
        }
        search_from = start + prefix.len();
    }
    if candidates.is_empty() {
        return None;
    }
    // Pick the highest version; on a tie take the latest occurrence
    // (pages tend to list newest first but hedge against the opposite).
    candidates.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
    let (start, model_id, _) = candidates.into_iter().last()?;

    // Within a 2000-char window after the id, find "$<input>"
    // followed later by "$<output>". Rates are dollars per MTok.
    let tail = &flat[start..];
    let window = &tail[..tail.len().min(2000)];
    let (input, output) = extract_two_dollar_rates(window)?;
    let rates = ModelRates {
        input_per_mtok: input,
        output_per_mtok: output,
        cache_write_per_mtok: input * 1.25,
        cache_read_per_mtok: input * 0.10,
    };
    Some((model_id, rates))
}

fn extract_two_dollar_rates(s: &str) -> Option<(f64, f64)> {
    let mut hits = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            if j > i + 1 {
                if let Ok(n) = s[i + 1..j].parse::<f64>() {
                    hits.push(n);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    if hits.len() < 2 {
        return None;
    }
    Some((hits[0], hits[1]))
}

/// Top-level resolver. Always returns a usable table. The rules are
/// a cascade: fresh cache → bundled (and a background refresh is
/// kicked off by the caller, not here — this function is sync).
pub fn load() -> PriceTable {
    if let Some(cached) = load_cached() {
        return cached;
    }
    bundled()
}

// A free `refresh_now()` used to live here. It had no callers — every
// refresh goes through `PricingCacheService::refresh_now`, which
// singleflights concurrent callers and records observed rate changes
// into `pricing-history.json`. Keeping a second entry point that did
// neither meant the next caller to reach for it would silently skip
// history recording, so it was removed rather than patched.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_has_frontier_models() {
        let t = bundled();
        assert!(t.models.contains_key("claude-opus-5"));
        assert!(t.models.contains_key("claude-sonnet-5"));
        assert!(t.models.contains_key("claude-fable-5"));
        assert!(t.models.contains_key("claude-opus-4-7"));
        assert!(t.models.contains_key("claude-sonnet-4-6"));
        assert!(t.models.contains_key("claude-haiku-4-5"));
    }

    #[test]
    fn bundled_prices_every_model_the_rate_table_knows() {
        // The regression that motivated deriving this list: a model
        // priced by `session_live::pricing` but missing here resolved
        // to "unpriced" in the cost dashboard while the Activity strip
        // showed a cost for the same session.
        let t = bundled();
        for id in crate::session_live::pricing::priced_model_ids() {
            assert!(
                t.models.contains_key(id),
                "{id} is priced by the rate table but missing from the dashboard table"
            );
        }
    }

    #[test]
    fn bundled_cache_math_follows_formula() {
        let t = bundled();
        let opus = t.models.get("claude-opus-5").unwrap();
        // Cache write = 1.25 × input; cache read = 0.10 × input.
        assert!((opus.cache_write_per_mtok - opus.input_per_mtok * 1.25).abs() < 1e-9);
        assert!((opus.cache_read_per_mtok - opus.input_per_mtok * 0.10).abs() < 1e-9);
    }

    // Exact-hit / date-suffix / unknown-model resolution used to be
    // tested here against a second resolver. That resolver is gone;
    // `pricing::book` owns resolution and covers all three through the
    // shared cross-language vectors.

    #[test]
    fn extract_two_rates_simple() {
        let (a, b) = extract_two_dollar_rates("Input $15.00 Output $75.00").unwrap();
        assert_eq!(a, 15.0);
        assert_eq!(b, 75.0);
    }

    #[test]
    fn extract_two_rates_skips_nondollars() {
        let (a, b) = extract_two_dollar_rates("junk 42 more $3.00 words $15.00 trail").unwrap();
        assert_eq!(a, 3.0);
        assert_eq!(b, 15.0);
    }

    #[test]
    fn scrape_family_finds_model_and_rates() {
        let html = "noise claude-opus-4-7 marketing Input $15.00 / MTok Output $75.00 / MTok tail";
        let hit = scrape_family(html, "claude-opus-");
        let (id, rates) = hit.unwrap();
        assert_eq!(id, "claude-opus-4-7");
        assert_eq!(rates.input_per_mtok, 15.0);
        assert_eq!(rates.output_per_mtok, 75.0);
    }

    #[test]
    fn scrape_family_reads_the_dateless_id_shape() {
        // `claude-opus-5` — major version only. The old scraper baked
        // the major version into the prefix (`claude-opus-4-`) and so
        // couldn't see this generation at all.
        let html = "noise claude-opus-5 marketing Input $5.00 / MTok Output $25.00 / MTok tail";
        let (id, rates) = scrape_family(html, "claude-opus-").unwrap();
        assert_eq!(id, "claude-opus-5");
        assert_eq!(rates.input_per_mtok, 5.0);
        assert_eq!(rates.output_per_mtok, 25.0);
    }

    #[test]
    fn scrape_family_prefers_the_newer_generation_over_a_higher_minor() {
        // A page listing both generations must yield Opus 5, not
        // Opus 4.8 — (5, 0) outranks (4, 8) as a version tuple, which
        // a plain integer comparison on the trailing digits would get
        // backwards.
        let html = "claude-opus-4-8 Input $5.00 / MTok Output $25.00 / MTok \
                    then claude-opus-5 Input $6.00 / MTok Output $30.00 / MTok";
        let (id, rates) = scrape_family(html, "claude-opus-").unwrap();
        assert_eq!(id, "claude-opus-5");
        assert_eq!(rates.input_per_mtok, 6.0);
    }

    #[test]
    fn scrape_family_ignores_snapshot_date_stamps() {
        // `-20251001` is a snapshot stamp, not a minor version. It must
        // stay out of the id (the table is keyed undated) and must not
        // be read as version 20_251_001.
        let html = "claude-haiku-4-5-20251001 Input $1.00 / MTok Output $5.00 / MTok";
        let (id, _) = scrape_family(html, "claude-haiku-").unwrap();
        assert_eq!(id, "claude-haiku-4-5");
    }

    #[test]
    fn scrape_family_skips_non_version_matches() {
        // A family prefix followed by a word rather than a version is
        // not a model id and must not be scraped as one.
        let html = "claude-opus-latest Input $5.00 / MTok Output $25.00 / MTok";
        assert!(scrape_family(html, "claude-opus-").is_none());
    }

    #[test]
    fn parse_pricing_html_covers_the_current_lineup() {
        // Every family in SCRAPED_FAMILIES must be reachable — the
        // scraper going quiet is invisible in production because the
        // bundled table silently covers for it.
        let html = "claude-opus-5 $5.00 / MTok $25.00 / MTok \
                    claude-sonnet-5 $3.00 / MTok $15.00 / MTok \
                    claude-haiku-4-5 $1.00 / MTok $5.00 / MTok \
                    claude-fable-5 $10.00 / MTok $50.00 / MTok";
        let out = parse_pricing_html(html).unwrap();
        assert_eq!(out.len(), SCRAPED_FAMILIES.len());
        assert_eq!(out.get("claude-opus-5").unwrap().input_per_mtok, 5.0);
        assert_eq!(out.get("claude-sonnet-5").unwrap().output_per_mtok, 15.0);
        assert_eq!(out.get("claude-haiku-4-5").unwrap().input_per_mtok, 1.0);
        assert_eq!(out.get("claude-fable-5").unwrap().output_per_mtok, 50.0);
    }

    #[test]
    fn load_returns_bundled_when_no_cache() {
        // Direct-call the bundled path; tests don't mess with the
        // real data dir. This asserts the minimum contract.
        let t = bundled();
        assert!(matches!(t.source, PriceSource::Bundled { .. }));
    }

    #[test]
    fn cache_ttl_default_matches_constant() {
        // Guards against an accidental edit of the TTL; dashboard
        // copy references "daily" — keep this at 24 unless you also
        // update the UI.
        assert_eq!(CACHE_TTL_HOURS, 24);
    }

    // ── Freshness floor (RATES_VERIFIED_AT) ────────────────────────

    #[test]
    fn cache_older_than_rates_floor_is_rejected() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        // A cache stamped one second before this build verified its
        // bundled rates predates our price knowledge. Even if it were
        // otherwise TTL-fresh it must be discarded, so a shipped rate
        // correction wins the instant a user updates.
        let stale = PriceTable {
            models: bundled().models,
            source: PriceSource::Live {
                url: "https://example.test/pricing".to_string(),
                fetched_at_unix: rates_verified_at_unix().saturating_sub(1),
            },
            last_fetch_error: None,
        };
        write_cache(&stale).unwrap();
        assert!(load_cached().is_none(), "pre-floor cache must be rejected");
    }

    #[test]
    fn cache_newer_than_rates_floor_is_kept() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        // A freshly-fetched cache (now ≥ the floor, age within TTL) is a
        // real live scrape newer than the bundled snapshot — the floor
        // must not over-reject it.
        let fresh = PriceTable {
            models: bundled().models,
            source: PriceSource::Live {
                url: "https://example.test/pricing".to_string(),
                fetched_at_unix: now_unix_secs(),
            },
            last_fetch_error: None,
        };
        write_cache(&fresh).unwrap();
        let loaded = load_cached();
        assert!(loaded.is_some(), "fresh post-floor cache must be kept");
        assert!(matches!(loaded.unwrap().source, PriceSource::Cached { .. }));
    }

    // ── Scrape overlay-on-bundled floor ────────────────────────────

    #[test]
    fn overlay_partial_scrape_keeps_bundled_floor() {
        // A scrape that yielded only one model must not shrink coverage:
        // every bundled model survives, and the scraped rate overrides.
        let mut scraped = BTreeMap::new();
        scraped.insert(
            "claude-opus-4-8".to_string(),
            ModelRates {
                input_per_mtok: 999.0,
                output_per_mtok: 999.0,
                cache_write_per_mtok: 0.0,
                cache_read_per_mtok: 0.0,
            },
        );
        let merged = overlay_scrape_on_bundled(scraped);
        // Bundled models the scrape never mentioned are still present —
        // no regression to "unpriced" for current models.
        assert!(merged.contains_key("claude-sonnet-5"));
        assert!(merged.contains_key("claude-fable-5"));
        assert!(merged.contains_key("claude-haiku-4-5"));
        // The scraped rate wins for the model it covered.
        assert_eq!(merged.get("claude-opus-4-8").unwrap().input_per_mtok, 999.0);
    }

    #[test]
    fn overlay_empty_scrape_equals_bundled() {
        // A parse that found nothing collapses to exactly the bundled
        // table (fetch_live rejects an empty scrape before this, but the
        // floor property should hold regardless).
        let merged = overlay_scrape_on_bundled(BTreeMap::new());
        assert_eq!(merged, bundled().models);
    }
}
