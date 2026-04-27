//! Claude Desktop's `Claude-3p/` writer. Two surfaces:
//!
//! 1. `configLibrary/<uuid>.json` — multi-profile registry. One file
//!    per defined route. Idempotent overwrite.
//! 2. `claude_desktop_config.json` — top-level. Mirrors the *active*
//!    profile's keys into the `enterpriseConfig` block, plus
//!    optional `disableDeploymentModeChooser`. Preserves
//!    `_cfprefsMigrated` and any unknown fields from the existing
//!    file.
//!
//! macOS path: `~/Library/Application Support/Claude-3p/`. Other
//! platforms: feature-gated in phase-3 follow-up.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fs_utils;

use super::error::RouteError;
use super::helper::helper_path;
use super::keychain::SecretField;
use super::types::{Route, RouteProvider};
use super::CLAUDEPOT_MANAGED_MARKER;

/// `~/Library/Application Support/Claude-3p/`.
pub fn data_dir() -> Result<PathBuf, RouteError> {
    let home = dirs::home_dir().ok_or(RouteError::NoHomeDir)?;
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("Claude-3p"))
}

/// `~/Library/Application Support/Claude-3p/configLibrary/`.
pub fn library_dir() -> Result<PathBuf, RouteError> {
    Ok(data_dir()?.join("configLibrary"))
}

/// `~/Library/Application Support/Claude-3p/claude_desktop_config.json`.
pub fn enterprise_config_path() -> Result<PathBuf, RouteError> {
    Ok(data_dir()?.join("claude_desktop_config.json"))
}

/// One profile entry under `configLibrary/`. Field names match the
/// shape `lonr-6/cc-desktop-switch` and `dypaul/...-Gateway-Setup`
/// already write; Claude Desktop's UI consumes them directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryProfile {
    pub uuid: String,
    pub name: String,
    /// `inferenceProvider` value: "gateway" / "bedrock" / "vertex" /
    /// "foundry".
    pub provider: String,
    /// Provider-specific config block (the `inference<Provider>*`
    /// keys, e.g. `inferenceGatewayBaseUrl`).
    pub keys: Map<String, Value>,
    /// `inferenceModels` for the picker.
    pub models: Vec<String>,
    /// Marker so future Claudepot runs know we own this entry.
    #[serde(rename = "claudepot_managed")]
    pub claudepot_managed: bool,
}

/// Write (or overwrite) one library profile derived from a Route.
/// Returns the absolute path written.
pub fn write_library_profile(route: &Route) -> Result<PathBuf, RouteError> {
    let dir = library_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", route.id));

    let profile = build_library_profile(route);
    let bytes = serde_json::to_vec_pretty(&profile)?;
    fs_utils::atomic_write(&path, &bytes)?;
    Ok(path)
}

/// Activate a route on Desktop: write the library profile and
/// mirror the same keys into `enterpriseConfig` of the top-level
/// `claude_desktop_config.json`. Other fields (e.g.
/// `_cfprefsMigrated`, future Anthropic additions) are preserved.
pub fn activate_desktop(
    route: &Route,
    disable_chooser: bool,
) -> Result<PathBuf, RouteError> {
    write_library_profile(route)?;
    let path = enterprise_config_path()?;
    let mut top = read_top_level(&path)?;

    let enterprise = build_enterprise_config(route, disable_chooser);
    top.insert(
        "enterpriseConfig".to_string(),
        Value::Object(enterprise),
    );

    let bytes = serde_json::to_vec_pretty(&Value::Object(top))?;
    fs_utils::atomic_write(&path, &bytes)?;
    Ok(path)
}

/// Reset the top-level `enterpriseConfig` to `{}`. Library entries
/// are kept intact — `clear` is "no route is active right now," not
/// "delete everything."
pub fn clear_desktop_active() -> Result<PathBuf, RouteError> {
    let path = enterprise_config_path()?;
    let mut top = read_top_level(&path)?;
    top.insert(
        "enterpriseConfig".to_string(),
        Value::Object(Map::new()),
    );
    let bytes = serde_json::to_vec_pretty(&Value::Object(top))?;
    fs_utils::atomic_write(&path, &bytes)?;
    Ok(path)
}

fn read_top_level(path: &Path) -> Result<Map<String, Value>, RouteError> {
    if !path.exists() {
        let mut m = Map::new();
        // Preserve the marker Claude Desktop writes itself.
        m.insert("_cfprefsMigrated".to_string(), Value::Bool(true));
        return Ok(m);
    }
    let raw = std::fs::read(path)?;
    if raw.is_empty() {
        let mut m = Map::new();
        m.insert("_cfprefsMigrated".to_string(), Value::Bool(true));
        return Ok(m);
    }
    let v: Value = serde_json::from_slice(&raw)?;
    match v {
        Value::Object(m) => Ok(m),
        _ => Ok(Map::new()),
    }
}

fn build_library_profile(route: &Route) -> LibraryProfile {
    let keys = build_inference_keys(route);
    let mut models = vec![route.model.clone()];
    for m in &route.additional_models {
        if !models.contains(m) {
            models.push(m.clone());
        }
    }
    LibraryProfile {
        uuid: route.id.to_string(),
        name: route.name.clone(),
        provider: route.provider.kind().as_str().to_string(),
        keys,
        models,
        claudepot_managed: true,
    }
}

fn build_enterprise_config(
    route: &Route,
    disable_chooser: bool,
) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(
        "inferenceProvider".to_string(),
        Value::String(route.provider.kind().as_str().to_string()),
    );
    let keys = build_inference_keys(route);
    for (k, v) in keys {
        m.insert(k, v);
    }
    let mut models = vec![Value::String(route.model.clone())];
    for extra in &route.additional_models {
        if !models.iter().any(|v| v.as_str() == Some(extra)) {
            models.push(Value::String(extra.clone()));
        }
    }
    m.insert("inferenceModels".to_string(), Value::Array(models));
    m.insert(
        "deploymentOrganizationUuid".to_string(),
        Value::String(route.deployment_organization_uuid.to_string()),
    );
    m.insert(
        "disableDeploymentModeChooser".to_string(),
        Value::Bool(disable_chooser),
    );
    m.insert(
        CLAUDEPOT_MANAGED_MARKER.to_string(),
        Value::Bool(true),
    );
    m
}

fn build_inference_keys(route: &Route) -> Map<String, Value> {
    let mut m = Map::new();
    match &route.provider {
        RouteProvider::Gateway(cfg) => {
            m.insert(
                "inferenceGatewayBaseUrl".to_string(),
                Value::String(cfg.base_url.clone()),
            );
            m.insert(
                "inferenceGatewayAuthScheme".to_string(),
                Value::String(cfg.auth_scheme.as_str().to_string()),
            );
            if cfg.use_keychain {
                m.insert(
                    "inferenceCredentialHelper".to_string(),
                    Value::String(
                        helper_path(route.id, SecretField::GatewayApiKey)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                );
            } else {
                m.insert(
                    "inferenceGatewayApiKey".to_string(),
                    Value::String(cfg.api_key.clone()),
                );
            }
        }
        RouteProvider::Bedrock(cfg) => {
            m.insert(
                "inferenceBedrockRegion".to_string(),
                Value::String(cfg.region.clone()),
            );
            if cfg.use_keychain {
                m.insert(
                    "inferenceCredentialHelper".to_string(),
                    Value::String(
                        helper_path(route.id, SecretField::BedrockBearerToken)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                );
            } else if let Some(t) = &cfg.bearer_token {
                m.insert(
                    "inferenceBedrockBearerToken".to_string(),
                    Value::String(t.clone()),
                );
            }
            if let Some(u) = &cfg.base_url {
                m.insert(
                    "inferenceBedrockBaseUrl".to_string(),
                    Value::String(u.clone()),
                );
            }
            if let Some(p) = &cfg.aws_profile {
                m.insert(
                    "inferenceBedrockProfile".to_string(),
                    Value::String(p.clone()),
                );
            }
        }
        RouteProvider::Vertex(cfg) => {
            m.insert(
                "inferenceVertexProjectId".to_string(),
                Value::String(cfg.project_id.clone()),
            );
            if let Some(r) = &cfg.region {
                m.insert(
                    "inferenceVertexRegion".to_string(),
                    Value::String(r.clone()),
                );
            }
            if let Some(u) = &cfg.base_url {
                m.insert(
                    "inferenceVertexBaseUrl".to_string(),
                    Value::String(u.clone()),
                );
            }
        }
        RouteProvider::Foundry(cfg) => {
            if cfg.use_keychain {
                m.insert(
                    "inferenceCredentialHelper".to_string(),
                    Value::String(
                        helper_path(route.id, SecretField::FoundryApiKey)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                );
            } else if let Some(k) = &cfg.api_key {
                m.insert(
                    "inferenceFoundryApiKey".to_string(),
                    Value::String(k.clone()),
                );
            }
            if let Some(u) = &cfg.base_url {
                m.insert(
                    "inferenceFoundryBaseUrl".to_string(),
                    Value::String(u.clone()),
                );
            }
            if let Some(r) = &cfg.resource {
                m.insert(
                    "inferenceFoundryResource".to_string(),
                    Value::String(r.clone()),
                );
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::types::{
        AuthScheme, BedrockConfig, FoundryConfig, GatewayConfig, RouteProvider,
        VertexConfig,
    };
    use uuid::Uuid;

    fn sample() -> Route {
        Route {
            id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            name: "Local Ollama".into(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: "http://127.0.0.1:11434".into(),
                api_key: "ollama".into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain: false,
            }),
            model: "llama3.2:3b".into(),
            small_fast_model: None,
            additional_models: vec!["qwen2.5:7b".into()],
            wrapper_name: "claude-llama3-2-3b".into(),
            deployment_organization_uuid: Uuid::parse_str(
                "22222222-2222-2222-2222-222222222222",
            )
            .unwrap(),
            active_on_desktop: false,
            installed_on_cli: false,
        }
    }

    #[test]
    fn library_profile_shape() {
        let p = build_library_profile(&sample());
        assert_eq!(p.uuid, "11111111-1111-1111-1111-111111111111");
        assert_eq!(p.name, "Local Ollama");
        assert_eq!(p.provider, "gateway");
        assert_eq!(p.models, vec!["llama3.2:3b", "qwen2.5:7b"]);
        assert!(p.claudepot_managed);
        assert_eq!(
            p.keys.get("inferenceGatewayBaseUrl"),
            Some(&Value::String("http://127.0.0.1:11434".into()))
        );
        assert_eq!(
            p.keys.get("inferenceGatewayApiKey"),
            Some(&Value::String("ollama".into()))
        );
        assert_eq!(
            p.keys.get("inferenceGatewayAuthScheme"),
            Some(&Value::String("bearer".into()))
        );
    }

    #[test]
    fn library_profile_dedups_model() {
        let mut r = sample();
        r.additional_models = vec!["llama3.2:3b".into(), "qwen:7b".into()];
        let p = build_library_profile(&r);
        // Primary "llama3.2:3b" appears once even though additional_models
        // also mentions it.
        let count = p.models.iter().filter(|m| *m == "llama3.2:3b").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn enterprise_config_carries_provider_and_models() {
        let m = build_enterprise_config(&sample(), false);
        assert_eq!(
            m.get("inferenceProvider"),
            Some(&Value::String("gateway".into()))
        );
        assert!(m.get("inferenceModels").unwrap().as_array().is_some());
        assert_eq!(
            m.get("disableDeploymentModeChooser"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            m.get("claudepot_managed"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            m.get("deploymentOrganizationUuid"),
            Some(&Value::String(
                "22222222-2222-2222-2222-222222222222".into()
            ))
        );
    }

    #[test]
    fn enterprise_config_chooser_flag() {
        let on = build_enterprise_config(&sample(), true);
        assert_eq!(
            on.get("disableDeploymentModeChooser"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn read_top_level_missing_seeds_marker() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("missing.json");
        let m = read_top_level(&p).unwrap();
        assert_eq!(m.get("_cfprefsMigrated"), Some(&Value::Bool(true)));
    }

    #[test]
    fn enterprise_config_bedrock_keys() {
        let r = Route {
            id: Uuid::new_v4(),
            name: "Bedrock prod".into(),
            provider: RouteProvider::Bedrock(BedrockConfig {
                region: "us-west-2".into(),
                bearer_token: Some("token".into()),
                base_url: None,
                aws_profile: Some("claudepot".into()),
                skip_aws_auth: false,
                use_keychain: false,
            }),
            model: "anthropic.claude-sonnet-4".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-bedrock".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        };
        let m = build_enterprise_config(&r, false);
        assert_eq!(
            m.get("inferenceProvider"),
            Some(&Value::String("bedrock".into()))
        );
        assert_eq!(
            m.get("inferenceBedrockRegion"),
            Some(&Value::String("us-west-2".into()))
        );
        assert_eq!(
            m.get("inferenceBedrockProfile"),
            Some(&Value::String("claudepot".into()))
        );
        // Optional field absent when not set:
        assert!(m.get("inferenceBedrockBaseUrl").is_none());
    }

    #[test]
    fn enterprise_config_vertex_keys() {
        let r = Route {
            id: Uuid::new_v4(),
            name: "Vertex".into(),
            provider: RouteProvider::Vertex(VertexConfig {
                project_id: "my-proj".into(),
                region: Some("us-east5".into()),
                base_url: None,
                skip_gcp_auth: false,
            }),
            model: "claude-sonnet-4-5".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-vertex".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        };
        let m = build_enterprise_config(&r, true);
        assert_eq!(
            m.get("inferenceProvider"),
            Some(&Value::String("vertex".into()))
        );
        assert_eq!(
            m.get("inferenceVertexProjectId"),
            Some(&Value::String("my-proj".into()))
        );
        assert_eq!(
            m.get("inferenceVertexRegion"),
            Some(&Value::String("us-east5".into()))
        );
        assert_eq!(
            m.get("disableDeploymentModeChooser"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn enterprise_config_foundry_resource_xor_url() {
        let r_resource = Route {
            id: Uuid::new_v4(),
            name: "Foundry res".into(),
            provider: RouteProvider::Foundry(FoundryConfig {
                api_key: Some("k".into()),
                base_url: None,
                resource: Some("my-resource".into()),
                skip_azure_auth: false,
                use_keychain: false,
            }),
            model: "claude-sonnet-4-5".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-foundry".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        };
        let m = build_enterprise_config(&r_resource, false);
        assert_eq!(
            m.get("inferenceFoundryResource"),
            Some(&Value::String("my-resource".into()))
        );
        assert!(m.get("inferenceFoundryBaseUrl").is_none());

        let mut r_url = r_resource.clone();
        if let RouteProvider::Foundry(ref mut cfg) = r_url.provider {
            cfg.resource = None;
            cfg.base_url = Some("https://x.openai.azure.com".into());
        }
        let m2 = build_enterprise_config(&r_url, false);
        assert_eq!(
            m2.get("inferenceFoundryBaseUrl"),
            Some(&Value::String("https://x.openai.azure.com".into()))
        );
        assert!(m2.get("inferenceFoundryResource").is_none());
    }

    #[test]
    fn read_top_level_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        std::fs::write(
            &p,
            r#"{"_cfprefsMigrated": true, "foo": "bar", "enterpriseConfig": {}}"#,
        )
        .unwrap();
        let m = read_top_level(&p).unwrap();
        assert_eq!(m.get("foo"), Some(&Value::String("bar".into())));
        assert_eq!(m.get("_cfprefsMigrated"), Some(&Value::Bool(true)));
    }
}
