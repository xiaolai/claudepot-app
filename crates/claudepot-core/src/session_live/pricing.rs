//! Model pricing — canonicalize CC-reported model ids and look up
//! per-million-token rates for cost estimation in the Activity
//! section.
//!
//! Baked table, not network-fetched. Per plan §11 decision 2, we
//! prefer predictable (bound at release time) to "accurate" (requires
//! an internet round-trip in a hot path). Unknown models return
//! `None`; the UI renders them as `—` with a tooltip, not `$0.00`.
//!
//! The canonicalizer strips CC's dated suffixes (e.g.
//! `claude-haiku-4-5-20251001` → `claude-haiku-4-5`) before lookup
//! so every release-series-compatible model id hits the same row.
//! Families not listed fall back to a prefix match on the
//! `claude-<family>-` portion; a truly unrecognized id returns
//! `None` cleanly.

use once_cell::sync::Lazy;
use regex::Regex;

/// Rates in US dollars per million tokens. Numbers match Anthropic's
/// published "standard" tier as of 2026-04; update on each release.
/// Cache-read rates are NOT yet in this table (M4 polish — the
/// `TokenCounters.cache_read_input_tokens` path is reported
/// separately on the backend and multiplied by a different rate).
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
    static ALIAS_SUFFIX_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"-(preview|latest|experimental)$").expect("static regex")
    });
    let lower = raw.to_ascii_lowercase();
    let no_date = DATE_SUFFIX_RE.replace(&lower, "").into_owned();
    ALIAS_SUFFIX_RE.replace(&no_date, "").into_owned()
}

/// Per-model rate lookup. Returns `None` for unknown ids so the UI
/// can render `—` instead of guessing.
pub fn rates_for(model: &str) -> Option<ModelRates> {
    let key = canonicalize_model_id(model);
    rates_by_canonical_id(&key).or_else(|| rates_by_family_prefix(&key))
}

/// Exact-match lookup against the baked table.
fn rates_by_canonical_id(id: &str) -> Option<ModelRates> {
    Some(match id {
        // Opus 4.7 (current generation)
        "claude-opus-4-7" => ModelRates {
            input_per_million_usd: 15.0,
            output_per_million_usd: 75.0,
            cache_read_per_million_usd: 1.5,
            cache_write_per_million_usd: 18.75,
        },
        // Sonnet 4.6 (current generation)
        "claude-sonnet-4-6" => ModelRates {
            input_per_million_usd: 3.0,
            output_per_million_usd: 15.0,
            cache_read_per_million_usd: 0.3,
            cache_write_per_million_usd: 3.75,
        },
        // Haiku 4.5 (current generation)
        "claude-haiku-4-5" => ModelRates {
            input_per_million_usd: 1.0,
            output_per_million_usd: 5.0,
            cache_read_per_million_usd: 0.1,
            cache_write_per_million_usd: 1.25,
        },
        _ => return None,
    })
}

/// Fallback: match by `claude-<family>-` prefix so a new point
/// release like `claude-opus-4-8` still gets a reasonable rate
/// (the last-seen rate for that family) before a release updates
/// the baked table.
fn rates_by_family_prefix(id: &str) -> Option<ModelRates> {
    if id.starts_with("claude-opus-") {
        return rates_by_canonical_id("claude-opus-4-7");
    }
    if id.starts_with("claude-sonnet-") {
        return rates_by_canonical_id("claude-sonnet-4-6");
    }
    if id.starts_with("claude-haiku-") {
        return rates_by_canonical_id("claude-haiku-4-5");
    }
    None
}

/// Compute estimated cost in USD for the given token counts. Returns
/// `None` when the model is unrecognized — callers should not invent
/// a number.
pub fn estimate_cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let r = rates_for(model)?;
    let million = 1_000_000.0;
    Some(
        (input_tokens as f64 / million) * r.input_per_million_usd
            + (output_tokens as f64 / million) * r.output_per_million_usd
            + (cache_read_tokens as f64 / million) * r.cache_read_per_million_usd
            + (cache_write_tokens as f64 / million) * r.cache_write_per_million_usd,
    )
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
        assert_eq!(
            canonicalize_model_id("Claude-Opus-4-7"),
            "claude-opus-4-7"
        );
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

    #[test]
    fn exact_rates_for_known_ids() {
        let opus = rates_for("claude-opus-4-7").unwrap();
        assert_eq!(opus.input_per_million_usd, 15.0);
        assert_eq!(opus.output_per_million_usd, 75.0);

        let son = rates_for("claude-sonnet-4-6").unwrap();
        assert_eq!(son.input_per_million_usd, 3.0);

        let hai = rates_for("claude-haiku-4-5").unwrap();
        assert_eq!(hai.input_per_million_usd, 1.0);
    }

    #[test]
    fn dated_model_id_resolves_to_family_rate() {
        let a = rates_for("claude-haiku-4-5-20251001").unwrap();
        let b = rates_for("claude-haiku-4-5").unwrap();
        assert_eq!(a, b, "dated id must canonicalize to undated");
    }

    // ── Prefix fallback ────────────────────────────────────────────

    #[test]
    fn future_point_release_falls_back_to_family() {
        // `claude-opus-4-8` isn't in the table yet; the family
        // prefix path should return the 4-7 rate until a release
        // updates it.
        let future = rates_for("claude-opus-4-8").unwrap();
        let known = rates_for("claude-opus-4-7").unwrap();
        assert_eq!(future, known);
    }

    #[test]
    fn unknown_family_returns_none() {
        assert!(rates_for("gpt-4").is_none());
        assert!(rates_for("").is_none());
        assert!(rates_for("unknown-thing").is_none());
    }

    // ── Cost estimation ────────────────────────────────────────────

    #[test]
    fn estimate_zero_tokens_is_zero_cost() {
        let c = estimate_cost_usd("claude-opus-4-7", 0, 0, 0, 0).unwrap();
        assert!((c - 0.0).abs() < 1e-9);
    }

    #[test]
    fn estimate_opus_million_in_million_out() {
        // 1M in at $15 + 1M out at $75 = $90.
        let c = estimate_cost_usd(
            "claude-opus-4-7",
            1_000_000,
            1_000_000,
            0,
            0,
        )
        .unwrap();
        assert!((c - 90.0).abs() < 1e-6);
    }

    #[test]
    fn estimate_cache_read_is_dramatically_cheaper() {
        // 1M cache-read at $1.50 vs 1M input at $15 — 10× savings.
        let read = estimate_cost_usd("claude-opus-4-7", 0, 0, 1_000_000, 0)
            .unwrap();
        let raw_in = estimate_cost_usd("claude-opus-4-7", 1_000_000, 0, 0, 0)
            .unwrap();
        assert!(read * 10.0 > raw_in * 0.99 && read * 10.0 < raw_in * 1.01);
    }

    #[test]
    fn estimate_unknown_model_returns_none() {
        assert!(estimate_cost_usd("gpt-4", 1, 1, 0, 0).is_none());
    }

    #[test]
    fn estimate_is_additive_across_token_classes() {
        let split = estimate_cost_usd(
            "claude-sonnet-4-6",
            100_000,
            50_000,
            25_000,
            10_000,
        )
        .unwrap();
        let sum = estimate_cost_usd("claude-sonnet-4-6", 100_000, 0, 0, 0)
            .unwrap()
            + estimate_cost_usd("claude-sonnet-4-6", 0, 50_000, 0, 0).unwrap()
            + estimate_cost_usd("claude-sonnet-4-6", 0, 0, 25_000, 0).unwrap()
            + estimate_cost_usd("claude-sonnet-4-6", 0, 0, 0, 10_000).unwrap();
        assert!((split - sum).abs() < 1e-9);
    }
}
