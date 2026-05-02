//! Tauri command surface for templates.
//!
//! Five commands map to the install + read paths described in
//! `dev-docs/templates-implementation-plan.md` §6:
//!
//! - `templates_list` — gallery card data
//! - `templates_get` — full details for one template
//! - `templates_sample_report` — bundled sample markdown
//! - `templates_capable_routes` — Routes that can run a given
//!   template (filtered by capability + privacy)
//! - `templates_install` — instantiate + persist as a regular
//!   automation
//!
//! Routes flag derivation (`is_local`, `is_private_cloud`,
//! `capabilities_override`) is partial today: `is_local` is
//! computed from gateway URLs in this module; `is_private_cloud`
//! defaults to false until the routes module exposes a flag.
//! `capabilities_override` is read when present.

use claudepot_core::automations::types::HostPlatform;
use claudepot_core::automations::AutomationStore;
use claudepot_core::paths::claudepot_data_dir;
use claudepot_core::routes::{Route, RouteProvider, RouteStore};
use claudepot_core::templates::apply::{apply_selected, sidecar, ApplyReceipt, PendingChanges};
use claudepot_core::templates::routing::{evaluate, RoutingRules, RoutingStore, Suggestion};
use claudepot_core::templates::{
    self as tpl, Blueprint, PrivacyClass, TemplateInstance, TemplateRegistry,
};
use serde::Serialize;

use crate::dto_automations::{AutomationCreateDto, AutomationSummaryDto, PlatformOptionsDto};
use crate::dto_templates::{TemplateDetailsDto, TemplateInstanceDto, TemplateSummaryDto};

#[derive(Debug, Clone, Serialize)]
pub struct RouteSummaryDto {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub is_local: bool,
    pub is_private_cloud: bool,
    pub is_capable: bool,
    /// Plain-English explanation when `is_capable == false`. Empty
    /// otherwise.
    pub ineligibility_reason: String,
}

fn registry() -> Result<&'static TemplateRegistry, String> {
    use std::sync::OnceLock;
    static CELL: OnceLock<Result<TemplateRegistry, String>> = OnceLock::new();
    CELL.get_or_init(|| TemplateRegistry::load_bundled().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| e.clone())
}

#[tauri::command]
pub async fn templates_list() -> Result<Vec<TemplateSummaryDto>, String> {
    use claudepot_core::automations::types::HostPlatform;
    let r = registry()?;
    // Filter to templates that declare support for the current
    // host. Without this, every macOS-shaped blueprint shows up
    // on Linux/Windows and fails opaquely at run time.
    Ok(r.list_for(HostPlatform::current())
        .map(TemplateSummaryDto::from)
        .collect())
}

#[tauri::command]
pub async fn templates_get(id: String) -> Result<TemplateDetailsDto, String> {
    let r = registry()?;
    let bp = r
        .get(&id)
        .ok_or_else(|| format!("unknown template id: {id}"))?;
    Ok(TemplateDetailsDto::from(bp))
}

#[tauri::command]
pub async fn templates_sample_report(id: String) -> Result<String, String> {
    let r = registry()?;
    r.sample_report(&id)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no sample report bundled for template id: {id}"))
}

/// Read a `pending-changes.json` side-car for a specific run.
/// The path is scoped to the claudepot data dir (which honors
/// `CLAUDEPOT_DATA_DIR`) like other report reads. Returns the
/// parsed structure.
#[tauri::command]
pub async fn templates_pending_changes(path: String) -> Result<PendingChanges, String> {
    use std::path::PathBuf;
    let claudepot_root = claudepot_data_dir();
    let target = PathBuf::from(&path);
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("cannot resolve {path}: {e}"))?;
    let canonical_root = claudepot_root
        .canonicalize()
        .unwrap_or(claudepot_root.clone());
    if !canonical_target.starts_with(&canonical_root) {
        return Err(format!(
            "pending-changes path is outside {}: {}",
            canonical_root.display(),
            canonical_target.display()
        ));
    }
    sidecar::read(&canonical_target)
}

/// Apply selected items from a `pending-changes.json` side-car.
/// Re-validates every operation against the blueprint's apply
/// configuration before execution; rejected items appear in
/// the receipt as `Rejected` (and are never executed).
#[tauri::command]
pub async fn templates_apply_pending(
    automation_id: String,
    pending_path: String,
    selected_ids: Vec<String>,
) -> Result<ApplyReceipt, String> {
    let pending = templates_pending_changes(pending_path).await?;

    // Refuse to apply pending changes whose recorded
    // `automation_id` doesn't match the caller's argument.
    // Otherwise a sidecar from automation A could be paired with
    // automation B's broader apply policy and execute under the
    // wrong scope.
    if pending.automation_id != automation_id {
        return Err(format!(
            "pending-changes file belongs to automation {} but apply was requested for {}",
            pending.automation_id, automation_id
        ));
    }

    // Resolve the blueprint via the automation's template_id.
    let store =
        AutomationStore::open().map_err(|e| format!("automations store open failed: {e}"))?;
    let auto = store
        .list()
        .iter()
        .find(|a| a.id.to_string() == automation_id)
        .ok_or_else(|| format!("automation {automation_id} not found"))?
        .clone();
    let template_id = auto
        .template_id
        .clone()
        .ok_or_else(|| format!("automation {automation_id} is not template-driven"))?;
    let r = registry()?;
    let bp = r
        .get(&template_id)
        .ok_or_else(|| format!("blueprint {template_id} not in registry"))?;
    let apply = bp
        .apply
        .as_ref()
        .ok_or_else(|| format!("blueprint {template_id} has no apply config"))?;

    let receipt = apply_selected(&pending, apply, &selected_ids).await;
    Ok(receipt)
}

// ---------- Routing rules ----------

#[tauri::command]
pub async fn routing_rules_get() -> Result<RoutingRules, String> {
    let store = RoutingStore::open().map_err(|e| format!("routing rules open: {e}"))?;
    Ok(store.rules().clone())
}

#[tauri::command]
pub async fn routing_rules_set(rules: RoutingRules) -> Result<(), String> {
    let mut store = RoutingStore::open().map_err(|e| format!("routing rules open: {e}"))?;
    store.replace(rules);
    store
        .save()
        .map_err(|e| format!("routing rules save: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutingSuggestionDto {
    pub kind: String,
    pub route_id: Option<String>,
}

#[tauri::command]
pub async fn routing_rules_evaluate_for(
    blueprint_id: String,
) -> Result<RoutingSuggestionDto, String> {
    let r = registry()?;
    let bp = r
        .get(&blueprint_id)
        .ok_or_else(|| format!("unknown template id: {blueprint_id}"))?;
    let store = RoutingStore::open().map_err(|e| format!("routing rules open: {e}"))?;
    let route_store = RouteStore::open().map_err(|e| format!("routes store open: {e}"))?;
    let routes: Vec<&Route> = route_store.list().iter().collect();
    let privacy_str = match bp.privacy {
        PrivacyClass::Local => "local",
        PrivacyClass::PrivateCloud => "private_cloud",
        PrivacyClass::Any => "any",
    };
    let category_str = format!("{:?}", bp.category).to_lowercase();
    let cost_str = format!("{:?}", bp.cost_class).to_lowercase();

    let suggestion = evaluate(
        store.rules(),
        privacy_str,
        &category_str,
        &cost_str,
        &bp.id().0,
        &routes,
        &|r| is_local_route(r),
        &|r| {
            let caps = effective_capabilities(r);
            caps.contains_all(&bp.capabilities_required)
        },
    );
    Ok(match suggestion {
        Suggestion::Route(id) => RoutingSuggestionDto {
            kind: "route".into(),
            route_id: Some(id),
        },
        Suggestion::DefaultClaude => RoutingSuggestionDto {
            kind: "default_claude".into(),
            route_id: None,
        },
    })
}

/// Read a generated report file. Scoped to `~/.claudepot/` —
/// any path outside that directory is rejected. Caps file size
/// at 4 MB to bound the wire payload.
#[tauri::command]
pub async fn templates_read_report(path: String) -> Result<String, String> {
    use std::path::PathBuf;
    let claudepot_root = claudepot_data_dir();
    let target = PathBuf::from(&path);
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("cannot resolve {path}: {e}"))?;
    let canonical_root = claudepot_root
        .canonicalize()
        .unwrap_or(claudepot_root.clone());
    if !canonical_target.starts_with(&canonical_root) {
        return Err(format!(
            "report path is outside {}: {}",
            canonical_root.display(),
            canonical_target.display()
        ));
    }
    let meta = std::fs::metadata(&canonical_target).map_err(|e| format!("stat {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("not a file: {path}"));
    }
    if meta.len() > 4 * 1024 * 1024 {
        return Err(format!(
            "report is too large to display: {} bytes",
            meta.len()
        ));
    }
    std::fs::read_to_string(&canonical_target).map_err(|e| format!("read {path}: {e}"))
}

#[tauri::command]
pub async fn templates_capable_routes(id: String) -> Result<Vec<RouteSummaryDto>, String> {
    let r = registry()?;
    let bp = r
        .get(&id)
        .ok_or_else(|| format!("unknown template id: {id}"))?;
    let store = RouteStore::open().map_err(|e| format!("routes store open failed: {e}"))?;
    let summaries = store
        .list()
        .iter()
        .map(|rt| route_summary(rt, bp))
        .collect();
    Ok(filter_for_privacy(summaries, bp))
}

#[tauri::command]
pub async fn templates_install(
    instance: TemplateInstanceDto,
) -> Result<AutomationSummaryDto, String> {
    let r = registry()?;
    let bp = r
        .get(&instance.blueprint_id)
        .ok_or_else(|| format!("unknown template id: {}", instance.blueprint_id))?;
    // Defense-in-depth: the gallery filters by `supported_platforms`
    // via `templates_list`, but a direct IPC call to
    // `templates_install` would otherwise still let a macOS-only
    // template instantiate on Linux/Windows. Reject here so the
    // contract is symmetric.
    let host = HostPlatform::current();
    if !bp.supports(host) {
        return Err(format!(
            "template {} does not declare support for {:?}",
            bp.id().0,
            host
        ));
    }

    // Translate the wire DTO to the core `TemplateInstance`.
    let placeholder_values = decode_placeholder_values(bp, &instance.placeholder_values)?;
    let core_inst = TemplateInstance {
        blueprint_id: instance.blueprint_id.clone(),
        blueprint_schema_version: instance.blueprint_schema_version,
        placeholder_values,
        route_id: instance.route_id.clone(),
        schedule: instance.schedule.into_core()?,
        name_override: instance.name_override.clone(),
    };

    let resolved = tpl::instantiate(bp, &core_inst).map_err(|e| e.to_string())?;

    // Translate `ResolvedAutomation` into the existing
    // `AutomationCreateDto` shape and feed it into the existing
    // automation-add path. The two stores (templates registry +
    // automations store) are independent — no cross-store
    // transaction needed since template installation produces
    // exactly one automation row.
    let unique_name = derive_unique_name(&resolved.name)?;
    let dto = AutomationCreateDto {
        name: unique_name,
        display_name: resolved.display_name,
        description: resolved.description,
        binary_kind: resolved.binary_kind,
        binary_route_id: resolved.binary_route_id,
        model: resolved.model,
        cwd: resolved.cwd,
        prompt: resolved.prompt,
        system_prompt: None,
        append_system_prompt: None,
        permission_mode: resolved.permission_mode,
        allowed_tools: resolved.allowed_tools,
        add_dir: Vec::new(),
        max_budget_usd: resolved.max_budget_usd,
        fallback_model: None,
        output_format: "json".to_string(),
        json_schema: None,
        bare: false,
        extra_env: std::collections::BTreeMap::from([(
            "CLAUDEPOT_OUTPUT_PATH".to_string(),
            resolved.output_path,
        )]),
        trigger_kind: Some(resolved.trigger_kind),
        cron: resolved.cron,
        timezone: resolved.timezone,
        platform_options: PlatformOptionsDto {
            wake_to_run: false,
            catch_up_if_missed: true,
            run_when_logged_out: false,
        },
        log_retention_runs: resolved.log_retention_runs,
        template_id: Some(resolved.template_id),
    };

    crate::commands_automations::automations_add(dto).await
}

// ---------- Helpers ----------

/// Build a [`RouteSummaryDto`] for one route, deriving capability
/// and privacy compatibility flags. Honors the Route's
/// `capabilities_override` (the enforcement boundary) before
/// falling back to the templates module's default-by-prefix hint
/// table.
fn route_summary(rt: &Route, bp: &Blueprint) -> RouteSummaryDto {
    let caps = effective_capabilities(rt);
    let missing = caps.missing(&bp.capabilities_required);
    let is_local = is_local_route(rt);
    let is_private_cloud = rt.is_private_cloud;

    let (is_capable, reason) = match (bp.privacy, is_local, is_private_cloud, missing.is_empty()) {
        (PrivacyClass::Local, false, _, _) => {
            (false, "this template requires a local route".to_string())
        }
        (PrivacyClass::PrivateCloud, false, false, _) => (
            false,
            "this template requires a local or private-cloud route".to_string(),
        ),
        (_, _, _, false) => (
            false,
            format!(
                "missing capabilities: {}",
                missing
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        _ => (true, String::new()),
    };

    RouteSummaryDto {
        id: rt.id.to_string(),
        name: rt.name.clone(),
        provider: format!("{:?}", rt.provider.kind()),
        model: rt.model.clone(),
        is_local,
        is_private_cloud,
        is_capable,
        ineligibility_reason: reason,
    }
}

/// Resolve a route's effective capabilities. The Route's
/// `capabilities_override` is the enforcement boundary; when
/// absent, fall back to the templates module's default-by-prefix
/// hint table.
fn effective_capabilities(rt: &Route) -> tpl::CapabilitySet {
    if let Some(strs) = &rt.capabilities_override {
        let mut out = std::collections::HashSet::new();
        for s in strs {
            // Parse via the same serde rename used by Capability.
            if let Ok(c) = serde_json::from_value::<tpl::Capability>(serde_json::json!(s)) {
                out.insert(c);
            }
        }
        return tpl::CapabilitySet(out);
    }
    tpl::default_capabilities_for(&rt.model)
}

/// Derive `is_local` from a route's gateway URL when the
/// provider is Gateway: a localhost / 127.0.0.1 / unix-socket
/// base URL means the model runs on this machine. Cloud
/// providers (Bedrock, Vertex, Foundry) are never local.
fn is_local_route(rt: &Route) -> bool {
    if let RouteProvider::Gateway(cfg) = &rt.provider {
        let url = cfg.base_url.to_lowercase();
        return url.contains("://localhost")
            || url.contains("://127.0.0.1")
            || url.contains("://0.0.0.0")
            || url.starts_with("unix:")
            || url.starts_with("file://");
    }
    false
}

/// `local-only` and `private-cloud` blueprints get a hard filter:
/// only matching routes are returned. `any` returns the full list.
fn filter_for_privacy(summaries: Vec<RouteSummaryDto>, bp: &Blueprint) -> Vec<RouteSummaryDto> {
    match bp.privacy {
        PrivacyClass::Local => summaries.into_iter().filter(|r| r.is_local).collect(),
        PrivacyClass::PrivateCloud => summaries
            .into_iter()
            .filter(|r| r.is_local || r.is_private_cloud)
            .collect(),
        PrivacyClass::Any => summaries,
    }
}

/// Translate the wire's loosely-typed `serde_json::Value` map
/// into the typed `PlaceholderValue` core expects.
fn decode_placeholder_values(
    bp: &Blueprint,
    values: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<std::collections::BTreeMap<String, tpl::PlaceholderValue>, String> {
    use claudepot_core::templates::PlaceholderType as PT;
    let mut out = std::collections::BTreeMap::new();
    for ph in &bp.placeholders {
        let Some(v) = values.get(&ph.name) else {
            continue;
        };
        let pv = match ph.kind {
            PT::Path => tpl::PlaceholderValue::Path {
                value: v
                    .as_str()
                    .ok_or_else(|| format!("placeholder {} expected string path", ph.name))?
                    .to_string(),
            },
            PT::Text => tpl::PlaceholderValue::Text {
                value: v
                    .as_str()
                    .ok_or_else(|| format!("placeholder {} expected string", ph.name))?
                    .to_string(),
            },
            PT::Boolean => tpl::PlaceholderValue::Boolean {
                value: v
                    .as_bool()
                    .ok_or_else(|| format!("placeholder {} expected boolean", ph.name))?,
            },
            PT::Number => tpl::PlaceholderValue::Number {
                value: v
                    .as_f64()
                    .ok_or_else(|| format!("placeholder {} expected number", ph.name))?,
            },
            PT::List => tpl::PlaceholderValue::List {
                value: v
                    .as_array()
                    .ok_or_else(|| format!("placeholder {} expected array", ph.name))?
                    .iter()
                    .map(|x| {
                        x.as_str().map(String::from).ok_or_else(|| {
                            format!("placeholder {} list entries must be strings", ph.name)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        };
        out.insert(ph.name.clone(), pv);
    }
    Ok(out)
}

/// Templates supply `name = blueprint.name` (e.g. "Morning health
/// check"); two installs would collide on the existing-store
/// uniqueness rule. Append a numeric suffix until unique.
fn derive_unique_name(base: &str) -> Result<String, String> {
    let store =
        AutomationStore::open().map_err(|e| format!("automations store open failed: {e}"))?;
    let existing: std::collections::HashSet<String> =
        store.list().iter().map(|a| a.name.clone()).collect();
    if !existing.contains(base) {
        return Ok(base.to_string());
    }
    for n in 2..=999 {
        let candidate = format!("{base} ({n})");
        if !existing.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "failed to derive a unique automation name for {base:?}"
    ))
}
