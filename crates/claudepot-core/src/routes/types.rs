use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable per-route identifier. Used as both the JSON-store key and
/// the filename in `Claude-3p/configLibrary/<uuid>.json`.
pub type RouteId = Uuid;

/// What kind of backend the route talks to. Mirrors Anthropic's
/// `inferenceProvider` enum verbatim so we can write the same string
/// into `enterpriseConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Gateway,
    Bedrock,
    Vertex,
    Foundry,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Gateway => "gateway",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::Vertex => "vertex",
            ProviderKind::Foundry => "foundry",
        }
    }
}

/// HTTP auth scheme the gateway expects. Mirrors
/// `inferenceGatewayAuthScheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthScheme {
    Bearer,
    Basic,
}

impl AuthScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthScheme::Bearer => "bearer",
            AuthScheme::Basic => "basic",
        }
    }
}

/// Gateway-provider configuration (Ollama, OpenRouter, Kimi, vLLM,
/// LiteLLM, etc.). Field names track Anthropic's `inferenceGateway*`
/// keys for direct round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// `inferenceGatewayBaseUrl`.
    pub base_url: String,
    /// `inferenceGatewayApiKey`. Stored plaintext — same posture as
    /// Claude Desktop's "Apply locally" UI. May be a placeholder for
    /// local servers (e.g. Ollama accepts any string).
    pub api_key: String,
    /// `inferenceGatewayAuthScheme`. Defaults to `Bearer` when omitted.
    #[serde(default = "default_auth_scheme")]
    pub auth_scheme: AuthScheme,
    /// Whether to set `ENABLE_TOOL_SEARCH=true` so CC keeps the
    /// `tool_reference` beta blocks active. Only enable for proxies
    /// that forward beta headers (LiteLLM passthrough, Cloudflare AI
    /// Gateway). See design doc §15.3.
    #[serde(default)]
    pub enable_tool_search: bool,
    /// When true, `api_key` is stored in the OS keychain instead of
    /// in this struct (the field is blanked on persist) and the
    /// wrapper / Cowork-on-3P pull it via a helper script. See
    /// `routes::helper` and `routes::keychain`.
    #[serde(default)]
    pub use_keychain: bool,
}

fn default_auth_scheme() -> AuthScheme {
    AuthScheme::Bearer
}

/// Bedrock-provider configuration. Mirrors `inferenceBedrock*` keys.
/// One of `bearer_token`, `aws_profile`, or `inferenceCredentialHelper`
/// must provide credentials at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockConfig {
    /// `inferenceBedrockRegion`. Required.
    pub region: String,
    /// `inferenceBedrockBearerToken`. Stored plaintext when present.
    /// `None` falls back to `aws_profile` or external helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
    /// `inferenceBedrockBaseUrl`. Optional override (LiteLLM, custom
    /// endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// `inferenceBedrockProfile`. Named AWS profile (e.g. resolved by
    /// `~/.aws/credentials`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_profile: Option<String>,
    /// When true, set `CLAUDE_CODE_SKIP_BEDROCK_AUTH=1` in the
    /// wrapper — for gateways that handle AWS auth on the proxy side.
    #[serde(default)]
    pub skip_aws_auth: bool,
    /// Keychain-backing for `bearer_token`. See [`GatewayConfig`].
    #[serde(default)]
    pub use_keychain: bool,
}

/// Vertex-provider configuration. `project_id` is required; other
/// fields override CC defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexConfig {
    /// `inferenceVertexProjectId` (mirrored as
    /// `ANTHROPIC_VERTEX_PROJECT_ID` in the wrapper). Required.
    pub project_id: String,
    /// `inferenceVertexRegion` / `CLOUD_ML_REGION`. Falls back to
    /// CC's `us-east5` default when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// `inferenceVertexBaseUrl`. Optional override for LiteLLM-fronted
    /// Vertex routes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// When true, set `CLAUDE_CODE_SKIP_VERTEX_AUTH=1` in the wrapper.
    #[serde(default)]
    pub skip_gcp_auth: bool,
}

/// Foundry-provider configuration. Set EITHER `base_url` (full URL)
/// OR `resource` (Azure resource name; SDK constructs the URL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundryConfig {
    /// `inferenceFoundryApiKey`. Stored plaintext when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// `inferenceFoundryBaseUrl`. Mutually exclusive with `resource`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// `inferenceFoundryResource`. Mutually exclusive with `base_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// When true, set `CLAUDE_CODE_SKIP_FOUNDRY_AUTH=1` in the wrapper.
    #[serde(default)]
    pub skip_azure_auth: bool,
    /// Keychain-backing for `api_key`. See [`GatewayConfig`].
    #[serde(default)]
    pub use_keychain: bool,
}

/// Per-provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RouteProvider {
    Gateway(GatewayConfig),
    Bedrock(BedrockConfig),
    Vertex(VertexConfig),
    Foundry(FoundryConfig),
}

impl RouteProvider {
    pub fn kind(&self) -> ProviderKind {
        match self {
            RouteProvider::Gateway(_) => ProviderKind::Gateway,
            RouteProvider::Bedrock(_) => ProviderKind::Bedrock,
            RouteProvider::Vertex(_) => ProviderKind::Vertex,
            RouteProvider::Foundry(_) => ProviderKind::Foundry,
        }
    }
}

/// One named third-party backend route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub id: RouteId,
    /// User-facing display name (free text, e.g. "Local Ollama",
    /// "OpenRouter Kimi K2"). Unique across the store.
    pub name: String,
    pub provider: RouteProvider,
    /// Primary model — drives the wrapper-name slug derivation and
    /// becomes `ANTHROPIC_MODEL` in the wrapper script.
    pub model: String,
    /// Override for `ANTHROPIC_SMALL_FAST_MODEL` (haiku-tier slot).
    /// `None` falls back to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_fast_model: Option<String>,
    /// Additional models (shown in Claude Desktop's picker, not
    /// pinned per-wrapper). `inferenceModels` becomes
    /// `[model, ...additional_models]`.
    #[serde(default)]
    pub additional_models: Vec<String>,
    /// Wrapper binary name on PATH. Defaults to
    /// `claude-<model-slug>`; user-overridable.
    pub wrapper_name: String,
    /// Per-route org UUID for Cowork session scoping. Auto-generated.
    pub deployment_organization_uuid: RouteId,
    /// Whether the route is currently the "active" 3P profile in
    /// Claude Desktop's `enterpriseConfig`. At most one route may
    /// have this true at a time.
    #[serde(default)]
    pub active_on_desktop: bool,
    /// Whether the wrapper script currently exists on disk.
    #[serde(default)]
    pub installed_on_cli: bool,
}

/// Lightweight projection for IPC / GUI list views. Carries no
/// secrets — `api_key` is replaced with a preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSummary {
    pub id: RouteId,
    pub name: String,
    pub provider_kind: ProviderKind,
    pub base_url: String,
    pub api_key_preview: String,
    pub model: String,
    pub small_fast_model: Option<String>,
    pub additional_models: Vec<String>,
    pub wrapper_name: String,
    pub active_on_desktop: bool,
    pub installed_on_cli: bool,
}

impl Route {
    pub fn summary(&self) -> RouteSummary {
        let (base_url, api_key_preview) = match &self.provider {
            RouteProvider::Gateway(cfg) => {
                (cfg.base_url.clone(), preview_secret(&cfg.api_key))
            }
            RouteProvider::Bedrock(cfg) => {
                let url = cfg.base_url.clone().unwrap_or_else(|| {
                    format!("bedrock://{}", cfg.region)
                });
                let preview = match (cfg.bearer_token.as_ref(), cfg.aws_profile.as_ref()) {
                    (Some(t), _) => preview_secret(t),
                    (None, Some(p)) => format!("aws-profile:{p}"),
                    (None, None) => String::from("(iam-default)"),
                };
                (url, preview)
            }
            RouteProvider::Vertex(cfg) => {
                let url = cfg
                    .base_url
                    .clone()
                    .unwrap_or_else(|| format!("vertex://{}", cfg.project_id));
                (url, format!("project:{}", cfg.project_id))
            }
            RouteProvider::Foundry(cfg) => {
                let url = match (cfg.base_url.as_ref(), cfg.resource.as_ref()) {
                    (Some(u), _) => u.clone(),
                    (None, Some(r)) => format!("foundry://{r}"),
                    (None, None) => String::from("(unconfigured)"),
                };
                let preview = cfg
                    .api_key
                    .as_deref()
                    .map(preview_secret)
                    .unwrap_or_else(|| String::from("(none)"));
                (url, preview)
            }
        };
        RouteSummary {
            id: self.id,
            name: self.name.clone(),
            provider_kind: self.provider.kind(),
            base_url,
            api_key_preview,
            model: self.model.clone(),
            small_fast_model: self.small_fast_model.clone(),
            additional_models: self.additional_models.clone(),
            wrapper_name: self.wrapper_name.clone(),
            active_on_desktop: self.active_on_desktop,
            installed_on_cli: self.installed_on_cli,
        }
    }
}

/// Truncated key preview that fits the `sk-ant-oat01-Abc…xyz` shape
/// used elsewhere in Claudepot. Fewer than 8 chars → return placeholder.
fn preview_secret(key: &str) -> String {
    if key.is_empty() {
        return String::from("(none)");
    }
    let len = key.chars().count();
    if len <= 8 {
        return format!("{}…", "*".repeat(len.min(4)));
    }
    let head: String = key.chars().take(6).collect();
    let tail: String = key.chars().skip(len.saturating_sub(3)).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_renders_lowercase() {
        assert_eq!(ProviderKind::Gateway.as_str(), "gateway");
        assert_eq!(ProviderKind::Bedrock.as_str(), "bedrock");
        assert_eq!(ProviderKind::Vertex.as_str(), "vertex");
        assert_eq!(ProviderKind::Foundry.as_str(), "foundry");
    }

    #[test]
    fn auth_scheme_renders_lowercase() {
        assert_eq!(AuthScheme::Bearer.as_str(), "bearer");
        assert_eq!(AuthScheme::Basic.as_str(), "basic");
    }

    #[test]
    fn provider_kind_serde_roundtrip() {
        let s = serde_json::to_string(&ProviderKind::Gateway).unwrap();
        assert_eq!(s, "\"gateway\"");
        let back: ProviderKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ProviderKind::Gateway);
    }

    #[test]
    fn preview_secret_short() {
        assert_eq!(preview_secret(""), "(none)");
        assert_eq!(preview_secret("ab"), "**…");
        assert_eq!(preview_secret("eight888"), "****…");
    }

    #[test]
    fn preview_secret_long() {
        assert_eq!(
            preview_secret("sk-or-v1-abcdef1234567890xyz"),
            "sk-or-…xyz"
        );
    }

    #[test]
    fn route_summary_omits_api_key() {
        let r = Route {
            id: Uuid::nil(),
            name: "Test".into(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: "http://127.0.0.1:11434".into(),
                api_key: "secret-12345-xyz".into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain: false,
            }),
            model: "llama3.2:3b".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-llama3-2-3b".into(),
            deployment_organization_uuid: Uuid::nil(),
            active_on_desktop: false,
            installed_on_cli: false,
        };
        let s = r.summary();
        assert_eq!(s.api_key_preview, "secret…xyz");
        assert!(!s.api_key_preview.contains("12345"));
    }
}
