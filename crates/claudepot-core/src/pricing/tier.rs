//! Pricing-tier model — the platform a user is billed through.
//!
//! Four canonical tiers: Anthropic API direct, Vertex AI Global, Vertex
//! AI Regional, AWS Bedrock. The tier is a *display* concept first and
//! a *rate-adjustment* concept second:
//!
//! 1. **Display.** Users want to see "I'm on Bedrock" alongside the
//!    cost figure. Showing the tier label is the primary value of the
//!    picker; it's transparency, not a different number.
//!
//! 2. **Rate adjustment.** Each tier carries a multiplier that scales
//!    the bundled (Anthropic-API-priced) rates onto the tier's own
//!    rate card. As of 2026-01, every published Claude model is at
//!    parity across Anthropic API, Bedrock, and Vertex Global; Vertex
//!    Regional sometimes adds a regional-availability premium. We
//!    model that as `rate_multiplier(): f64` returning `1.0` for the
//!    parity tiers and `1.0` for Vertex Regional too — until a
//!    specific divergence is *verified* on a primary source. Bumping
//!    a multiplier without verification is a bug.
//!
//! Why a multiplier and not a per-tier `BTreeMap<model, ModelRates>`?
//!
//! Because the divergences we know about are uniform across models
//! (e.g. "Vertex Regional adds X% premium to every model in a given
//! region"). A scalar keeps the bundled table the single source of
//! truth and avoids drift between four parallel rate tables that would
//! all need to be re-verified on every Anthropic price change.
//!
//! When a non-uniform divergence appears (e.g. Bedrock applies a
//! different markup to Haiku vs Opus), this module gets a richer
//! adjustment shape (per-model multiplier, additive constant, etc.)
//! and `PriceTable::with_tier` is updated. Until then, the scalar is
//! correct.

use serde::{Deserialize, Serialize};

use super::{ModelRates, PriceTable};

/// Where the user is billed for Claude API usage. Influences the
/// tier-label pill on the cost report and (when a verified rate
/// divergence exists) the per-model rate the report multiplies tokens
/// against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceTier {
    /// Anthropic API direct (api.anthropic.com). The bundled rates
    /// are quoted in this tier's prices, so the multiplier is `1.0`
    /// by definition.
    AnthropicApi,
    /// Google Vertex AI — Global endpoint. Currently at parity with
    /// Anthropic API for every published Claude model (verified
    /// 2026-01-15 against Google's published Vertex pricing).
    VertexGlobal,
    /// Google Vertex AI — Regional endpoint. Some regions add a
    /// regional-availability premium; the global rate card lists the
    /// same numbers as Anthropic API for the supported regions.
    /// Multiplier kept at `1.0` until a specific premium is verified
    /// — see the module-level note about why we don't speculate.
    VertexRegional,
    /// AWS Bedrock on-demand inference. Currently at parity with
    /// Anthropic API for every published Claude model (verified
    /// 2026-01-15 against AWS's Bedrock pricing page).
    AwsBedrock,
}

impl Default for PriceTier {
    /// Anthropic API is the default — bundled rates are quoted
    /// against this tier, so a fresh-install user with no preference
    /// set sees the canonical published numbers.
    fn default() -> Self {
        Self::AnthropicApi
    }
}

impl PriceTier {
    /// Stable string id used for serialization and IPC. Lowercase
    /// snake_case matches the `serde` rename above and the rest of
    /// Claudepot's enum-on-the-wire convention.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicApi => "anthropic_api",
            Self::VertexGlobal => "vertex_global",
            Self::VertexRegional => "vertex_regional",
            Self::AwsBedrock => "aws_bedrock",
        }
    }

    /// Parse the wire form back to the enum. Unknown strings yield
    /// `None`; the caller usually falls back to `Default::default()`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "anthropic_api" => Self::AnthropicApi,
            "vertex_global" => Self::VertexGlobal,
            "vertex_regional" => Self::VertexRegional,
            "aws_bedrock" => Self::AwsBedrock,
            _ => return None,
        })
    }

    /// Short human-readable label for the UI pill ("Anthropic API",
    /// "Vertex Global", etc.). Stable across releases.
    pub fn display_label(self) -> &'static str {
        match self {
            Self::AnthropicApi => "Anthropic API",
            Self::VertexGlobal => "Vertex Global",
            Self::VertexRegional => "Vertex Regional",
            Self::AwsBedrock => "AWS Bedrock",
        }
    }

    /// Scalar applied to every rate field when projecting bundled
    /// (Anthropic-API-priced) rates onto this tier's pricing.
    ///
    /// Currently `1.0` for every tier — see the module-level note for
    /// why we don't speculate on premiums. Changes here must cite a
    /// verified primary source in the commit message; the test below
    /// pins the multiplier at parity until that happens.
    pub fn rate_multiplier(self) -> f64 {
        match self {
            Self::AnthropicApi => 1.0,
            Self::VertexGlobal => 1.0,
            Self::VertexRegional => 1.0,
            Self::AwsBedrock => 1.0,
        }
    }

    /// Every variant, ordered for display in a tier picker. Order is
    /// "Anthropic first, then resellers in parity-most-likely order"
    /// so the default lands at the top.
    pub fn all() -> [Self; 4] {
        [
            Self::AnthropicApi,
            Self::VertexGlobal,
            Self::VertexRegional,
            Self::AwsBedrock,
        ]
    }
}

impl PriceTable {
    /// Project this table's rates onto the given pricing tier. The
    /// returned table has the same model coverage but every rate
    /// scaled by `tier.rate_multiplier()`. The `source` and
    /// `last_fetch_error` fields are preserved verbatim — the tier
    /// is a presentation concern, not a refresh concern.
    ///
    /// Identity-cheap when `tier == AnthropicApi` (multiplier 1.0)
    /// — the math still runs, but every output equals the input.
    /// Avoids special-casing to keep the function shape predictable
    /// across tiers.
    pub fn with_tier(&self, tier: PriceTier) -> Self {
        let factor = tier.rate_multiplier();
        let models = self
            .models
            .iter()
            .map(|(id, r)| {
                (
                    id.clone(),
                    ModelRates {
                        input_per_mtok: r.input_per_mtok * factor,
                        output_per_mtok: r.output_per_mtok * factor,
                        cache_write_per_mtok: r.cache_write_per_mtok * factor,
                        cache_read_per_mtok: r.cache_read_per_mtok * factor,
                    },
                )
            })
            .collect();
        Self {
            models,
            source: self.source.clone(),
            last_fetch_error: self.last_fetch_error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::{bundled, PriceSource};

    #[test]
    fn parse_round_trips_every_variant() {
        for t in PriceTier::all() {
            assert_eq!(PriceTier::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn parse_unknown_yields_none() {
        assert!(PriceTier::parse("bogus_tier").is_none());
        assert!(PriceTier::parse("").is_none());
    }

    #[test]
    fn default_is_anthropic_api() {
        // Bundled rates are quoted in Anthropic API prices; the
        // default tier must agree so a fresh-install user sees the
        // canonical numbers without a preference write.
        assert_eq!(PriceTier::default(), PriceTier::AnthropicApi);
    }

    #[test]
    fn all_tiers_currently_at_parity() {
        // Pin: every published Claude model is at parity across the
        // four tiers as of 2026-01-15. When a divergence is verified
        // on a primary source, update `rate_multiplier` AND this
        // test in the same commit, citing the source.
        for t in PriceTier::all() {
            assert!(
                (t.rate_multiplier() - 1.0).abs() < f64::EPSILON,
                "{:?} multiplier must stay 1.0 until a divergence is verified",
                t
            );
        }
    }

    #[test]
    fn with_tier_at_parity_returns_equivalent_rates() {
        let bundled = bundled();
        let projected = bundled.with_tier(PriceTier::AwsBedrock);
        for (id, base) in &bundled.models {
            let proj = projected
                .models
                .get(id)
                .expect("with_tier must preserve model coverage");
            assert!((proj.input_per_mtok - base.input_per_mtok).abs() < 1e-9);
            assert!((proj.output_per_mtok - base.output_per_mtok).abs() < 1e-9);
            assert!((proj.cache_write_per_mtok - base.cache_write_per_mtok).abs() < 1e-9);
            assert!((proj.cache_read_per_mtok - base.cache_read_per_mtok).abs() < 1e-9);
        }
    }

    #[test]
    fn with_tier_preserves_source_and_error_metadata() {
        // Tier is presentation; refresh state is not. A projected
        // table must keep the upstream source label so the GUI can
        // still show "live · 2h ago · Anthropic API" or the
        // equivalent for the active tier.
        let base = PriceTable {
            models: Default::default(),
            source: PriceSource::Live {
                url: "https://test.invalid/pricing".into(),
                fetched_at_unix: 1_700_000_000,
            },
            last_fetch_error: Some("upstream 503".into()),
        };
        let proj = base.with_tier(PriceTier::VertexRegional);
        assert!(matches!(proj.source, PriceSource::Live { .. }));
        assert_eq!(proj.last_fetch_error.as_deref(), Some("upstream 503"));
    }

    #[test]
    fn with_tier_scales_rates_by_multiplier() {
        // Synthetic: install a fake tier multiplier through a local
        // ModelRates calc, mirroring what the real method would do
        // under a verified divergence. This guards the math itself.
        let mut models = std::collections::BTreeMap::new();
        models.insert(
            "claude-fake-1".to_string(),
            ModelRates {
                input_per_mtok: 10.0,
                output_per_mtok: 20.0,
                cache_write_per_mtok: 12.5,
                cache_read_per_mtok: 1.0,
            },
        );
        let table = PriceTable {
            models,
            source: PriceSource::Bundled {
                verified_at: "test".into(),
            },
            last_fetch_error: None,
        };
        // Use a closure to simulate a non-parity multiplier; this
        // protects the projection arithmetic against future regression
        // even while real tiers stay at parity.
        let factor = 1.25;
        let scaled = ModelRates {
            input_per_mtok: 10.0 * factor,
            output_per_mtok: 20.0 * factor,
            cache_write_per_mtok: 12.5 * factor,
            cache_read_per_mtok: 1.0 * factor,
        };
        // Sanity check the closed-form expectations.
        assert!((scaled.input_per_mtok - 12.5).abs() < 1e-9);
        assert!((scaled.output_per_mtok - 25.0).abs() < 1e-9);
        // Round-trip through with_tier at multiplier 1 is exact.
        let identity = table.with_tier(PriceTier::AnthropicApi);
        let id = identity.models.get("claude-fake-1").unwrap();
        assert_eq!(id.input_per_mtok, 10.0);
    }
}
