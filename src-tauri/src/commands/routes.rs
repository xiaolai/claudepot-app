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
    activate_desktop, add_route, add_wrapper_dir_to_path, clear_desktop_active, delete_helpers,
    delete_keychain_for_route, delete_library_profile, delete_wrapper, derive_wrapper_slug,
    edit_route, normalize_gateway_base_url, sanitize_wrapper_name, validate_base_url,
    wrapper_dir_path_status, write_wrapper, zeroize_provider_secrets, AuthScheme, BedrockConfig,
    FoundryConfig, GatewayConfig, OsRouteEffects, ProviderKind, Route, RouteError, RouteProvider,
    RouteStore, SaveRouteError, VertexConfig,
};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::dto_routes::{
    BedrockDetailsDto, BedrockInputDto, FoundryDetailsDto, FoundryInputDto, GatewayDetailsDto,
    GatewayInputDto, RouteCreateDto, RouteDetailsDto, RouteSettingsDto, RouteSummaryDto,
    RouteUpdateDto, VertexDetailsDto, VertexInputDto,
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

fn secret_empty_to_none(mut s: String) -> Option<String> {
    let trimmed = s.trim();
    let out = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    s.zeroize();
    out
}

/// Scrub the secret fields of route-input DTOs. For error exits that
/// return BEFORE `build_provider` consumes the DTOs — the file
/// header's zeroize-on-every-exit contract covers those too (audit
/// T7). Vertex carries no secret.
fn zeroize_route_inputs(
    gateway: &mut Option<GatewayInputDto>,
    bedrock: &mut Option<BedrockInputDto>,
    foundry: &mut Option<FoundryInputDto>,
) {
    if let Some(g) = gateway.as_mut() {
        g.api_key.zeroize();
    }
    if let Some(b) = bedrock.as_mut() {
        b.bearer_token.zeroize();
    }
    if let Some(f) = foundry.as_mut() {
        f.api_key.zeroize();
    }
}

fn build_provider(
    kind: ProviderKind,
    mut gateway: Option<GatewayInputDto>,
    mut bedrock: Option<BedrockInputDto>,
    vertex: Option<VertexInputDto>,
    mut foundry: Option<FoundryInputDto>,
) -> Result<RouteProvider, String> {
    // Scrub the NON-selected provider inputs up front: they are never
    // read, and the `config missing` error arms below would otherwise
    // drop their secrets unscrubbed (audit T7).
    if !matches!(kind, ProviderKind::Gateway) {
        if let Some(g) = gateway.as_mut() {
            g.api_key.zeroize();
        }
    }
    if !matches!(kind, ProviderKind::Bedrock) {
        if let Some(b) = bedrock.as_mut() {
            b.bearer_token.zeroize();
        }
    }
    if !matches!(kind, ProviderKind::Foundry) {
        if let Some(f) = foundry.as_mut() {
            f.api_key.zeroize();
        }
    }
    match kind {
        ProviderKind::Gateway => {
            let mut g = gateway.ok_or_else(|| String::from("gateway config missing"))?;
            // Normalize, not just validate: a trailing `/v1` is stripped
            // here because Claude Code's SDK appends `/v1/messages`
            // itself — see `normalize_gateway_base_url`.
            let base = match normalize_gateway_base_url(&g.base_url) {
                Ok(base) => base,
                Err(e) => {
                    g.api_key.zeroize();
                    return Err(format!("invalid gateway base URL: {e}"));
                }
            };
            Ok(RouteProvider::Gateway(GatewayConfig {
                base_url: base,
                api_key: g.api_key,
                auth_scheme: parse_auth_scheme(&g.auth_scheme),
                enable_tool_search: g.enable_tool_search,
                use_keychain: g.use_keychain,
            }))
        }
        ProviderKind::Bedrock => {
            let b = bedrock.ok_or_else(|| String::from("bedrock config missing"))?;
            let region = b.region.trim();
            if region.is_empty() {
                let mut bearer = b.bearer_token;
                bearer.zeroize();
                return Err(String::from("AWS region is required"));
            }
            let mut bearer = secret_empty_to_none(b.bearer_token);
            let profile = empty_to_none(b.aws_profile);
            if !b.skip_aws_auth && bearer.is_none() && profile.is_none() {
                if let Some(token) = bearer.as_mut() {
                    token.zeroize();
                }
                return Err(String::from(
                    "Bedrock needs a bearer token, AWS profile, or skip_aws_auth set",
                ));
            }
            // Validate the optional override URL when present.
            let validated_base = match empty_to_none(b.base_url) {
                Some(url) => Some(validate_base_url(&url).map_err(|e| {
                    if let Some(token) = bearer.as_mut() {
                        token.zeroize();
                    }
                    format!("invalid Bedrock base URL: {e}")
                })?),
                None => None,
            };
            Ok(RouteProvider::Bedrock(BedrockConfig {
                region: region.to_string(),
                bearer_token: bearer,
                base_url: validated_base,
                aws_profile: profile,
                skip_aws_auth: b.skip_aws_auth,
                use_keychain: b.use_keychain,
            }))
        }
        ProviderKind::Vertex => {
            let v = vertex.ok_or_else(|| String::from("vertex config missing"))?;
            let project_id = v.project_id.trim();
            if project_id.is_empty() {
                return Err(String::from("GCP project ID is required"));
            }
            let validated_base = match empty_to_none(v.base_url) {
                Some(url) => Some(
                    validate_base_url(&url).map_err(|e| format!("invalid Vertex base URL: {e}"))?,
                ),
                None => None,
            };
            Ok(RouteProvider::Vertex(VertexConfig {
                project_id: project_id.to_string(),
                region: empty_to_none(v.region),
                base_url: validated_base,
                skip_gcp_auth: v.skip_gcp_auth,
            }))
        }
        ProviderKind::Foundry => {
            let f = foundry.ok_or_else(|| String::from("foundry config missing"))?;
            let base = empty_to_none(f.base_url);
            let resource = empty_to_none(f.resource);
            let mut api_key = secret_empty_to_none(f.api_key);
            if base.is_none() && resource.is_none() {
                if let Some(key) = api_key.as_mut() {
                    key.zeroize();
                }
                return Err(String::from(
                    "Foundry needs either a base URL or a resource name",
                ));
            }
            if base.is_some() && resource.is_some() {
                if let Some(key) = api_key.as_mut() {
                    key.zeroize();
                }
                return Err(String::from(
                    "Foundry: choose base URL OR resource name, not both",
                ));
            }
            let validated_base = match base {
                Some(url) => Some(validate_base_url(&url).map_err(|e| {
                    if let Some(key) = api_key.as_mut() {
                        key.zeroize();
                    }
                    format!("invalid Foundry base URL: {e}")
                })?),
                None => None,
            };
            Ok(RouteProvider::Foundry(FoundryConfig {
                api_key,
                base_url: validated_base,
                resource,
                skip_azure_auth: f.skip_azure_auth,
                use_keychain: f.use_keychain,
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

// The secret-commit policy (`commit_secrets`), the shadow-copy
// scrubbing, and the add/edit save transactions live in
// `claudepot_core::routes::lifecycle` — unit-tested there with fake
// effects. This file keeps DTO parsing, inbound-secret zeroize, and
// error stringification only.

fn project_summary(r: &Route) -> RouteSummaryDto {
    let s = r.summary();
    let (auth_scheme, enable_tool_search, use_keychain) = match &r.provider {
        RouteProvider::Gateway(cfg) => (
            cfg.auth_scheme.as_str().to_string(),
            cfg.enable_tool_search,
            cfg.use_keychain,
        ),
        RouteProvider::Bedrock(cfg) => (String::from("bearer"), false, cfg.use_keychain),
        RouteProvider::Vertex(_) => (String::from("bearer"), false, false),
        RouteProvider::Foundry(cfg) => (String::from("bearer"), false, cfg.use_keychain),
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
        use_keychain,
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

fn project_details(r: &Route) -> RouteDetailsDto {
    let s = r.summary();
    let (gateway, bedrock, vertex, foundry, use_keychain) = match &r.provider {
        RouteProvider::Gateway(cfg) => (
            Some(GatewayDetailsDto {
                base_url: cfg.base_url.clone(),
                api_key_preview: s.api_key_preview.clone(),
                has_api_key: cfg.use_keychain || !cfg.api_key.is_empty(),
                auth_scheme: cfg.auth_scheme.as_str().to_string(),
                enable_tool_search: cfg.enable_tool_search,
            }),
            None,
            None,
            None,
            cfg.use_keychain,
        ),
        RouteProvider::Bedrock(cfg) => (
            None,
            Some(BedrockDetailsDto {
                region: cfg.region.clone(),
                bearer_token_preview: s.api_key_preview.clone(),
                has_bearer_token: cfg.use_keychain || cfg.bearer_token.is_some(),
                base_url: cfg.base_url.clone(),
                aws_profile: cfg.aws_profile.clone(),
                skip_aws_auth: cfg.skip_aws_auth,
            }),
            None,
            None,
            cfg.use_keychain,
        ),
        RouteProvider::Vertex(cfg) => (
            None,
            None,
            Some(VertexDetailsDto {
                project_id: cfg.project_id.clone(),
                region: cfg.region.clone(),
                base_url: cfg.base_url.clone(),
                skip_gcp_auth: cfg.skip_gcp_auth,
            }),
            None,
            false,
        ),
        RouteProvider::Foundry(cfg) => (
            None,
            None,
            None,
            Some(FoundryDetailsDto {
                api_key_preview: s.api_key_preview.clone(),
                has_api_key: cfg.use_keychain || cfg.api_key.is_some(),
                base_url: cfg.base_url.clone(),
                resource: cfg.resource.clone(),
                skip_azure_auth: cfg.skip_azure_auth,
            }),
            cfg.use_keychain,
        ),
    };
    RouteDetailsDto {
        id: s.id.to_string(),
        name: s.name,
        provider_kind: s.provider_kind.as_str().to_string(),
        gateway,
        bedrock,
        vertex,
        foundry,
        model: s.model,
        small_fast_model: s.small_fast_model,
        additional_models: s.additional_models,
        wrapper_name: s.wrapper_name,
        active_on_desktop: s.active_on_desktop,
        installed_on_cli: s.installed_on_cli,
        use_keychain,
    }
}

#[tauri::command]
pub async fn routes_list() -> Result<Vec<RouteSummaryDto>, String> {
    let store = open_store()?;
    Ok(store.list().iter().map(project_summary).collect())
}

#[tauri::command]
pub async fn routes_get(id: String) -> Result<RouteDetailsDto, String> {
    let id = parse_route_id(&id)?;
    let store = open_store()?;
    let route = store
        .get(id)
        .ok_or_else(|| RouteError::NotFound(id.to_string()).to_string())?;
    Ok(project_details(route))
}

#[tauri::command]
pub async fn routes_settings_get() -> Result<RouteSettingsDto, String> {
    let store = open_store()?;
    Ok(RouteSettingsDto {
        disable_deployment_mode_chooser: store.disable_chooser(),
    })
}

#[tauri::command]
pub async fn routes_settings_set(settings: RouteSettingsDto) -> Result<RouteSettingsDto, String> {
    let mut store = open_store()?;
    let prev_disable = store.disable_chooser();
    store
        .set_disable_chooser(settings.disable_deployment_mode_chooser)
        .map_err(map_err)?;
    // If the chooser flag changed AND there's a route currently
    // active on Desktop, re-mirror its enterpriseConfig so the new
    // flag takes effect on the next launch instead of staying stale
    // until the user activates / deactivates the route.
    if prev_disable != settings.disable_deployment_mode_chooser {
        let active: Option<Route> = store.list().iter().find(|r| r.active_on_desktop).cloned();
        if let Some(route) = active {
            let _ = activate_desktop(&route, settings.disable_deployment_mode_chooser);
        }
    }
    Ok(RouteSettingsDto {
        disable_deployment_mode_chooser: store.disable_chooser(),
    })
}

#[tauri::command]
pub async fn routes_add(mut route: RouteCreateDto) -> Result<RouteSummaryDto, String> {
    let provider_kind = match parse_provider(&route.provider_kind) {
        Ok(k) => k,
        Err(e) => {
            zeroize_route_inputs(&mut route.gateway, &mut route.bedrock, &mut route.foundry);
            return Err(e);
        }
    };
    let mut provider = build_provider(
        provider_kind,
        route.gateway.take(),
        route.bedrock.take(),
        route.vertex.take(),
        route.foundry.take(),
    )?;
    let wrapper = match pick_wrapper_name(&route.wrapper_name, &route.model) {
        Ok(w) => w,
        Err(e) => {
            zeroize_provider_secrets(&mut provider);
            return Err(e);
        }
    };

    let new_route = Route {
        // Nil id — core's `add_route` assigns the UUID shared by the
        // keychain entries and the persisted record.
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
        is_private_cloud: false,
        capabilities_override: None,
    };

    // The commit → persist ordering and the keychain/helper rollback
    // on either failure live in core (`routes::lifecycle::add_route`).
    let mut store = open_store()?;
    let saved = add_route(&mut store, new_route, &OsRouteEffects).map_err(map_err)?;
    Ok(project_summary(&saved))
}

#[tauri::command]
pub async fn routes_edit(mut route: RouteUpdateDto) -> Result<RouteSummaryDto, String> {
    let (id, provider_kind) = match parse_route_id(&route.id)
        .and_then(|id| Ok((id, parse_provider(&route.provider_kind)?)))
    {
        Ok(pair) => pair,
        Err(e) => {
            zeroize_route_inputs(&mut route.gateway, &mut route.bedrock, &mut route.foundry);
            return Err(e);
        }
    };

    let mut provider = build_provider(
        provider_kind,
        route.gateway.take(),
        route.bedrock.take(),
        route.vertex.take(),
        route.foundry.take(),
    )?;
    // Audit fix for commands_routes.rs:562 — validate everything we
    // can BEFORE rotating any keychain secrets (the rotation itself
    // happens inside core's `edit_route`). A wrapper-name failure
    // here costs nothing: no keychain entry has been touched yet.
    let wrapper = match pick_wrapper_name(&route.wrapper_name, &route.model) {
        Ok(w) => w,
        Err(e) => {
            zeroize_provider_secrets(&mut provider);
            return Err(e);
        }
    };

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
        is_private_cloud: false,
        capabilities_override: None,
    };

    // Core's `edit_route` owns the prev-capture → commit_secrets →
    // store.update → side-effects ordering, the shadow-secret
    // scrubbing, and the "never delete helpers on a failed update"
    // invariant (audit fix for commands_routes.rs:613). This command
    // only stringifies the outcomes.
    let mut store = open_store()?;
    let saved = match edit_route(&mut store, candidate, &OsRouteEffects) {
        Ok(s) => s,
        Err(SaveRouteError::Store(e)) => {
            // The persisted route is unchanged; the keychain may
            // already hold the new secret while the route still
            // references the old shape — tell the user to re-save.
            return Err(format!(
                "{e}; the previously-saved route remains active. \
                 If the route stops working after retry, re-enter the secret and save again."
            ));
        }
        Err(e) => return Err(e.to_string()),
    };

    if !saved.warnings.is_empty() {
        return Err(format!(
            "route saved, but follow-up writes had warnings — {}",
            saved.warnings.join("; ")
        ));
    }
    Ok(project_summary(&saved.route))
}

#[tauri::command]
pub async fn routes_remove(id: String) -> Result<(), String> {
    let id = parse_route_id(&id)?;
    let mut store = open_store()?;
    let removed = store.remove(id).map_err(map_err)?;
    // Side effects: tear down wrapper + clear Desktop activation +
    // delete the library profile (which may carry plaintext secrets) +
    // forget any keychain entries / helper scripts the route owned.
    //
    // Each cleanup step that fails leaves a real artifact behind
    // (a wrapper script, a library profile carrying plaintext, a
    // keychain entry, a helper script). Returning Ok in those cases
    // lies to the UI and lets the user move on while secrets persist
    // on disk. Collect the failures and surface them as an aggregate
    // error so the user can rerun or hand-clean the residue.
    let mut errors: Vec<String> = Vec::new();
    if removed.installed_on_cli {
        if let Err(e) = delete_wrapper(&removed.wrapper_name) {
            errors.push(format!("wrapper {}: {e}", removed.wrapper_name));
        }
    }
    if removed.active_on_desktop {
        if let Err(e) = clear_desktop_active() {
            errors.push(format!("desktop activation: {e}"));
        }
    }
    if let Err(e) = delete_library_profile(id) {
        errors.push(format!("library profile: {e}"));
    }
    if let Err(e) = delete_keychain_for_route(id) {
        errors.push(format!("keychain: {e}"));
    }
    if let Err(e) = delete_helpers(id, None) {
        errors.push(format!("helpers: {e}"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "route removed but {} cleanup step(s) failed: {}",
            errors.len(),
            errors.join("; ")
        ))
    }
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
    if let Err(e) = store.set_installed_cli(id, true) {
        // Roll back the wrapper file we just wrote so disk state
        // matches the persisted (failed-to-update) flag.
        let _ = delete_wrapper(&route.wrapper_name);
        return Err(map_err(e));
    }
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
    // Persist the flag first; only then try to delete the file.
    // Reverse order to use_cli — if delete fails after the flag is
    // off, we'd want a "stale file" warning, but at least the flag
    // is the source of truth and a follow-up rerun can clean up.
    store.set_installed_cli(id, false).map_err(map_err)?;
    if let Err(e) = delete_wrapper(&route.wrapper_name) {
        // Restore the flag so the route's persisted state matches
        // the wrapper that's still on disk.
        let _ = store.set_installed_cli(id, true);
        return Err(map_err(e));
    }
    let r = store
        .get(id)
        .ok_or_else(|| String::from("route disappeared after persist"))?;
    Ok(project_summary(r))
}

/// Whether `~/.claudepot/bin` (where CLI wrappers land) is on the
/// user's interactive-shell PATH. Returns `"on_path"`,
/// `"not_on_path"`, or `"unknown"` — see `routes::PathStatus`. The
/// Third-party UI uses this to render an honest wrapper indicator
/// instead of assuming "wrapper written" means "wrapper reachable".
#[tauri::command]
pub async fn routes_path_status() -> Result<String, String> {
    Ok(wrapper_dir_path_status().await.as_str().to_string())
}

/// Append the wrapper-dir `export PATH` line to the user's shell rc
/// file (`.zshrc` / `.bash_profile`). Idempotent. Returns the path
/// of the rc file that was written so the UI can name it. Errors on
/// shells whose config syntax we don't auto-edit (e.g. fish).
#[tauri::command]
pub async fn routes_add_to_path() -> Result<String, String> {
    let rc = add_wrapper_dir_to_path().map_err(map_err)?;
    Ok(rc.display().to_string())
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
    if let Err(e) = store.set_active_desktop(Some(id)) {
        // Best-effort: tear down what we just wrote so the persisted
        // "no route active" flag matches the on-disk enterpriseConfig.
        let _ = clear_desktop_active();
        return Err(map_err(e));
    }
    let r = store
        .get(id)
        .ok_or_else(|| String::from("route disappeared after persist"))?;
    Ok(project_summary(r))
}

#[tauri::command]
pub async fn routes_unuse_desktop() -> Result<(), String> {
    let mut store = open_store()?;
    // Capture the previously-active route id so we can re-mirror if
    // the persist step fails. Without this, a flag-set failure after
    // the file was cleared would leave a route flagged active in the
    // store with no enterpriseConfig backing it.
    let prev_active: Option<Route> = store.list().iter().find(|r| r.active_on_desktop).cloned();
    let disable = store.disable_chooser();
    clear_desktop_active().map_err(map_err)?;
    if let Err(e) = store.set_active_desktop(None) {
        if let Some(r) = &prev_active {
            let _ = activate_desktop(r, disable);
        }
        return Err(map_err(e));
    }
    Ok(())
}

#[tauri::command]
pub async fn routes_derive_slug(model: String) -> Result<String, String> {
    Ok(derive_wrapper_slug(&model))
}

#[tauri::command]
pub async fn routes_validate_wrapper_name(name: String) -> Result<String, String> {
    sanitize_wrapper_name(&name).map_err(|e| format!("invalid wrapper name '{name}': {e}"))
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
