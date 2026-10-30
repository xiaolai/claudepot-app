//! Curated gateway presets: providers that speak Anthropic's Messages
//! API, so a gateway route pointed at them works with no translation.
//!
//! The data is `data/gateway-presets.json`, shared with the route form
//! (`src/sections/third-party/RouteForm.tsx` imports the same file). It
//! comes from the `anthropic`-protocol rows of clai's `providers.json`,
//! and every row carries the date its endpoint was last checked against
//! the vendor's docs — a preset is vendor data and drifts like any other.
//!
//! The earlier presets pointed at OpenAI-compatible `…/v1` roots. A
//! gateway route sets `ANTHROPIC_BASE_URL`, and Claude Code then calls
//! `<base>/v1/messages`, so those routes could never answer. A preset
//! belongs here only if its endpoint speaks the Messages API.
//!
//! clai's two `anthropic` rows with no fixed endpoint (`local`,
//! `local-deepseek`) are user-supplied URLs; the blank form covers them.
//!
//! Compiled only under `cfg(test)`: no Rust code consumes the presets
//! yet, so the parser exists to vet the shared file's shape. Make it
//! public again when a Rust consumer appears.

use serde::Deserialize;

const RAW: &str = include_str!("../../data/gateway-presets.json");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GatewayPreset {
    pub id: String,
    /// Vendor brand name; `None` when the visible label is localized
    /// copy ("Ollama (local)").
    pub label: Option<String>,
    pub base_url: String,
    /// Empty for local servers, whose model is whatever the user pulled.
    pub model: String,
    pub docs: String,
    /// `YYYY-MM-DD` the endpoint was last checked against `docs`.
    pub checked: String,
    pub reachable_from_blocked_regions: bool,
}

#[derive(Deserialize)]
struct File {
    schema_version: u32,
    presets: Vec<GatewayPreset>,
}

#[derive(Debug, thiserror::Error)]
#[error("gateway-presets.json is malformed: {0}")]
pub struct PresetsError(String);

/// The presets, in display order.
pub fn gateway_presets() -> Result<Vec<GatewayPreset>, PresetsError> {
    let f: File = serde_json::from_str(RAW).map_err(|e| PresetsError(e.to_string()))?;
    if f.schema_version != 1 {
        return Err(PresetsError(format!(
            "unknown schema_version {}",
            f.schema_version
        )));
    }
    Ok(f.presets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::{
        normalize_gateway_base_url, AuthScheme, GatewayConfig, Route, RouteProvider,
    };

    fn route_from(p: &GatewayPreset) -> Route {
        let model = if p.model.is_empty() {
            "llama3.2:3b"
        } else {
            &p.model
        };
        Route {
            id: uuid::Uuid::new_v4(),
            name: p.id.clone(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: normalize_gateway_base_url(&p.base_url).unwrap(),
                api_key: "sk-test".into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain: false,
            }),
            model: model.into(),
            small_fast_model: None,
            additional_models: Vec::new(),
            wrapper_name: format!("claude-{}", p.id),
            deployment_organization_uuid: uuid::Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
            is_private_cloud: false,
            capabilities_override: None,
        }
    }

    #[test]
    fn presets_parse_and_ids_are_unique() {
        let ps = gateway_presets().unwrap();
        assert_eq!(ps.len(), 6);
        let mut ids: Vec<_> = ps.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ps.len());
    }

    #[test]
    fn every_preset_survives_save_unchanged() {
        // Saving runs the gateway normalizer; a preset it rewrote would
        // show the user one URL and store another.
        for p in gateway_presets().unwrap() {
            assert_eq!(
                normalize_gateway_base_url(&p.base_url).unwrap(),
                p.base_url,
                "{}",
                p.id
            );
        }
    }

    #[test]
    fn no_preset_points_at_an_openai_compatible_root() {
        for p in gateway_presets().unwrap() {
            assert!(
                !p.base_url.ends_with("/v1") && !p.base_url.contains("/chat/completions"),
                "{} is not a Messages API endpoint",
                p.id
            );
        }
    }

    #[test]
    fn remote_presets_are_https_and_local_ones_loopback() {
        for p in gateway_presets().unwrap() {
            let local = p.label.is_none();
            if local {
                assert!(p.base_url.starts_with("http://127.0.0.1:"), "{}", p.id);
                assert!(p.model.is_empty(), "{}", p.id);
            } else {
                assert!(p.base_url.starts_with("https://"), "{}", p.id);
                assert!(!p.model.is_empty(), "{}", p.id);
            }
        }
    }

    #[test]
    fn every_preset_is_dated_and_cites_its_docs() {
        for p in gateway_presets().unwrap() {
            assert!(
                chrono::NaiveDate::parse_from_str(&p.checked, "%Y-%m-%d").is_ok(),
                "{}",
                p.id
            );
            assert!(p.docs.starts_with("https://"), "{}", p.id);
        }
    }

    #[test]
    fn every_preset_makes_a_wrapper_that_targets_its_endpoint() {
        for p in gateway_presets().unwrap() {
            let s = crate::routes::wrapper::render_script(&route_from(&p));
            assert!(
                s.contains(&format!("ANTHROPIC_BASE_URL='{}'", p.base_url)),
                "{}",
                p.id
            );
            assert!(s.contains("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST='1'"));
        }
    }
}
