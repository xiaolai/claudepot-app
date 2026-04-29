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
    activate_desktop, clear_desktop_active, delete_helpers, delete_keychain_for_route,
    delete_library_profile, delete_wrapper, derive_wrapper_slug, sanitize_wrapper_name,
    store_keychain_secret, validate_base_url, write_helper, write_library_profile, write_wrapper,
    AuthScheme, BedrockConfig, FoundryConfig, GatewayConfig, ProviderKind, Route, RouteError,
    RouteId, RouteProvider, RouteStore, SecretField, VertexConfig,
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

fn build_provider(
    kind: ProviderKind,
    gateway: Option<GatewayInputDto>,
    bedrock: Option<BedrockInputDto>,
    vertex: Option<VertexInputDto>,
    foundry: Option<FoundryInputDto>,
) -> Result<RouteProvider, String> {
    match kind {
        ProviderKind::Gateway => {
            let g = gateway.ok_or_else(|| String::from("gateway config missing"))?;
            let base = validate_base_url(&g.base_url)
                .map_err(|e| format!("invalid gateway base URL: {e}"))?;
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
                return Err(String::from("AWS region is required"));
            }
            let bearer = empty_to_none(b.bearer_token);
            let profile = empty_to_none(b.aws_profile);
            if !b.skip_aws_auth && bearer.is_none() && profile.is_none() {
                return Err(String::from(
                    "Bedrock needs a bearer token, AWS profile, or skip_aws_auth set",
                ));
            }
            // Validate the optional override URL when present.
            let validated_base = match empty_to_none(b.base_url) {
                Some(url) => Some(
                    validate_base_url(&url)
                        .map_err(|e| format!("invalid Bedrock base URL: {e}"))?,
                ),
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
            let validated_base = match base {
                Some(url) => Some(
                    validate_base_url(&url)
                        .map_err(|e| format!("invalid Foundry base URL: {e}"))?,
                ),
                None => None,
            };
            Ok(RouteProvider::Foundry(FoundryConfig {
                api_key: empty_to_none(f.api_key),
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

/// Side-effect function that resolves how each provider's secret
/// should be stored, given the new (post-form) and the previous
/// (already-on-disk) provider configs:
///
///   - **Plaintext mode (`use_keychain == false`)**: blank secret on
///     edit means "keep prev value"; non-empty replaces.
///   - **Keychain mode (`use_keychain == true`)**: any non-empty
///     incoming secret is written to the OS keychain and the helper
///     script is (re)materialized; the inline field is then blanked
///     so it never reaches the routes.json on disk.
///
/// Run before `RouteStore::add` / `update` so the persisted route
/// reflects the post-effect state.
fn commit_secrets(
    new_provider: &mut RouteProvider,
    route_id: RouteId,
    prev: Option<&RouteProvider>,
) -> Result<(), String> {
    match new_provider {
        RouteProvider::Gateway(cfg) => {
            if cfg.use_keychain {
                if !cfg.api_key.is_empty() {
                    store_keychain_secret(route_id, SecretField::GatewayApiKey, &cfg.api_key)
                        .map_err(map_err)?;
                    write_helper(route_id, SecretField::GatewayApiKey).map_err(map_err)?;
                }
                // Zeroize before truncate — `String::clear` only sets
                // len = 0, leaving the secret bytes in the buffer until
                // the allocator hands them out for reuse.
                cfg.api_key.zeroize();
            } else if cfg.api_key.is_empty() {
                if let Some(RouteProvider::Gateway(p)) = prev {
                    cfg.api_key = p.api_key.clone();
                }
            }
        }
        RouteProvider::Bedrock(cfg) => {
            if cfg.use_keychain {
                if let Some(mut t) = cfg.bearer_token.take() {
                    if !t.is_empty() {
                        store_keychain_secret(route_id, SecretField::BedrockBearerToken, &t)
                            .map_err(map_err)?;
                        write_helper(route_id, SecretField::BedrockBearerToken).map_err(map_err)?;
                    }
                    t.zeroize();
                }
                cfg.bearer_token = None;
            } else {
                let need_inherit = cfg.bearer_token.as_ref().is_some_and(|t| t.is_empty());
                if need_inherit {
                    if let Some(RouteProvider::Bedrock(p)) = prev {
                        cfg.bearer_token = p.bearer_token.clone();
                    } else {
                        cfg.bearer_token = None;
                    }
                }
            }
        }
        RouteProvider::Foundry(cfg) => {
            if cfg.use_keychain {
                if let Some(mut k) = cfg.api_key.take() {
                    if !k.is_empty() {
                        store_keychain_secret(route_id, SecretField::FoundryApiKey, &k)
                            .map_err(map_err)?;
                        write_helper(route_id, SecretField::FoundryApiKey).map_err(map_err)?;
                    }
                    k.zeroize();
                }
                cfg.api_key = None;
            } else {
                let need_inherit = cfg.api_key.as_ref().is_some_and(|k| k.is_empty());
                if need_inherit {
                    if let Some(RouteProvider::Foundry(p)) = prev {
                        cfg.api_key = p.api_key.clone();
                    } else {
                        cfg.api_key = None;
                    }
                }
            }
        }
        RouteProvider::Vertex(_) => {}
    }
    Ok(())
}

/// Snapshot every inline secret on a provider config into owned
/// Strings. Caller is expected to zeroize them on every exit path
/// (used by `routes_edit` to scrub shadow copies even when the
/// store-update path drops the original `candidate` without
/// scrubbing).
fn collect_inline_secrets(p: &RouteProvider) -> Vec<String> {
    match p {
        RouteProvider::Gateway(c) if !c.api_key.is_empty() => {
            vec![c.api_key.clone()]
        }
        RouteProvider::Bedrock(c) => c
            .bearer_token
            .as_ref()
            .filter(|t| !t.is_empty())
            .cloned()
            .map(|t| vec![t])
            .unwrap_or_default(),
        RouteProvider::Foundry(c) => c
            .api_key
            .as_ref()
            .filter(|k| !k.is_empty())
            .cloned()
            .map(|k| vec![k])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Scrub every inline secret on a provider config. Used by error
/// paths so a half-built route doesn't leave the user-typed key
/// resident in process memory until the allocator overwrites it.
fn zeroize_provider_secrets(p: &mut RouteProvider) {
    match p {
        RouteProvider::Gateway(c) => c.api_key.zeroize(),
        RouteProvider::Bedrock(c) => {
            if let Some(t) = c.bearer_token.as_mut() {
                t.zeroize();
            }
            c.bearer_token = None;
        }
        RouteProvider::Foundry(c) => {
            if let Some(k) = c.api_key.as_mut() {
                k.zeroize();
            }
            c.api_key = None;
        }
        RouteProvider::Vertex(_) => {}
    }
}

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
    let provider_kind = parse_provider(&route.provider_kind)?;
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

    let route_id = Uuid::new_v4();
    if let Err(e) = commit_secrets(&mut provider, route_id, None) {
        // commit_secrets may have written to keychain / dropped a helper
        // before failing — best-effort tear-down so we don't leak state
        // for a route that was never persisted.
        let _ = delete_keychain_for_route(route_id);
        let _ = delete_helpers(route_id, None);
        zeroize_provider_secrets(&mut provider);
        return Err(e);
    }

    let new_route = Route {
        id: route_id,
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
    let saved = match store.add(new_route) {
        Ok(s) => s,
        Err(mut e) => {
            // Roll back any keychain / helper writes commit_secrets did,
            // since the store call rejected the route.
            let _ = delete_keychain_for_route(route_id);
            let _ = delete_helpers(route_id, None);
            // We can't reach the moved provider anymore; the in-flight
            // copy was scrubbed by commit_secrets if it took the
            // keychain path. The error itself is a String — make sure
            // it doesn't carry the secret (it doesn't, by error-type
            // construction, but be defensive).
            let s = e.to_string();
            e = RouteError::Io(std::io::Error::other(s.clone()));
            return Err(e.to_string());
        }
    };
    Ok(project_summary(&saved))
}

#[tauri::command]
pub async fn routes_edit(mut route: RouteUpdateDto) -> Result<RouteSummaryDto, String> {
    let id = parse_route_id(&route.id)?;
    let provider_kind = parse_provider(&route.provider_kind)?;

    // Capture the prior provider so commit_secrets can decide
    // "blank = keep existing" for plaintext mode, and capture the
    // prior wrapper name to detect renames for stale-file cleanup.
    let (prev_provider, prev_wrapper_name) = {
        let store = open_store()?;
        let prev = store
            .get(id)
            .ok_or_else(|| RouteError::NotFound(id.to_string()).to_string())?;
        (
            prev.provider.clone(),
            if prev.installed_on_cli {
                Some(prev.wrapper_name.clone())
            } else {
                None
            },
        )
    };

    let mut provider = build_provider(
        provider_kind,
        route.gateway.take(),
        route.bedrock.take(),
        route.vertex.take(),
        route.foundry.take(),
    )?;
    if let Err(e) = commit_secrets(&mut provider, id, Some(&prev_provider)) {
        zeroize_provider_secrets(&mut provider);
        return Err(e);
    }
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
    };

    let mut store = open_store()?;
    // Snapshot any inline secrets in the candidate into local Strings
    // BEFORE we move it into `store.update`. The local strings are
    // explicitly zeroized at every exit point of this function so
    // even if `store.update` fails (and drops `candidate` without
    // scrubbing), we still scrub our shadow copies.
    let mut shadow_secrets = collect_inline_secrets(&candidate.provider);
    let updated = match store.update(candidate) {
        Ok(u) => u,
        Err(e) => {
            for s in shadow_secrets.iter_mut() {
                s.zeroize();
            }
            // Best-effort: roll any keychain helpers we just wrote.
            // Any keychain entry that `commit_secrets` overwrote
            // already replaced the prior value and we don't have
            // the old one to restore, so flag that to the caller.
            let _ = delete_helpers(id, None);
            return Err(format!(
                "{e}; helper scripts cleaned up but keychain entries may still hold the new secret",
            ));
        }
    };
    for s in shadow_secrets.iter_mut() {
        s.zeroize();
    }

    // Post-store side effects: surface any failure so the user sees
    // when wrapper / Desktop state diverges from the persisted route
    // (instead of silently leaving stale files behind).
    let mut warnings: Vec<String> = Vec::new();
    if updated.installed_on_cli {
        if let Some(prev_name) = &prev_wrapper_name {
            if prev_name != &updated.wrapper_name {
                if let Err(e) = delete_wrapper(prev_name) {
                    warnings.push(format!(
                        "old wrapper '{prev_name}' could not be removed: {e}"
                    ));
                }
            }
        }
        if let Err(e) = write_wrapper(&updated) {
            warnings.push(format!("wrapper rewrite failed: {e}"));
        }
    }
    // Always rewrite the library profile (regardless of active state),
    // so a defined-but-inactive 3P profile in `configLibrary/` reflects
    // the latest fields and any pre-existing plaintext secret on disk
    // is replaced.
    if let Err(e) = write_library_profile(&updated) {
        warnings.push(format!("Desktop library profile write failed: {e}"));
    }
    if updated.active_on_desktop {
        let disable = store.disable_chooser();
        if let Err(e) = activate_desktop(&updated, disable) {
            warnings.push(format!("Desktop activation re-mirror failed: {e}"));
        }
    }

    if !warnings.is_empty() {
        return Err(format!(
            "route saved, but follow-up writes had warnings — {}",
            warnings.join("; ")
        ));
    }
    Ok(project_summary(&updated))
}

#[tauri::command]
pub async fn routes_remove(id: String) -> Result<(), String> {
    let id = parse_route_id(&id)?;
    let mut store = open_store()?;
    let removed = store.remove(id).map_err(map_err)?;
    // Side effects: tear down wrapper + clear Desktop activation +
    // delete the library profile (which may carry plaintext secrets) +
    // forget any keychain entries / helper scripts the route owned.
    if removed.installed_on_cli {
        let _ = delete_wrapper(&removed.wrapper_name);
    }
    if removed.active_on_desktop {
        let _ = clear_desktop_active();
    }
    let _ = delete_library_profile(id);
    let _ = delete_keychain_for_route(id);
    let _ = delete_helpers(id, None);
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
