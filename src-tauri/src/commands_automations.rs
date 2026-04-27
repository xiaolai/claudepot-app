//! Tauri commands for the Automations sidebar section.
//!
//! Thin wrappers over `claudepot_core::automations`. No business
//! logic. Outbound DTOs follow the routes pattern; inbound DTOs
//! carry only public fields (no secrets).

use chrono::Utc;
use claudepot_core::automations::{
    active_scheduler, current_claudepot_cli, install_shim, read_run as core_read_run,
    resolve_binary, scheduler::cron_next_runs, store::automation_runs_dir, Automation,
    AutomationBinary, AutomationId, AutomationPatch, AutomationStore, PlatformOptions,
    Trigger,
};
use claudepot_core::routes::RouteStore;
use uuid::Uuid;

use crate::dto_automations::{
    parse_output_format, parse_permission_mode, AutomationCreateDto, AutomationDetailsDto,
    AutomationRunDto, AutomationSummaryDto, AutomationUpdateDto, CronValidationDto,
    NameValidationDto, SchedulerCapabilitiesDto,
};
use crate::ops::{emit_terminal, new_running_op, OpKind, RunningOps};
use tauri::{AppHandle, State};

// ---------- helpers ----------

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn open_store() -> Result<AutomationStore, String> {
    AutomationStore::open().map_err(|e| format!("automations store open failed: {e}"))
}

fn parse_id(s: &str) -> Result<AutomationId, String> {
    Uuid::parse_str(s.trim()).map_err(|e| format!("invalid automation id: {e}"))
}

fn route_lookup_fn() -> impl Fn(&Uuid) -> Option<String> {
    move |id: &Uuid| -> Option<String> {
        let store = RouteStore::open().ok()?;
        store
            .list()
            .iter()
            .find(|r| &r.id == id)
            .map(|r| {
                // Wrapper name is derived from route name + provider; we
                // store it explicitly on the route. RouteSummaryDto's
                // wrapper_name is the cached form.
                claudepot_core::routes::derive_wrapper_slug(&r.name)
                    .trim_start_matches("claude-")
                    .to_string()
            })
            .map(|slug| format!("claude-{slug}"))
    }
}

fn build_automation_from_create(
    dto: AutomationCreateDto,
) -> Result<Automation, String> {
    claudepot_core::automations::validate_name(&dto.name).map_err(err)?;
    let permission_mode = parse_permission_mode(&dto.permission_mode)
        .ok_or_else(|| format!("invalid permission_mode: {}", dto.permission_mode))?;
    let output_format = parse_output_format(&dto.output_format)
        .ok_or_else(|| format!("invalid output_format: {}", dto.output_format))?;
    if matches!(
        permission_mode,
        claudepot_core::automations::PermissionMode::BypassPermissions
    ) && dto.allowed_tools.is_empty()
    {
        return Err(String::from(
            "bypassPermissions requires a non-empty allowed_tools whitelist",
        ));
    }
    claudepot_core::automations::env::validate_map(&dto.extra_env).map_err(err)?;

    let binary = match dto.binary_kind.as_str() {
        "first_party" => AutomationBinary::FirstParty,
        "route" => {
            let route_id = dto
                .binary_route_id
                .as_deref()
                .ok_or_else(|| String::from("route binary requires binary_route_id"))?;
            AutomationBinary::Route {
                route_id: Uuid::parse_str(route_id).map_err(|e| format!("invalid route id: {e}"))?,
            }
        }
        other => return Err(format!("unknown binary_kind: {other}")),
    };

    // Validate cron now so we fail fast.
    let _ = claudepot_core::automations::cron::expand(&dto.cron).map_err(err)?;

    let now = Utc::now();
    Ok(Automation {
        id: Uuid::new_v4(),
        name: dto.name,
        display_name: dto.display_name,
        description: dto.description,
        enabled: true,
        binary,
        model: dto.model,
        cwd: dto.cwd,
        prompt: dto.prompt,
        system_prompt: dto.system_prompt,
        append_system_prompt: dto.append_system_prompt,
        permission_mode,
        allowed_tools: dto.allowed_tools,
        add_dir: dto.add_dir,
        max_budget_usd: dto.max_budget_usd,
        fallback_model: dto.fallback_model,
        output_format,
        json_schema: dto.json_schema,
        bare: dto.bare,
        extra_env: dto.extra_env,
        trigger: Trigger::Cron {
            cron: dto.cron,
            timezone: dto.timezone,
        },
        platform_options: PlatformOptions {
            wake_to_run: dto.platform_options.wake_to_run,
            catch_up_if_missed: dto.platform_options.catch_up_if_missed,
            run_when_logged_out: dto.platform_options.run_when_logged_out,
        },
        log_retention_runs: dto.log_retention_runs,
        created_at: now,
        updated_at: now,
        claudepot_managed: true,
    })
}

fn build_patch_from_update(
    dto: AutomationUpdateDto,
) -> Result<AutomationPatch, String> {
    let mut patch = AutomationPatch::default();
    patch.display_name = dto.display_name;
    patch.description = dto.description;
    patch.enabled = dto.enabled;
    patch.model = dto.model;
    patch.cwd = dto.cwd;
    patch.prompt = dto.prompt;
    patch.system_prompt = dto.system_prompt;
    patch.append_system_prompt = dto.append_system_prompt;
    if let Some(s) = dto.permission_mode {
        let pm = parse_permission_mode(&s)
            .ok_or_else(|| format!("invalid permission_mode: {s}"))?;
        patch.permission_mode = Some(pm);
    }
    patch.allowed_tools = dto.allowed_tools;
    patch.add_dir = dto.add_dir;
    patch.max_budget_usd = dto.max_budget_usd;
    patch.fallback_model = dto.fallback_model;
    if let Some(s) = dto.output_format {
        let of = parse_output_format(&s).ok_or_else(|| format!("invalid output_format: {s}"))?;
        patch.output_format = Some(of);
    }
    patch.json_schema = dto.json_schema;
    patch.bare = dto.bare;
    if let Some(env) = dto.extra_env {
        claudepot_core::automations::env::validate_map(&env).map_err(err)?;
        patch.extra_env = Some(env);
    }
    if dto.cron.is_some() || dto.timezone.is_some() {
        // Build a fresh cron trigger; we need at least the cron string
        // to validate.
        if let Some(c) = &dto.cron {
            let _ = claudepot_core::automations::cron::expand(c).map_err(err)?;
        }
        // Default tz handling: if cron supplied alone, keep null tz.
        let cron = dto.cron.unwrap_or_default();
        let timezone = dto.timezone.unwrap_or(None);
        if !cron.is_empty() {
            patch.trigger = Some(Trigger::Cron { cron, timezone });
        }
    }
    if let Some(po) = dto.platform_options {
        patch.platform_options = Some(PlatformOptions {
            wake_to_run: po.wake_to_run,
            catch_up_if_missed: po.catch_up_if_missed,
            run_when_logged_out: po.run_when_logged_out,
        });
    }
    patch.log_retention_runs = dto.log_retention_runs;
    Ok(patch)
}

// ---------- commands ----------

#[tauri::command]
pub async fn automations_list() -> Result<Vec<AutomationSummaryDto>, String> {
    let store = open_store()?;
    Ok(store
        .list()
        .iter()
        .map(AutomationSummaryDto::from)
        .collect())
}

#[tauri::command]
pub async fn automations_get(id: String) -> Result<AutomationDetailsDto, String> {
    let store = open_store()?;
    let id = parse_id(&id)?;
    let a = store
        .get(&id)
        .ok_or_else(|| format!("automation {id} not found"))?;
    Ok(AutomationDetailsDto::from(a))
}

#[tauri::command]
pub async fn automations_add(
    dto: AutomationCreateDto,
) -> Result<AutomationSummaryDto, String> {
    let mut store = open_store()?;
    if store.get_by_name(&dto.name).is_some() {
        return Err(format!("automation name '{}' is already taken", dto.name));
    }
    let automation = build_automation_from_create(dto)?;

    // Resolve binary + install shim before scheduler register so a
    // failed render unwinds cleanly.
    let binary_path = resolve_binary(&automation, &route_lookup_fn()).map_err(err)?;
    let cli_path = current_claudepot_cli().map_err(err)?;
    install_shim(&automation, &binary_path, &cli_path).map_err(err)?;

    let scheduler = active_scheduler();
    if automation.enabled {
        scheduler.register(&automation).map_err(err)?;
    }

    let summary = AutomationSummaryDto::from(&automation);
    store.add(automation).map_err(err)?;
    store.save().map_err(err)?;
    Ok(summary)
}

#[tauri::command]
pub async fn automations_update(
    dto: AutomationUpdateDto,
) -> Result<AutomationSummaryDto, String> {
    let mut store = open_store()?;
    let id = parse_id(&dto.id)?;
    let patch = build_patch_from_update(dto)?;
    store.update(&id, patch).map_err(err)?;

    // Pull the updated record so we can re-render shim + plist.
    let updated = store
        .get(&id)
        .ok_or_else(|| format!("automation {id} not found after update"))?
        .clone();
    let binary_path = resolve_binary(&updated, &route_lookup_fn()).map_err(err)?;
    let cli_path = current_claudepot_cli().map_err(err)?;
    install_shim(&updated, &binary_path, &cli_path).map_err(err)?;

    let scheduler = active_scheduler();
    let _ = scheduler.unregister(&id);
    if updated.enabled {
        scheduler.register(&updated).map_err(err)?;
    }

    store.save().map_err(err)?;
    Ok(AutomationSummaryDto::from(&updated))
}

#[tauri::command]
pub async fn automations_remove(id: String) -> Result<(), String> {
    let mut store = open_store()?;
    let aid = parse_id(&id)?;
    let _ = store.remove(&aid).map_err(err)?;
    let scheduler = active_scheduler();
    let _ = scheduler.unregister(&aid);
    // Best-effort cleanup of the on-disk per-automation dir.
    let auto_dir = claudepot_core::automations::automation_dir(&aid);
    if auto_dir.exists() {
        let _ = std::fs::remove_dir_all(&auto_dir);
    }
    store.save().map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn automations_set_enabled(
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut store = open_store()?;
    let aid = parse_id(&id)?;
    let mut patch = AutomationPatch::default();
    patch.enabled = Some(enabled);
    store.update(&aid, patch).map_err(err)?;

    let scheduler = active_scheduler();
    if enabled {
        let updated = store
            .get(&aid)
            .ok_or_else(|| format!("automation {aid} not found"))?
            .clone();
        let binary_path = resolve_binary(&updated, &route_lookup_fn()).map_err(err)?;
        let cli_path = current_claudepot_cli().map_err(err)?;
        install_shim(&updated, &binary_path, &cli_path).map_err(err)?;
        scheduler.register(&updated).map_err(err)?;
    } else {
        let _ = scheduler.unregister(&aid);
    }

    store.save().map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn automations_run_now_start(
    id: String,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let aid = parse_id(&id)?;
    // Load the automation now so we fail fast on missing.
    let store = open_store()?;
    let automation = store
        .get(&aid)
        .ok_or_else(|| format!("automation {aid} not found"))?
        .clone();

    let op_id = format!("automation-run-{}", Uuid::new_v4());
    ops.insert(new_running_op(
        &op_id,
        OpKind::AutomationRun,
        aid.to_string(),
        String::new(),
    ));

    let ops_for_task = ops.inner().clone();
    let app_for_task = app.clone();
    let op_id_clone = op_id.clone();

    crate::ops::spawn_op_thread(
        app_for_task,
        ops_for_task,
        op_id_clone,
        move |sink, app, ops, op_id| {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    emit_terminal(&app, &ops, &op_id, Some(format!("runtime: {e}")));
                    return;
                }
            };
            let result = rt.block_on(async move {
                let binary_path = resolve_binary(&automation, &route_lookup_fn())
                    .map_err(|e| e.to_string())?;
                let cli_path = current_claudepot_cli().map_err(|e| e.to_string())?;
                claudepot_core::automations::run_now(
                    &automation,
                    &binary_path,
                    &cli_path,
                    &sink,
                )
                .await
                .map_err(|e| e.to_string())
            });
            match result {
                Ok(_run) => emit_terminal(&app, &ops, &op_id, None),
                Err(e) => emit_terminal(&app, &ops, &op_id, Some(e)),
            }
        },
    );

    Ok(op_id)
}

#[tauri::command]
pub async fn automations_runs_list(
    id: String,
    limit: Option<usize>,
) -> Result<Vec<AutomationRunDto>, String> {
    let aid = parse_id(&id)?;
    let cap = limit.unwrap_or(50);
    let runs_dir = automation_runs_dir(&aid);
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(&runs_dir)
        .map_err(err)?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names.reverse();

    let mut out = Vec::with_capacity(cap.min(names.len()));
    for name in names.into_iter().take(cap) {
        let result_path = runs_dir.join(&name).join("result.json");
        if let Ok(raw) = std::fs::read(&result_path) {
            if let Ok(run) = serde_json::from_slice::<
                claudepot_core::automations::AutomationRun,
            >(&raw)
            {
                out.push(AutomationRunDto::from(run));
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn automations_run_get(
    id: String,
    run_id: String,
) -> Result<AutomationRunDto, String> {
    let aid = parse_id(&id)?;
    let run = core_read_run(&aid, &run_id).map_err(err)?;
    Ok(AutomationRunDto::from(run))
}

#[tauri::command]
pub async fn automations_validate_name(
    name: String,
) -> Result<NameValidationDto, String> {
    let mut already_taken = false;
    let validation = match claudepot_core::automations::validate_name(&name) {
        Ok(_) => {
            if let Ok(store) = AutomationStore::open() {
                if store.get_by_name(name.trim()).is_some() {
                    already_taken = true;
                }
            }
            NameValidationDto {
                valid: !already_taken,
                error: if already_taken {
                    Some(format!("name '{}' is already taken", name.trim()))
                } else {
                    None
                },
                already_taken,
            }
        }
        Err(e) => NameValidationDto {
            valid: false,
            error: Some(e.to_string()),
            already_taken: false,
        },
    };
    Ok(validation)
}

#[tauri::command]
pub async fn automations_validate_cron(
    expr: String,
) -> Result<CronValidationDto, String> {
    match claudepot_core::automations::cron::expand(&expr) {
        Ok(_) => {
            let from = Utc::now();
            let next = cron_next_runs(&expr, from, 5).unwrap_or_default();
            Ok(CronValidationDto {
                valid: true,
                error: None,
                next_runs: next.into_iter().map(|t| t.to_rfc3339()).collect(),
            })
        }
        Err(e) => Ok(CronValidationDto {
            valid: false,
            error: Some(e.to_string()),
            next_runs: vec![],
        }),
    }
}

#[tauri::command]
pub async fn automations_scheduler_capabilities(
) -> Result<SchedulerCapabilitiesDto, String> {
    let scheduler = active_scheduler();
    Ok(SchedulerCapabilitiesDto::from(scheduler.capabilities()))
}

#[tauri::command]
pub async fn automations_dry_run_artifact(id: String) -> Result<String, String> {
    let store = open_store()?;
    let aid = parse_id(&id)?;
    let automation = store
        .get(&aid)
        .ok_or_else(|| format!("automation {aid} not found"))?;

    #[cfg(target_os = "macos")]
    {
        return claudepot_core::automations::scheduler::launchd::render_plist(automation)
            .map_err(err);
    }
    #[cfg(target_os = "linux")]
    {
        let (timer, service) =
            claudepot_core::automations::scheduler::systemd::render_units(automation)
                .map_err(err)?;
        return Ok(format!(
            "# {} ===== timer ======\n{}\n# ===== service =====\n{}",
            automation.id, timer, service
        ));
    }
    #[cfg(target_os = "windows")]
    {
        return claudepot_core::automations::scheduler::schtasks::render_xml(automation)
            .map_err(err);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = automation;
        Err(String::from("no scheduler adapter for this platform"))
    }
}

#[tauri::command]
pub async fn automations_open_artifact_dir() -> Result<(), String> {
    let scheduler = active_scheduler();
    let dir = scheduler
        .capabilities()
        .artifact_dir
        .ok_or_else(|| String::from("no artifact dir for active scheduler"))?;
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&dir).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", &dir]).status()
    } else {
        std::process::Command::new("xdg-open").arg(&dir).status()
    };
    let s = status.map_err(err)?;
    if !s.success() {
        return Err(format!("open '{dir}' exited {s}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn automations_linger_status() -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        return claudepot_core::automations::scheduler::systemd::linger_status()
            .map_err(err);
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(false)
    }
}

#[tauri::command]
pub async fn automations_linger_enable() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let user = std::env::var("USER").map_err(|e| e.to_string())?;
        let status = std::process::Command::new("loginctl")
            .args(["enable-linger", &user])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("loginctl enable-linger exited {status}"));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(String::from("linger is a Linux-only feature"))
    }
}

