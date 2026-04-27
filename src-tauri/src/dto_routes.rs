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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteCreateDto {
    pub name: String,
    pub provider_kind: String,
    pub gateway: Option<GatewayInputDto>,
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
