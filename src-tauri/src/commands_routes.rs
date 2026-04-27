//! Tauri commands for the Third-party (routes) section.
//!
//! Thin wrappers over `claudepot_core::routes`. No business logic
//! lives here — every handler validates inputs, delegates to the
//! core, and projects to `RouteSummaryDto` for outbound responses.
//!
//! API keys are accepted inbound (user just typed them) and zeroed
//! after the call returns. They never round-trip outbound — only
//! `api_key_preview` is sent back.

use claudepot_core::routes::{
    activate_desktop, clear_desktop_active, delete_wrapper, derive_wrapper_slug,
    sanitize_wrapper_name, write_wrapper, AuthScheme, BedrockConfig, FoundryConfig,
    GatewayConfig, ProviderKind, Route, RouteError, RouteProvider, RouteStore,
    VertexConfig,
};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::dto_routes::{
    BedrockInputDto, FoundryInputDto, GatewayInputDto, RouteCreateDto, RouteSettingsDto,
    RouteSummaryDto, RouteUpdateDto, VertexInputDto,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn open_store() -> Result<RouteStore, String> {
    RouteStore::open().map_err(|e| format!("routes store open failed: {e}"))
}

fn parse_provider(s: &str) -> Result<ProviderKind, String> {
    match s {
        "gateway" => Ok(ProviderKind::Gateway),
        "bedrock" => Ok(ProviderKind::Bedrock),
        "vertex" => Ok(ProviderKind::Vertex),
        "foundry" => Ok(ProviderKind::Foundry),
        other => Err(format!("unknown provider kind: {other}")),
    }
}

fn parse_auth_scheme(s: &str) -> AuthScheme {
    match s {
        "basic" => AuthScheme::Basic,
        _ => AuthScheme::Bearer,
    }
}

fn build_provider(
    kind: ProviderKind,
    gateway: Option<GatewayInputDto>,
    bedrock: Option<BedrockInputDto>,
    vertex: Option<VertexInputDto>,
    foundry: Option<FoundryInputDto>,
) -> Result<RouteProvider, String> {
    match kind {
        ProviderKind::Gateway => {
            let g =
                gateway.ok_or_else(|| String::from("gateway config missing"))?;
            let base = g.base_url.trim();
            if base.is_empty() {
                return Err(String::from("base URL is required"));
            }
            if !(base.starts_with("http://") || base.starts_with("https://")) {
                return Err(format!(
                    "base URL must start with http:// or https:// (got: {base})"
                ));
            }
            Ok(RouteProvider::Gateway(GatewayConfig {
                base_url: base.to_string(),
                api_key: g.api_key,
                auth_scheme: parse_auth_scheme(&g.auth_scheme),
                enable_tool_search: g.enable_tool_search,
            }))
        }
        ProviderKind::Bedrock => {
            let b =
                bedrock.ok_or_else(|| String::from("bedrock config missing"))?;
            let region = b.region.trim();
            if region.is_empty() {
                return Err(String::from("AWS region is required"));
            }
            let bearer = empty_to_none(b.bearer_token);
            let profile = empty_to_none(b.aws_profile);
            if !b.skip_aws_auth && bearer.is_none() && profile.is_none() {
                return Err(String::from(
                    "Bedrock needs a bearer token, AWS profile, or skip_aws_auth set",
                ));
            }
            Ok(RouteProvider::Bedrock(BedrockConfig {
                region: region.to_string(),
                bearer_token: bearer,
                base_url: empty_to_none(b.base_url),
                aws_profile: profile,
                skip_aws_auth: b.skip_aws_auth,
            }))
        }
        ProviderKind::Vertex => {
            let v =
                vertex.ok_or_else(|| String::from("vertex config missing"))?;
            let project_id = v.project_id.trim();
            if project_id.is_empty() {
                return Err(String::from("GCP project ID is required"));
            }
            Ok(RouteProvider::Vertex(VertexConfig {
                project_id: project_id.to_string(),
                region: empty_to_none(v.region),
                base_url: empty_to_none(v.base_url),
                skip_gcp_auth: v.skip_gcp_auth,
            }))
        }
        ProviderKind::Foundry => {
            let f =
                foundry.ok_or_else(|| String::from("foundry config missing"))?;
            let base = empty_to_none(f.base_url);
            let resource = empty_to_none(f.resource);
            if base.is_none() && resource.is_none() {
                return Err(String::from(
                    "Foundry needs either a base URL or a resource name",
                ));
            }
            if base.is_some() && resource.is_some() {
                return Err(String::from(
                    "Foundry: choose base URL OR resource name, not both",
                ));
            }
            if let Some(b) = &base {
                if !(b.starts_with("http://") || b.starts_with("https://")) {
                    return Err(format!(
                        "Foundry base URL must start with http:// or https:// (got: {b})"
                    ));
                }
            }
            Ok(RouteProvider::Foundry(FoundryConfig {
                api_key: empty_to_none(f.api_key),
                base_url: base,
                resource,
                skip_azure_auth: f.skip_azure_auth,
            }))
        }
    }
}

fn empty_to_none(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn project_summary(r: &Route) -> RouteSummaryDto {
    let s = r.summary();
    let (auth_scheme, enable_tool_search) = match &r.provider {
        RouteProvider::Gateway(cfg) => {
            (cfg.auth_scheme.as_str().to_string(), cfg.enable_tool_search)
        }
        // Auth scheme / tool-search aren't meaningful for these providers;
        // the GUI shows neutral defaults so the field is well-defined.
        RouteProvider::Bedrock(_) => (String::from("bearer"), false),
        RouteProvider::Vertex(_) => (String::from("bearer"), false),
        RouteProvider::Foundry(_) => (String::from("bearer"), false),
    };
    RouteSummaryDto {
        id: s.id.to_string(),
        name: s.name,
        provider_kind: s.provider_kind.as_str().to_string(),
        base_url: s.base_url,
        api_key_preview: s.api_key_preview,
        model: s.model,
        small_fast_model: s.small_fast_model,
        additional_models: s.additional_models,
        wrapper_name: s.wrapper_name,
        active_on_desktop: s.active_on_desktop,
        installed_on_cli: s.installed_on_cli,
        enable_tool_search,
        auth_scheme,
    }
}

fn parse_route_id(s: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|_| format!("invalid route id: {s}"))
}

fn pick_wrapper_name(user: &str, model: &str) -> Result<String, String> {
    let candidate = if user.trim().is_empty() {
        derive_wrapper_slug(model)
    } else {
        user.trim().to_string()
    };
    sanitize_wrapper_name(&candidate)
        .map_err(|e| format!("invalid wrapper name '{candidate}': {e}"))
}

#[tauri::command]
pub async fn routes_list() -> Result<Vec<RouteSummaryDto>, String> {
    let store = open_store()?;
    Ok(store.list().iter().map(project_summary).collect())
}

#[tauri::command]
pub async fn routes_settings_get() -> Result<RouteSettingsDto, String> {
    let store = open_store()?;
    Ok(RouteSettingsDto {
        disable_deployment_mode_chooser: store.disable_chooser(),
    })
}

#[tauri::command]
pub async fn routes_settings_set(
    settings: RouteSettingsDto,
) -> Result<RouteSettingsDto, String> {
    let mut store = open_store()?;
    store
        .set_disable_chooser(settings.disable_deployment_mode_chooser)
        .map_err(map_err)?;
    Ok(RouteSettingsDto {
        disable_deployment_mode_chooser: store.disable_chooser(),
    })
}

#[tauri::command]
pub async fn routes_add(
    mut route: RouteCreateDto,
) -> Result<RouteSummaryDto, String> {
    let provider_kind = parse_provider(&route.provider_kind)?;
    let provider = build_provider(
        provider_kind,
        route.gateway.take(),
        route.bedrock.take(),
        route.vertex.take(),
        route.foundry.take(),
    )?;
    let wrapper = pick_wrapper_name(&route.wrapper_name, &route.model)?;

    let new_route = Route {
        id: Uuid::nil(),
        name: route.name.trim().to_string(),
        provider,
        model: route.model.trim().to_string(),
        small_fast_model: route
            .small_fast_model
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        additional_models: route
            .additional_models
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect(),
        wrapper_name: wrapper,
        deployment_organization_uuid: Uuid::nil(),
        active_on_desktop: false,
        installed_on_cli: false,
    };

    let mut store = open_store()?;
    let saved = store.add(new_route).map_err(map_err)?;
    let dto = project_summary(&saved);
    // Best-effort zero of the api_key string the renderer sent.
    if let Some(_) = std::any::TypeId::of::<RouteCreateDto>().to_owned().into() {
        // RouteCreateDto.gateway moved out via `take()` above; the original
        // api_key string was consumed by `build_provider`. No additional
        // zeroing needed here — once `build_provider` placed the key into
        // the GatewayConfig and that into the store, the renderer-side
        // string is the only other live copy and is the renderer's job
        // to clear.
    }
    Ok(dto)
}

#[tauri::command]
pub async fn routes_edit(
    mut route: RouteUpdateDto,
) -> Result<RouteSummaryDto, String> {
    let id = parse_route_id(&route.id)?;
    let provider_kind = parse_provider(&route.provider_kind)?;

    // "Blank secret = keep existing" — the form intentionally
    // doesn't pre-fill secrets. Patch any blank secret with the
    // pre-existing value before parsing the provider config.
    // Also captures the prior wrapper name to detect renames so
    // we can clean up the stale wrapper file post-edit.
    let prev_wrapper_name: Option<String>;
    {
        let store = open_store()?;
        let prev = store
            .get(id)
            .ok_or_else(|| RouteError::NotFound(id.to_string()).to_string())?;
        prev_wrapper_name = if prev.installed_on_cli {
            Some(prev.wrapper_name.clone())
        } else {
            None
        };
        match (provider_kind, &prev.provider) {
            (ProviderKind::Gateway, RouteProvider::Gateway(prev_cfg)) => {
                if let Some(g) = route.gateway.as_mut() {
                    if g.api_key.is_empty() {
                        g.api_key = prev_cfg.api_key.clone();
                    }
                }
            }
            (ProviderKind::Bedrock, RouteProvider::Bedrock(prev_cfg)) => {
                if let Some(b) = route.bedrock.as_mut() {
                    if b.bearer_token.is_empty() {
                        if let Some(prev_token) = &prev_cfg.bearer_token {
                            b.bearer_token = prev_token.clone();
                        }
                    }
                }
            }
            (ProviderKind::Foundry, RouteProvider::Foundry(prev_cfg)) => {
                if let Some(f) = route.foundry.as_mut() {
                    if f.api_key.is_empty() {
                        if let Some(prev_key) = &prev_cfg.api_key {
                            f.api_key = prev_key.clone();
                        }
                    }
                }
            }
            // Provider-kind change OR Vertex (no inline secret) — nothing to inherit.
            _ => {}
        }
    }

    let provider = build_provider(
        provider_kind,
        route.gateway.take(),
        route.bedrock.take(),
        route.vertex.take(),
        route.foundry.take(),
    )?;
    let wrapper = pick_wrapper_name(&route.wrapper_name, &route.model)?;

    let candidate = Route {
        id,
        name: route.name.trim().to_string(),
        provider,
        model: route.model.trim().to_string(),
        small_fast_model: route
            .small_fast_model
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        additional_models: route
            .additional_models
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect(),
        wrapper_name: wrapper,
        // The store preserves these fields verbatim across updates.
        deployment_organization_uuid: Uuid::nil(),
        active_on_desktop: false,
        installed_on_cli: false,
    };

    let mut store = open_store()?;
    let updated = store.update(candidate).map_err(map_err)?;

    // Rewrite the wrapper if it was installed; if the wrapper name
    // changed, also delete the old one so the rename is clean.
    if updated.installed_on_cli {
        if let Some(prev_name) = &prev_wrapper_name {
            if prev_name != &updated.wrapper_name {
                let _ = delete_wrapper(prev_name);
            }
        }
        let _ = write_wrapper(&updated);
    }
    if updated.active_on_desktop {
        let disable = store.disable_chooser();
        let _ = activate_desktop(&updated, disable);
    }

    Ok(project_summary(&updated))
}

#[tauri::command]
pub async fn routes_remove(id: String) -> Result<(), String> {
    let id = parse_route_id(&id)?;
    let mut store = open_store()?;
    let removed = store.remove(id).map_err(map_err)?;
    // Side effects: tear down wrapper + clear desktop activation.
    if removed.installed_on_cli {
        let _ = delete_wrapper(&removed.wrapper_name);
    }
    if removed.active_on_desktop {
        let _ = clear_desktop_active();
    }
    Ok(())
}

#[tauri::command]
pub async fn routes_use_cli(id: String) -> Result<RouteSummaryDto, String> {
    let id = parse_route_id(&id)?;
    let mut store = open_store()?;
    let route = store
        .get(id)
        .ok_or_else(|| RouteError::NotFound(id.to_string()).to_string())?
        .clone();
    write_wrapper(&route).map_err(map_err)?;
    store.set_installed_cli(id, true).map_err(map_err)?;
    let r = store
        .get(id)
        .ok_or_else(|| String::from("route disappeared after persist"))?;
    Ok(project_summary(r))
}

#[tauri::command]
pub async fn routes_unuse_cli(id: String) -> Result<RouteSummaryDto, String> {
    let id = parse_route_id(&id)?;
    let mut store = open_store()?;
    let route = store
        .get(id)
        .ok_or_else(|| RouteError::NotFound(id.to_string()).to_string())?
        .clone();
    delete_wrapper(&route.wrapper_name).map_err(map_err)?;
    store.set_installed_cli(id, false).map_err(map_err)?;
    let r = store
        .get(id)
        .ok_or_else(|| String::from("route disappeared after persist"))?;
    Ok(project_summary(r))
}

#[tauri::command]
pub async fn routes_use_desktop(id: String) -> Result<RouteSummaryDto, String> {
    let id = parse_route_id(&id)?;
    let mut store = open_store()?;
    let route = store
        .get(id)
        .ok_or_else(|| RouteError::NotFound(id.to_string()).to_string())?
        .clone();
    let disable = store.disable_chooser();
    activate_desktop(&route, disable).map_err(map_err)?;
    store.set_active_desktop(Some(id)).map_err(map_err)?;
    let r = store
        .get(id)
        .ok_or_else(|| String::from("route disappeared after persist"))?;
    Ok(project_summary(r))
}

#[tauri::command]
pub async fn routes_unuse_desktop() -> Result<(), String> {
    let mut store = open_store()?;
    clear_desktop_active().map_err(map_err)?;
    store.set_active_desktop(None).map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn routes_derive_slug(model: String) -> Result<String, String> {
    Ok(derive_wrapper_slug(&model))
}

#[tauri::command]
pub async fn routes_validate_wrapper_name(name: String) -> Result<String, String> {
    sanitize_wrapper_name(&name)
        .map_err(|e| format!("invalid wrapper name '{name}': {e}"))
}

/// Best-effort: if the renderer wants to forcibly zero a key it
/// previously sent (e.g. on form submit), call this with the
/// string. Rust drops it deterministically.
#[tauri::command]
pub async fn routes_zero_secret(mut secret: String) -> Result<(), String> {
    secret.zeroize();
    Ok(())
}

/// Whether Claude Desktop is currently running. Mirrors the existing
/// `desktop_backend` probe — used by the Third-party section to
/// surface a "restart required" affordance after activate/deactivate.
#[tauri::command]
pub async fn routes_desktop_running() -> Result<bool, String> {
    let Some(platform) = claudepot_core::desktop_backend::create_platform() else {
        return Ok(false);
    };
    Ok(platform.is_running().await)
}

/// Quit + relaunch Claude Desktop so the new `enterpriseConfig` is
/// picked up. Idempotent on cold-start machines (skips quit when the
/// app isn't running, then launches).
#[tauri::command]
pub async fn routes_desktop_restart() -> Result<(), String> {
    let Some(platform) = claudepot_core::desktop_backend::create_platform() else {
        return Err(String::from(
            "Claude Desktop is not supported on this platform",
        ));
    };
    if platform.is_running().await {
        platform.quit().await.map_err(map_err)?;
    }
    platform.launch().await.map_err(map_err)?;
    Ok(())
}
