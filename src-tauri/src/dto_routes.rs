//! DTOs for the third-party routes feature.
//!
//! Direction: API keys cross the IPC bridge **inbound only** (in
//! `RouteCreateDto` / `RouteUpdateDto`, where the user just typed
//! them into the GUI form). Outbound DTOs (`RouteSummaryDto`)
//! return only a previewed key. This matches the IPC trust rules
//! in `.claude/rules/architecture.md`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSummaryDto {
    pub id: String,
    pub name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub api_key_preview: String,
    pub model: String,
    pub small_fast_model: Option<String>,
    pub additional_models: Vec<String>,
    pub wrapper_name: String,
    pub active_on_desktop: bool,
    pub installed_on_cli: bool,
    pub enable_tool_search: bool,
    pub auth_scheme: String,
    pub use_keychain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayInputDto {
    pub base_url: String,
    pub api_key: String,
    /// "bearer" | "basic". Defaults to "bearer" when empty.
    #[serde(default)]
    pub auth_scheme: String,
    #[serde(default)]
    pub enable_tool_search: bool,
    #[serde(default)]
    pub use_keychain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockInputDto {
    pub region: String,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub aws_profile: String,
    #[serde(default)]
    pub skip_aws_auth: bool,
    #[serde(default)]
    pub use_keychain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexInputDto {
    pub project_id: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub skip_gcp_auth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundryInputDto {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub resource: String,
    #[serde(default)]
    pub skip_azure_auth: bool,
    #[serde(default)]
    pub use_keychain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteCreateDto {
    pub name: String,
    pub provider_kind: String,
    pub gateway: Option<GatewayInputDto>,
    pub bedrock: Option<BedrockInputDto>,
    pub vertex: Option<VertexInputDto>,
    pub foundry: Option<FoundryInputDto>,
    pub model: String,
    pub small_fast_model: Option<String>,
    #[serde(default)]
    pub additional_models: Vec<String>,
    /// Optional override; empty/absent means auto-derive from `model`.
    #[serde(default)]
    pub wrapper_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteUpdateDto {
    pub id: String,
    pub name: String,
    pub provider_kind: String,
    pub gateway: Option<GatewayInputDto>,
    pub bedrock: Option<BedrockInputDto>,
    pub vertex: Option<VertexInputDto>,
    pub foundry: Option<FoundryInputDto>,
    pub model: String,
    pub small_fast_model: Option<String>,
    #[serde(default)]
    pub additional_models: Vec<String>,
    pub wrapper_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSettingsDto {
    pub disable_deployment_mode_chooser: bool,
}
