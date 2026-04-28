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

pub mod service;
pub use service::{Fetcher, LiveFetcher, PricingCacheService};

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
const RATES_VERIFIED_AT: &str = "2026-01-15";

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
    for id in ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"] {
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

/// Match a model identifier from an event to an entry in the table.
/// Handles two known degenerate cases:
///   - Exact id match (`claude-opus-4-7`).
///   - Date-stamped ids (`claude-opus-4-7-20260101`) — strip the
///     trailing date and retry.
/// Returns `None` for unknown ids; the UI should render "rate
/// unknown" rather than silently substituting a different model's
/// rate. That way rate drift surfaces instead of lying.
pub fn resolve_model_rates<'a>(table: &'a PriceTable, model_id: &str) -> Option<&'a ModelRates> {
    if let Some(r) = table.models.get(model_id) {
        return Some(r);
    }
    // Strip a trailing `-YYYYMMDD` (8 digits after the last dash) if
    // present. Anthropic stamps model snapshots this way.
    if let Some((stem, tail)) = model_id.rsplit_once('-') {
        if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) {
            return table.models.get(stem);
        }
    }
    None
}

fn cache_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(CACHE_FILENAME)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
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
    let bytes = serde_json::to_vec_pretty(table).map_err(|e| std::io::Error::other(e))?;
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
pub async fn fetch_live() -> Result<PriceTable, String> {
    let body = reqwest::get(ANTHROPIC_PRICING_URL)
        .await
        .map_err(|e| format!("fetch: {e}"))?
        .text()
        .await
        .map_err(|e| format!("read body: {e}"))?;

    let models = parse_pricing_html(&body).map_err(|e| format!("parse: {e}"))?;
    // Require at least one model from each frontier family. Previous
    // check hardcoded exact ids (`claude-opus-4-7` etc.) which would
    // fail forever once Anthropic ships the next point release —
    // family-level presence is the stable invariant to assert.
    for family in ["claude-opus-", "claude-sonnet-", "claude-haiku-"] {
        if !models.keys().any(|id| id.starts_with(family)) {
            return Err(format!("no {family}* model found in scrape"));
        }
    }
    Ok(PriceTable {
        models,
        source: PriceSource::Live {
            url: ANTHROPIC_PRICING_URL.to_string(),
            fetched_at_unix: now_unix_secs(),
        },
        last_fetch_error: None,
    })
}

/// Very forgiving HTML parse. Looks for repeated patterns of
/// `claude-<family>-<major>-<minor>` near `$X.XX` / MTok markers and
/// pairs them up. Doesn't require a specific DOM structure; survives
/// reasonable page restructures as long as the text is still present.
fn parse_pricing_html(html: &str) -> Result<BTreeMap<String, ModelRates>, String> {
    // Normalize whitespace so a multi-line cell collapses into
    // searchable text.
    let flat: String = html.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = BTreeMap::new();
    for family_prefix in ["claude-opus-4-", "claude-sonnet-4-", "claude-haiku-4-"] {
        if let Some(rates) = scrape_family(&flat, family_prefix) {
            out.insert(rates.0, rates.1);
        }
    }
    Ok(out)
}

/// Pull the latest version of a family from a flattened HTML blob.
/// Returns `(canonical_id, rates)` when both the id and its two
/// rates can be found, otherwise None. Heuristic; deliberately
/// minimal to resist page churn.
///
/// When the page lists several versions of a family (e.g.
/// `claude-opus-4-6` is still mentioned alongside `claude-opus-4-7`
/// in historical tables), we scan every occurrence and pick the
/// one with the highest minor version, then extract rates from that
/// model's window.
fn scrape_family(flat: &str, prefix: &str) -> Option<(String, ModelRates)> {
    // Collect every id + offset pair that starts with the family
    // prefix followed by an integer version. (offset, id, version).
    let mut candidates: Vec<(usize, String, u32)> = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = flat[search_from..].find(prefix) {
        let start = search_from + rel;
        let tail = &flat[start..];
        let mut end = prefix.len();
        for ch in tail[prefix.len()..].chars() {
            if ch.is_ascii_digit() {
                end += ch.len_utf8();
            } else {
                break;
            }
        }
        if end > prefix.len() {
            let id = tail[..end].to_string();
            if let Ok(version) = id[prefix.len()..].parse::<u32>() {
                candidates.push((start, id, version));
            }
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

/// Kick the background refresh. Caller is responsible for awaiting
/// on the handle only if they care about the outcome; the dashboard
/// should fire-and-forget so app start isn't blocked by a DNS hang.
pub async fn refresh_now() -> Result<PriceTable, String> {
    let fresh = fetch_live().await?;
    if let Err(e) = write_cache(&fresh) {
        // Cache-write failure is non-fatal — the in-memory table is
        // still usable, we just won't persist until next refresh.
        tracing::warn!("pricing cache write failed: {e}");
    }
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_has_frontier_models() {
        let t = bundled();
        assert!(t.models.contains_key("claude-opus-4-7"));
        assert!(t.models.contains_key("claude-sonnet-4-6"));
        assert!(t.models.contains_key("claude-haiku-4-5"));
    }

    #[test]
    fn bundled_cache_math_follows_formula() {
        let t = bundled();
        let opus = t.models.get("claude-opus-4-7").unwrap();
        // Cache write = 1.25 × input; cache read = 0.10 × input.
        assert!((opus.cache_write_per_mtok - opus.input_per_mtok * 1.25).abs() < 1e-9);
        assert!((opus.cache_read_per_mtok - opus.input_per_mtok * 0.10).abs() < 1e-9);
    }

    #[test]
    fn resolve_exact_id_hit() {
        let t = bundled();
        let r = resolve_model_rates(&t, "claude-opus-4-7");
        assert!(r.is_some());
    }

    #[test]
    fn resolve_strips_date_suffix() {
        let t = bundled();
        let r = resolve_model_rates(&t, "claude-opus-4-7-20260407");
        assert!(r.is_some());
    }

    #[test]
    fn resolve_unknown_model_returns_none() {
        let t = bundled();
        // No hallucinated rate for a model we've never seen.
        assert!(resolve_model_rates(&t, "claude-bogus-9-99").is_none());
    }

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
        let hit = scrape_family(html, "claude-opus-4-");
        let (id, rates) = hit.unwrap();
        assert_eq!(id, "claude-opus-4-7");
        assert_eq!(rates.input_per_mtok, 15.0);
        assert_eq!(rates.output_per_mtok, 75.0);
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
}
