//! Tauri commands for the Agents sidebar section.
//!
//! Thin wrappers over `claudepot_core::agent`. No business
//! logic. Outbound DTOs follow the routes pattern; inbound DTOs
//! carry only public fields (no secrets).

use chrono::Utc;
use claudepot_core::agent::{
    active_scheduler, apply_lifecycle_change, current_claudepot_cli, install_shim,
    read_run as core_read_run, resolve_binary, scheduler::cron_next_runs,
    store::agent_runs_dir, Agent, AgentBinary, AgentId, AgentPatch, AgentStore,
    CreatedVia, PlatformOptions, Trigger,
};
use claudepot_core::routes::RouteStore;
use uuid::Uuid;

use crate::dto_agents::{
    parse_output_format, parse_permission_mode, AgentCreateDto, AgentDetailsDto,
    AgentRunDto, AgentSummaryDto, AgentUpdateDto, CronValidationDto,
    NameValidationDto, SchedulerCapabilitiesDto,
};
use crate::ops::{emit_terminal, new_running_op, OpKind, RunningOps};
use tauri::{AppHandle, State};

// ---------- helpers ----------

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn open_store() -> Result<AgentStore, String> {
    AgentStore::open().map_err(|e| format!("agents store open failed: {e}"))
}

fn parse_id(s: &str) -> Result<AgentId, String> {
    Uuid::parse_str(s.trim()).map_err(|e| format!("invalid agent id: {e}"))
}

pub(crate) fn route_lookup_fn() -> impl Fn(&Uuid) -> Option<String> {
    move |id: &Uuid| -> Option<String> {
        let store = RouteStore::open().ok()?;
        // The route's `wrapper_name` is the canonical, on-disk
        // wrapper filename (the user may have supplied a custom
        // override, which `derive_wrapper_slug` does not capture).
        store
            .list()
            .iter()
            .find(|r| &r.id == id)
            .map(|r| r.wrapper_name.clone())
    }
}

fn build_agent_from_create(dto: AgentCreateDto) -> Result<Agent, String> {
    claudepot_core::agent::validate_name(&dto.name).map_err(err)?;
    let permission_mode = parse_permission_mode(&dto.permission_mode)
        .ok_or_else(|| format!("invalid permission_mode: {}", dto.permission_mode))?;
    let output_format = parse_output_format(&dto.output_format)
        .ok_or_else(|| format!("invalid output_format: {}", dto.output_format))?;
    if matches!(
        permission_mode,
        claudepot_core::agent::PermissionMode::BypassPermissions
    ) && dto.allowed_tools.is_empty()
    {
        return Err(String::from(
            "bypassPermissions requires a non-empty allowed_tools whitelist",
        ));
    }
    claudepot_core::agent::env::validate_map(&dto.extra_env).map_err(err)?;

    let binary = match dto.binary_kind.as_str() {
        "first_party" => AgentBinary::FirstParty,
        "route" => {
            let route_id = dto
                .binary_route_id
                .as_deref()
                .ok_or_else(|| String::from("route binary requires binary_route_id"))?;
            AgentBinary::Route {
                route_id: Uuid::parse_str(route_id)
                    .map_err(|e| format!("invalid route id: {e}"))?,
            }
        }
        other => return Err(format!("unknown binary_kind: {other}")),
    };

    // Determine trigger ahead of cron validation: event/manual
    // agents don't carry a cron string, so validating one would
    // either fail spuriously (empty string) or pin the trigger
    // shape unnecessarily.
    let trigger_kind = dto.trigger_kind.as_deref();
    let trigger = match trigger_kind {
        Some("event") => {
            // v1 only ships the session-settled kind. An absent
            // `event_kind` defaults to it (the form's only
            // option); an unknown value is a strong "this
            // payload was hand-crafted" signal — reject loudly
            // rather than silently treat it as session_settled.
            let event_str =
                dto.event_kind.as_deref().unwrap_or("session_settled");
            if event_str != "session_settled" {
                return Err(format!("unknown event_kind: {event_str}"));
            }
            let debounce_secs = dto
                .event_debounce_secs
                .unwrap_or(claudepot_core::agent::DEFAULT_DEBOUNCE_SECS);
            Trigger::Event {
                event: claudepot_core::agent::EventKind::SessionSettled {
                    debounce_secs,
                },
            }
        }
        Some("manual") => Trigger::Manual,
        _ => {
            // Default = "cron". Validate the cron string before
            // anything else so we fail fast on bad input.
            let _ =
                claudepot_core::agent::cron::expand(&dto.cron).map_err(err)?;
            Trigger::Cron {
                cron: dto.cron.clone(),
                timezone: dto.timezone.clone(),
            }
        }
    };

    let now = Utc::now();
    Ok(Agent {
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
        trigger,
        platform_options: PlatformOptions {
            wake_to_run: dto.platform_options.wake_to_run,
            catch_up_if_missed: dto.platform_options.catch_up_if_missed,
            run_when_logged_out: dto.platform_options.run_when_logged_out,
        },
        log_retention_runs: dto.log_retention_runs,
        created_at: now,
        updated_at: now,
        claudepot_managed: true,
        template_id: dto.template_id.clone(),
        disallowed_tools: dto.disallowed_tools,
        mcp_servers: dto.mcp_servers.into_iter().map(Into::into).collect(),
        // Empty string from the form = "run as the active account".
        run_as: dto.run_as.filter(|s| !s.is_empty()),
        // 0 from the form = "no per-run token ceiling".
        task_budget: dto.task_budget.filter(|&t| t != 0),
        // An all-null RateLimit means "no limit"; collapse it.
        rate_limit: dto.rate_limit.map(Into::into).filter(|r: &claudepot_core::agent::RateLimit| {
            r.min_interval_secs.is_some() || r.max_per_day.is_some()
        }),
        // Phase 1: the GUI Add-Agent flow both creates AND arms the
        // agent (`agents_add` registers it with the OS scheduler), so
        // a GUI-created agent is `Installed`. The `Draft` lifecycle
        // exists in the data model for the Phase-2 `agent draft` CLI
        // path; the human-only draft->install gate is also Phase 2.
        // `drafted_by` carries the audit actor when set by an AI /
        // template path.
        lifecycle: claudepot_core::agent::Lifecycle::Installed,
        drafted_by: dto.drafted_by.clone(),
        // F19 + grill X14: `created_via` is stamped by the create
        // *path*, never by the wire. The previous shape — branch on
        // `dto.template_id.is_some()` — was launderable: a
        // renderer-controlled `template_id` flipped the audit signal
        // to `Template`, but the real template-instantiation flow is
        // `agent_add_from_template`, which stamps `Template` itself.
        // This path is the GUI Add-Agent form; the truthful stamp is
        // always `Gui`. The `template_id` field is still persisted
        // (it remains independent metadata on `Agent`), but it can no
        // longer rewrite the provenance signal that the install
        // review flags against.
        created_via: CreatedVia::Gui,
    })
}

/// Build a [`AgentPatch`] from a wire DTO. `existing` lets us
/// merge `Trigger` correctly when the caller supplies only one of
/// `cron`/`timezone` (preserving the other), and lets us validate
/// the post-merge record's cross-field invariants.
fn build_patch_from_update(
    dto: AgentUpdateDto,
    existing: &Agent,
) -> Result<AgentPatch, String> {
    // Resolve every fallible / branchy field BEFORE constructing the
    // patch — keeps the struct literal below a single, scannable
    // shape and avoids the field_reassign_with_default lint that
    // accumulating `patch.x = …` after `Default::default()` triggers.
    let permission_mode = match dto.permission_mode {
        Some(s) => {
            Some(parse_permission_mode(&s).ok_or_else(|| format!("invalid permission_mode: {s}"))?)
        }
        None => None,
    };
    let max_budget_usd = match dto.max_budget_usd {
        Some(b) => {
            if !b.is_finite() || b < 0.0 {
                return Err(format!(
                    "max_budget_usd must be a finite non-negative number (got {b})"
                ));
            }
            Some(b)
        }
        None => None,
    };
    let output_format = match dto.output_format {
        Some(s) => {
            Some(parse_output_format(&s).ok_or_else(|| format!("invalid output_format: {s}"))?)
        }
        None => None,
    };
    let extra_env = match dto.extra_env {
        Some(env) => {
            claudepot_core::agent::env::validate_map(&env).map_err(err)?;
            Some(env)
        }
        None => None,
    };
    // Trigger merge: `cron` and `timezone` arrive independently; we
    // build a Cron trigger only when at least one of them changes,
    // preserving the un-supplied side from the existing record.
    let trigger = if dto.cron.is_some() || dto.timezone.is_some() {
        // Pull existing cron/tz when the existing trigger is Cron;
        // when transitioning from Manual, defaults are empty and
        // the patch must supply a usable cron string.
        let (existing_cron, existing_tz) = match &existing.trigger {
            Trigger::Cron { cron, timezone } => (cron.clone(), timezone.clone()),
            // Transitioning from Manual or Event to Cron: no
            // cron/tz to inherit, the patch must supply one.
            Trigger::Manual | Trigger::Event { .. } => (String::new(), None),
        };
        let cron = dto.cron.unwrap_or(existing_cron);
        // Validate the cron string before building the trigger.
        let _ = claudepot_core::agent::cron::expand(&cron).map_err(err)?;
        // Empty timezone string from the wire == "no timezone";
        // otherwise treat as a fresh override. Missing field == keep existing.
        let timezone = match dto.timezone {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s),
            None => existing_tz,
        };
        Some(Trigger::Cron { cron, timezone })
    } else {
        None
    };
    let platform_options = dto.platform_options.map(|po| PlatformOptions {
        wake_to_run: po.wake_to_run,
        catch_up_if_missed: po.catch_up_if_missed,
        run_when_logged_out: po.run_when_logged_out,
    });

    // Phase-1 spec fields. `mcp_servers` converts each wire ref to
    // its core form; `rate_limit` converts the wire DTO. The store's
    // `update` already collapses an all-null rate limit / a zero
    // task budget / an empty `run_as` string to `None`.
    let mcp_servers = dto
        .mcp_servers
        .map(|v| v.into_iter().map(Into::into).collect());
    let rate_limit = dto.rate_limit.map(Into::into);

    let patch = AgentPatch {
        display_name: dto.display_name,
        description: dto.description,
        enabled: dto.enabled,
        model: dto.model,
        cwd: dto.cwd,
        prompt: dto.prompt,
        system_prompt: dto.system_prompt,
        append_system_prompt: dto.append_system_prompt,
        permission_mode,
        allowed_tools: dto.allowed_tools,
        add_dir: dto.add_dir,
        max_budget_usd,
        fallback_model: dto.fallback_model,
        output_format,
        json_schema: dto.json_schema,
        bare: dto.bare,
        extra_env,
        trigger,
        platform_options,
        log_retention_runs: dto.log_retention_runs,
        disallowed_tools: dto.disallowed_tools,
        mcp_servers,
        run_as: dto.run_as,
        task_budget: dto.task_budget,
        rate_limit,
    };

    // Cross-field invariant: bypassPermissions + non-empty
    // allowed_tools. Compute the post-merge state and reject
    // unsafe combinations early.
    let post_mode = patch.permission_mode.unwrap_or(existing.permission_mode);
    let post_tools_empty = match &patch.allowed_tools {
        Some(v) => v.is_empty(),
        None => existing.allowed_tools.is_empty(),
    };
    if matches!(
        post_mode,
        claudepot_core::agent::PermissionMode::BypassPermissions
    ) && post_tools_empty
    {
        return Err(String::from(
            "bypassPermissions requires a non-empty allowed_tools whitelist",
        ));
    }
    Ok(patch)
}

// ---------- commands ----------

/// F21: instantiate a built-in agent template as a fresh draft.
///
/// v1 ships exactly one template — the **Session Narrator** — so
/// this command takes a single `template_id` string and dispatches
/// on it. The resulting record is added to the store as `Draft`
/// (the template constructor stamps `Lifecycle::Draft` and
/// `created_via = Template`); the human reviews and arms it via
/// the existing `agent_install` flow. Catalog growth is explicit
/// v2 (PRD §13), so the dispatch is a `match` rather than a
/// registry — additions are visible and a new arm forces a code
/// change.
#[tauri::command]
pub async fn agent_add_from_template(
    template_id: String,
    cwd: String,
) -> Result<AgentSummaryDto, String> {
    let agent = match template_id.as_str() {
        "session-narrator" => {
            // `session_narrator` is a pure constructor; the cwd is
            // the project the narrator watches (event scope rule).
            claudepot_core::agent::templates::session_narrator(
                &cwd,
                Utc::now(),
            )
        }
        other => return Err(format!("unknown template id: {other}")),
    };

    // Re-validate at the store boundary — `cwd` shape, name shape,
    // numeric bounds — so a template that drifts past the rules
    // surfaces here, not as a later failure during install.
    let mut store = open_store()?;
    if store.get_by_name(&agent.name).is_some() {
        return Err(format!(
            "an agent named '{}' already exists — rename or remove \
             it before instantiating this template again",
            agent.name
        ));
    }
    let summary = AgentSummaryDto::from(&agent);
    store.add(agent).map_err(err)?;
    store.save().map_err(err)?;
    Ok(summary)
}

#[tauri::command]
pub async fn agents_list() -> Result<Vec<AgentSummaryDto>, String> {
    let store = open_store()?;
    Ok(store
        .list()
        .iter()
        .map(AgentSummaryDto::from)
        .collect())
}

#[tauri::command]
pub async fn agents_get(id: String) -> Result<AgentDetailsDto, String> {
    let store = open_store()?;
    let id = parse_id(&id)?;
    let a = store
        .get(&id)
        .ok_or_else(|| format!("agent {id} not found"))?;
    Ok(AgentDetailsDto::from(a))
}

#[tauri::command]
pub async fn agents_add(dto: AgentCreateDto) -> Result<AgentSummaryDto, String> {
    let mut store = open_store()?;
    if store.get_by_name(&dto.name).is_some() {
        return Err(format!("agent name '{}' is already taken", dto.name));
    }
    let agent = build_agent_from_create(dto)?;
    let id = agent.id;

    // Resolve binary + CLI path ahead of the helper so a missing
    // binary surfaces *before* we touch the store. (The helper's
    // `install_shim` closure also surfaces it, but doing this lookup
    // first matches the previous behavior — and `resolve_binary` is
    // a pure path lookup with no side effects.)
    let cli_path = current_claudepot_cli().map_err(err)?;
    let lookup = route_lookup_fn();
    let scheduler = active_scheduler();

    // grill X2: route the add → shim → save → register sequence and
    // its full rollback matrix through the shared helper in
    // `install_gate`. `agents_add` previously had its own
    // bespoke ordering (shim before save, then register-with-
    // store-remove rollback); the helper enforces a single,
    // tested shape for every enabling verb.
    let inserted = apply_lifecycle_change(
        &mut store,
        &id,
        // `mutate`: insert the record.
        move |store| {
            store.add(agent.clone())?;
            store
                .get(&id)
                .cloned()
                .ok_or_else(|| {
                    claudepot_core::agent::AgentError::NotFound(id.to_string())
                })
        },
        // `rollback`: drop the just-inserted record.
        |store| {
            let _ = store.remove(&id);
        },
        // `install_shim`: the real disk-touching render. The helper
        // skips it for a disabled record.
        |a| {
            let binary_path = resolve_binary(a, &lookup)?;
            install_shim(a, &binary_path, &cli_path).map(|_| ())
        },
        scheduler.as_ref(),
    )
    .map_err(err)?;

    Ok(AgentSummaryDto::from(&inserted))
}

#[tauri::command]
pub async fn agents_update(dto: AgentUpdateDto) -> Result<AgentSummaryDto, String> {
    let mut store = open_store()?;
    let id = parse_id(&dto.id)?;
    // Snapshot the existing record so the patch builder can merge
    // partial trigger fields (cron without timezone, etc.), so we
    // can validate cross-field invariants against the post-merge
    // state, and so the helper's rollback can restore it.
    let existing = store
        .get(&id)
        .ok_or_else(|| format!("agent {id} not found"))?
        .clone();
    let patch = build_patch_from_update(dto, &existing)?;

    let cli_path = current_claudepot_cli().map_err(err)?;
    let lookup = route_lookup_fn();
    let scheduler = active_scheduler();

    // grill X2: previously, `agents_update` ran patch → save →
    // unregister → shim → register with **no rollback** — a failed
    // register left the patched record on disk *and* a stale or
    // missing artifact (the F10 orphan, reintroduced). The helper
    // owns the full rollback matrix; this verb just supplies the
    // patch as the mutation and the prior record as the rollback.
    let rollback_record = existing.clone();
    let updated = apply_lifecycle_change(
        &mut store,
        &id,
        move |store| {
            store.update(&id, patch)?;
            store
                .get(&id)
                .cloned()
                .ok_or_else(|| {
                    claudepot_core::agent::AgentError::NotFound(id.to_string())
                })
        },
        // Rollback: restore the pre-update record verbatim (drop the
        // patched record and re-insert the prior). `set_lifecycle` is
        // not enough — patch could have changed any field. Using
        // `remove` + direct in-memory restore via the store's normal
        // path keeps the rollback in-memory only; the helper decides
        // whether to re-save.
        move |store| {
            let _ = store.remove(&id);
            // Re-add the prior record. `add` re-runs validators —
            // they passed when the prior record was first inserted,
            // so they will pass again. A failure here is logged but
            // we cannot meaningfully propagate it from a rollback
            // closure; the helper's caller already has an Err.
            if let Err(e) = store.add(rollback_record) {
                tracing::error!(
                    agent_id = %id,
                    error = %e,
                    "agents_update rollback: re-adding the prior record \
                     failed; in-memory store may be inconsistent until \
                     the next open"
                );
            }
        },
        |a| {
            let binary_path = resolve_binary(a, &lookup)?;
            install_shim(a, &binary_path, &cli_path).map(|_| ())
        },
        scheduler.as_ref(),
    )
    .map_err(err)?;

    Ok(AgentSummaryDto::from(&updated))
}

/// Arm a draft agent: flip `lifecycle` to `Installed` and
/// materialize the scheduler artifact. This is the **human-only**
/// half of the Phase-2 draft/install gate (PRD §8.2 / D8) — the CLI
/// deliberately has no `install` verb, so an AI client that drafted
/// an agent can never arm it; only this GUI-invoked command can.
///
/// Thin wrapper over `claudepot_core::agent::install_draft` — the
/// pure, unit-tested gate engine. This command only resolves the
/// binary path, builds the real disk-touching shim-install closure,
/// and picks the OS scheduler; the arm → install-shim → register →
/// save ordering and both rollback directions live in core and are
/// covered by `install_gate`'s tests.
#[tauri::command]
pub async fn agent_install(id: String) -> Result<AgentSummaryDto, String> {
    let mut store = open_store()?;
    let aid = parse_id(&id)?;

    let cli_path = current_claudepot_cli().map_err(err)?;
    let lookup = route_lookup_fn();
    let scheduler = active_scheduler();

    let outcome = claudepot_core::agent::install_draft(
        &mut store,
        &aid,
        scheduler.as_ref(),
        |agent| {
            let binary_path = resolve_binary(agent, &lookup)?;
            install_shim(agent, &binary_path, &cli_path).map(|_| ())
        },
    )
    .map_err(err)?;

    Ok(AgentSummaryDto::from(&outcome.agent))
}

#[tauri::command]
pub async fn agents_remove(id: String) -> Result<(), String> {
    let mut store = open_store()?;
    let aid = parse_id(&id)?;
    let _ = store.remove(&aid).map_err(err)?;
    // Persist FIRST so the JSON store and OS scheduler can never
    // diverge if a later step fails. Even if scheduler unregister
    // errors, the store no longer points at the dropped record.
    store.save().map_err(err)?;
    let scheduler = active_scheduler();
    let _ = scheduler.unregister(&aid);
    // Best-effort cleanup of the on-disk per-agent dir.
    let auto_dir = claudepot_core::agent::agent_dir(&aid);
    if auto_dir.exists() {
        let _ = std::fs::remove_dir_all(&auto_dir);
    }
    Ok(())
}

#[tauri::command]
pub async fn agents_set_enabled(id: String, enabled: bool) -> Result<(), String> {
    let mut store = open_store()?;
    let aid = parse_id(&id)?;

    // Snapshot the existing record so the rollback can restore it
    // if a later step (shim render, save, register) fails. The
    // helper's X1 gate rejects a Draft + enabled=true combination
    // before any artifact is materialized; see install_gate.rs.
    let existing = store
        .get(&aid)
        .ok_or_else(|| format!("agent {aid} not found"))?
        .clone();
    let rollback_enabled = existing.enabled;

    let cli_path = current_claudepot_cli().map_err(err)?;
    let lookup = route_lookup_fn();
    let scheduler = active_scheduler();

    // grill X2: previously this verb ran update → save → register
    // with **no rollback** — the F10 orphan reintroduced. The
    // helper owns the rollback matrix. The X1 Draft-rejection
    // (a Draft must not acquire a scheduler artifact via the
    // enabled toggle) is enforced inside the helper for every
    // verb that materializes an artifact.
    apply_lifecycle_change(
        &mut store,
        &aid,
        move |store| {
            let patch = AgentPatch {
                enabled: Some(enabled),
                ..AgentPatch::default()
            };
            store.update(&aid, patch)?;
            store
                .get(&aid)
                .cloned()
                .ok_or_else(|| {
                    claudepot_core::agent::AgentError::NotFound(aid.to_string())
                })
        },
        move |store| {
            let patch = AgentPatch {
                enabled: Some(rollback_enabled),
                ..AgentPatch::default()
            };
            if let Err(e) = store.update(&aid, patch) {
                tracing::error!(
                    agent_id = %aid,
                    error = %e,
                    "agents_set_enabled rollback: restoring the prior \
                     `enabled` flag failed"
                );
            }
        },
        |a| {
            let binary_path = resolve_binary(a, &lookup)?;
            install_shim(a, &binary_path, &cli_path).map(|_| ())
        },
        scheduler.as_ref(),
    )
    .map_err(err)?;

    Ok(())
}

#[tauri::command]
pub async fn agents_run_now_start(
    id: String,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let aid = parse_id(&id)?;

    // grill X17: fast-path the Draft rejection without the
    // exclusive store lock. `lifecycle_of` does a non-blocking
    // `try_open` of the agents file; on a free lock it reads the
    // record's lifecycle and drops the lock immediately. A draft
    // never reaches Run-Now (see the matches!() check below), and
    // failing here means we never block on a GUI mid-install just
    // to reject a draft. Under contention the lookup falls through
    // (returns `Ok(None)`) and the gate runs again under the full
    // open below — same behavior as before, but at most one of the
    // two paths actually serializes.
    if let Ok(Some(claudepot_core::agent::Lifecycle::Draft)) =
        claudepot_core::agent::AgentStore::lifecycle_of(&aid)
    {
        // Pull the agent name for the error message via the full
        // open. The contended case is rare; the message format is
        // the same as the original (post-lock) path so callers see
        // identical text either way.
        let store = open_store()?;
        let name = store
            .get(&aid)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| aid.to_string());
        return Err(format!(
            "agent '{name}' is a draft — review and install it before running. \
             A draft is never executed directly; arming it through the \
             install review is the gate."
        ));
    }

    // Load the agent now so we fail fast on missing.
    let store = open_store()?;
    let agent = store
        .get(&aid)
        .ok_or_else(|| format!("agent {aid} not found"))?
        .clone();

    // grill F16: Run-Now must NOT execute a `Draft`. A draft has
    // never passed the human install review, yet Run-Now spawns
    // `claude -p` with its `--mcp-config`, `cwd`, and
    // `permission_mode` — running unreviewed config is exactly what
    // the draft/install gate exists to prevent. The GUI already
    // hides "Run now" on a draft card (it shows only "Review &
    // install"); this is the backend enforcement of that contract.
    // A draft is exercised by arming it through the install review,
    // not by Run-Now. Defense-in-depth: the X17 fast path above
    // catches the uncontended case without the lock; this check
    // remains the authoritative gate for the contended-fallback
    // path (and for any future caller that builds `agent` without
    // going through the X17 prelude).
    if matches!(
        agent.lifecycle,
        claudepot_core::agent::Lifecycle::Draft
    ) {
        return Err(format!(
            "agent '{}' is a draft — review and install it before running. \
             A draft is never executed directly; arming it through the \
             install review is the gate.",
            agent.name
        ));
    }

    let op_id = format!("agent-run-{}", Uuid::new_v4());
    ops.insert(new_running_op(
        &op_id,
        OpKind::AgentRun,
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
                let binary_path =
                    resolve_binary(&agent, &route_lookup_fn()).map_err(|e| e.to_string())?;
                let cli_path = current_claudepot_cli().map_err(|e| e.to_string())?;
                // Manual Run-Now never injects event env vars; the
                // event orchestrator passes a populated map to this
                // function only for `session-settled` dispatches.
                let empty_env = std::collections::BTreeMap::<String, String>::new();
                claudepot_core::agent::run_now(
                    &agent,
                    &binary_path,
                    &cli_path,
                    &sink,
                    &empty_env,
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
pub async fn agents_runs_list(
    id: String,
    limit: Option<usize>,
) -> Result<Vec<AgentRunDto>, String> {
    let aid = parse_id(&id)?;
    let cap = limit.unwrap_or(50);
    let runs_dir = agent_runs_dir(&aid);
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
            if let Ok(run) =
                serde_json::from_slice::<claudepot_core::agent::AgentRun>(&raw)
            {
                out.push(AgentRunDto::from(run));
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn agents_run_get(id: String, run_id: String) -> Result<AgentRunDto, String> {
    let aid = parse_id(&id)?;
    let run = core_read_run(&aid, &run_id).map_err(err)?;
    Ok(AgentRunDto::from(run))
}

#[tauri::command]
pub async fn agents_validate_name(name: String) -> Result<NameValidationDto, String> {
    let mut already_taken = false;
    let validation = match claudepot_core::agent::validate_name(&name) {
        Ok(_) => {
            if let Ok(store) = AgentStore::open() {
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
pub async fn agents_validate_cron(expr: String) -> Result<CronValidationDto, String> {
    match claudepot_core::agent::cron::expand(&expr) {
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
pub async fn agents_scheduler_capabilities() -> Result<SchedulerCapabilitiesDto, String> {
    let scheduler = active_scheduler();
    Ok(SchedulerCapabilitiesDto::from(scheduler.capabilities()))
}

#[tauri::command]
pub async fn agents_dry_run_artifact(id: String) -> Result<String, String> {
    let store = open_store()?;
    let aid = parse_id(&id)?;
    let agent = store
        .get(&aid)
        .ok_or_else(|| format!("agent {aid} not found"))?;

    #[cfg(target_os = "macos")]
    {
        claudepot_core::agent::scheduler::launchd::render_plist(agent).map_err(err)
    }
    #[cfg(target_os = "linux")]
    {
        let (timer, service) =
            claudepot_core::agent::scheduler::systemd::render_units(agent)
                .map_err(err)?;
        return Ok(format!(
            "# {} ===== timer ======\n{}\n# ===== service =====\n{}",
            agent.id, timer, service
        ));
    }
    #[cfg(target_os = "windows")]
    {
        return claudepot_core::agent::scheduler::schtasks::render_xml(agent)
            .map_err(err);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = agent;
        Err(String::from("no scheduler adapter for this platform"))
    }
}

#[tauri::command]
pub async fn agents_open_artifact_dir() -> Result<(), String> {
    let scheduler = active_scheduler();
    let dir = scheduler
        .capabilities()
        .artifact_dir
        .ok_or_else(|| String::from("no artifact dir for active scheduler"))?;
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&dir).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &dir])
            .status()
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
pub async fn agents_linger_status() -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        return claudepot_core::agent::scheduler::systemd::linger_status().map_err(err);
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(false)
    }
}

#[tauri::command]
pub async fn agents_linger_enable() -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto_agents::PlatformOptionsDto;

    /// grill X14: the GUI Add-Agent create path stamps
    /// `created_via = Gui` regardless of what the renderer puts in
    /// `template_id`. The real template path is
    /// `agent_add_from_template`, which stamps `Template` itself; a
    /// renderer can no longer launder the provenance signal through
    /// this verb's DTO.
    #[test]
    fn build_agent_from_create_always_stamps_gui_even_with_template_id() {
        let dto = AgentCreateDto {
            name: "x14-canary".into(),
            display_name: None,
            description: None,
            binary_kind: "first_party".into(),
            binary_route_id: None,
            model: Some("sonnet".into()),
            cwd: "/tmp".into(),
            prompt: "hi".into(),
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: "default".into(),
            allowed_tools: vec!["Read".into()],
            add_dir: vec![],
            max_budget_usd: None,
            fallback_model: None,
            output_format: "json".into(),
            json_schema: None,
            bare: false,
            extra_env: Default::default(),
            trigger_kind: Some("manual".into()),
            cron: String::new(),
            timezone: None,
            event_kind: None,
            event_debounce_secs: None,
            platform_options: PlatformOptionsDto {
                wake_to_run: false,
                catch_up_if_missed: false,
                run_when_logged_out: false,
            },
            log_retention_runs: 50,
            // A renderer-supplied template id MUST NOT flip the
            // provenance stamp — the GUI Add form has no template
            // flow; only `agent_add_from_template` does.
            template_id: Some("session-narrator".into()),
            disallowed_tools: vec![],
            mcp_servers: vec![],
            run_as: None,
            task_budget: None,
            rate_limit: None,
            drafted_by: None,
        };
        let agent = build_agent_from_create(dto).expect("build_agent_from_create");
        assert!(
            matches!(agent.created_via, CreatedVia::Gui),
            "X14: agents_add must always stamp Gui, got {:?}",
            agent.created_via
        );
        // `template_id` is retained as independent metadata — the
        // path doesn't strip it, it just doesn't let it rewrite the
        // provenance signal.
        assert_eq!(agent.template_id.as_deref(), Some("session-narrator"));
    }
}
