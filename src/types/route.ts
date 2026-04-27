// Route DTOs (third-party LLM backends).
// Mirrors src-tauri/src/dto_routes.rs.

/** Provider value matching Anthropic's `inferenceProvider`. */
export type RouteProviderKind = "gateway" | "bedrock" | "vertex" | "foundry";

/** Auth scheme matching `inferenceGatewayAuthScheme`. */
export type RouteAuthScheme = "bearer" | "basic";

/**
 * Outbound projection. Carries no full secrets — `api_key_preview`
 * is a `sk-or-…xyz`-shape truncation; the full key never crosses
 * outward over IPC. See dto_routes.rs header.
 */
export interface RouteSummaryDto {
  id: string;
  name: string;
  provider_kind: RouteProviderKind;
  base_url: string;
  api_key_preview: string;
  model: string;
  small_fast_model: string | null;
  additional_models: string[];
  wrapper_name: string;
  active_on_desktop: boolean;
  installed_on_cli: boolean;
  enable_tool_search: boolean;
  auth_scheme: RouteAuthScheme;
  use_keychain: boolean;
}

export interface GatewayInputDto {
  base_url: string;
  api_key: string;
  /** Empty string defaults to `bearer` on the Rust side. */
  auth_scheme: RouteAuthScheme | "";
  enable_tool_search: boolean;
  use_keychain: boolean;
}

export interface BedrockInputDto {
  region: string;
  bearer_token: string;
  base_url: string;
  aws_profile: string;
  skip_aws_auth: boolean;
  use_keychain: boolean;
}

export interface VertexInputDto {
  project_id: string;
  region: string;
  base_url: string;
  skip_gcp_auth: boolean;
}

export interface FoundryInputDto {
  api_key: string;
  base_url: string;
  resource: string;
  skip_azure_auth: boolean;
  use_keychain: boolean;
}

export interface RouteCreateDto {
  name: string;
  provider_kind: RouteProviderKind;
  gateway: GatewayInputDto | null;
  bedrock: BedrockInputDto | null;
  vertex: VertexInputDto | null;
  foundry: FoundryInputDto | null;
  model: string;
  small_fast_model: string | null;
  additional_models: string[];
  /** Empty / absent → Rust auto-derives `claude-<model-slug>`. */
  wrapper_name: string;
}

export interface RouteUpdateDto {
  id: string;
  name: string;
  provider_kind: RouteProviderKind;
  gateway: GatewayInputDto | null;
  bedrock: BedrockInputDto | null;
  vertex: VertexInputDto | null;
  foundry: FoundryInputDto | null;
  model: string;
  small_fast_model: string | null;
  additional_models: string[];
  wrapper_name: string;
}

export interface RouteSettingsDto {
  disable_deployment_mode_chooser: boolean;
}

// ── Outbound DETAILS DTOs ────────────────────────────────────────
//
// Returned by `routes_get` so the Edit modal can hydrate every
// provider-specific field. Secrets stay opaque — callers see
// `*_preview` and `has_*` booleans but never the actual value.

export interface GatewayDetailsDto {
  base_url: string;
  api_key_preview: string;
  has_api_key: boolean;
  auth_scheme: RouteAuthScheme;
  enable_tool_search: boolean;
}

export interface BedrockDetailsDto {
  region: string;
  bearer_token_preview: string;
  has_bearer_token: boolean;
  base_url: string | null;
  aws_profile: string | null;
  skip_aws_auth: boolean;
}

export interface VertexDetailsDto {
  project_id: string;
  region: string | null;
  base_url: string | null;
  skip_gcp_auth: boolean;
}

export interface FoundryDetailsDto {
  api_key_preview: string;
  has_api_key: boolean;
  base_url: string | null;
  resource: string | null;
  skip_azure_auth: boolean;
}

export interface RouteDetailsDto {
  id: string;
  name: string;
  provider_kind: RouteProviderKind;
  gateway: GatewayDetailsDto | null;
  bedrock: BedrockDetailsDto | null;
  vertex: VertexDetailsDto | null;
  foundry: FoundryDetailsDto | null;
  model: string;
  small_fast_model: string | null;
  additional_models: string[];
  wrapper_name: string;
  active_on_desktop: boolean;
  installed_on_cli: boolean;
  use_keychain: boolean;
}
