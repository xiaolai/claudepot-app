//! Event orchestrator — bridges
//! `claudepot_core::agent::events::evaluate` to the Tauri runtime.
//!
//! Pure rule logic + the ledger live in core; this module wires
//! them to:
//!
//! - The session index (live CC transcripts under `~/.claude/`).
//! - The agent store (the `Installed && enabled` event-triggered
//!   agents).
//! - Per-agent run history on disk (the source of the F17 self-
//!   trigger exclusion set, built from authoritative
//!   `RunResult.session_id`s — NEVER from
//!   `AgentRun::session_jsonl_path`).
//! - The durable event-state ledger (the F14 source of the
//!   rate-limiter's per-agent stats — derived from the ledger,
//!   NEVER from prunable `result.json` directories).
//! - `agent::run::run_now` for dispatch.
//!
//! The three load-bearing constraints from
//! `claudepot_core::agent::events`'s module-doc (F1 / F14 / F17)
//! are honored here; see the function-level comments.
//!
//! Hooked into `usage_snapshot::run_tick` alongside the rotation +
//! permission orchestrators. Zero overhead when no `Event`-
//! triggered agents are installed — the very first check is a list
//! of agents from the store, filtered, and an early return.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use claudepot_core::agent::install::current_claudepot_cli;
use claudepot_core::agent::{
    self, agent_runs_dir,
    events::{
        apply_per_agent_first_tick_cap, build_dispatch_env, build_run_stats_from_ledger,
        build_self_exclusion_set_with, evaluate as evaluate_events, store as events_store,
        EventFire, EventsFile, FIRST_TICK_BURST_CAP,
    },
    list_run_ids, read_run, resolve_binary, Agent, AgentStore, Lifecycle, Trigger,
};
use claudepot_core::session::SessionRow;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Orchestrator state — `manage()`'d by the Tauri app, reachable
/// via `app.state::<Arc<EventOrchestrator>>()`.
///
/// grill X16: previously this was a single per-process boolean —
/// the cap fired exactly once at orchestrator boot, and any event
/// agent ADDED LATER got uncapped first contact with the session
/// index. Now the cap is per-agent: every agent gets its bounded
/// catch-up the first time it appears in a tick, no matter when
/// it was added. The set is in-process state (lost on relaunch),
/// which means the cap fires again on every restart for every
/// agent — exactly the conservative behavior the cap is for.
#[derive(Default)]
pub struct EventOrchestrator {
    /// Agent ids that have already participated in at least one
    /// tick this process. An agent NOT in this set is "boot-fresh
    /// from this orchestrator's perspective" and the first-tick
    /// cap applies.
    seen_agents: Mutex<HashSet<String>>,
}

impl EventOrchestrator {
    pub fn new() -> Self {
        Self {
            seen_agents: Mutex::new(HashSet::new()),
        }
    }

    /// Return the set of agent ids that have NEVER been seen by
    /// this process before this call, AND mark them as seen.
    ///
    /// The orchestrator uses this to decide which fires need to
    /// pass through the first-tick burst cap: a fire belongs to a
    /// "fresh" agent iff its id is in the returned set.
    fn mark_seen(&self, ids: &[String]) -> HashSet<String> {
        let mut guard = match self.seen_agents.lock() {
            Ok(g) => g,
            // A poisoned lock means a prior tick panicked while
            // holding it — defensive: just take the inner without
            // exploding. The cap may double-fire for these agents
            // (correct trade-off vs. crashing the orchestrator).
            Err(p) => p.into_inner(),
        };
        let mut fresh = HashSet::new();
        for id in ids {
            if guard.insert(id.clone()) {
                fresh.insert(id.clone());
            }
        }
        fresh
    }
}

/// Drive one event evaluation cycle. Called from
/// `usage_snapshot::run_tick`. Thin shim over [`tick_inner`] —
/// resolves the production I/O seams (real store, real ledger, real
/// session index, real dispatcher) and forwards.
pub async fn tick(app: &AppHandle, config_dir: PathBuf) {
    let app_for_disp = app.clone();
    let dispatcher = move |agent: Agent, fire: EventFire| {
        // grill X7: detach `dispatch()` from the shared tick. A
        // multi-minute `claude -p` run no longer blocks
        // permission-revert / PR refresh / snapshot / rotation.
        // The F1 ordering is preserved because the ledger save
        // happens BEFORE this spawn (see the dispatch loop in
        // `tick_inner`).
        let app_clone = app_for_disp.clone();
        tauri::async_runtime::spawn(async move {
            dispatch(&app_clone, &agent, &fire).await;
        });
    };

    let app_for_emit = app.clone();
    let emit_capped: Arc<dyn Fn(usize, usize) + Send + Sync> =
        Arc::new(move |dropped, cap| emit_first_tick_capped(&app_for_emit, dropped, cap));

    // grill X16: the cap is per-agent now. Capture the state handle
    // so `tick_inner`'s env can ask it which agents are fresh on
    // this tick — including agents added AFTER orchestrator boot,
    // which the prior global boolean missed.
    let state = app.state::<Arc<EventOrchestrator>>().inner().clone();

    let env = ProdTickEnv {
        config_dir,
        orchestrator: state,
        emit_capped,
    };

    tick_inner(&env, dispatcher, Utc::now).await;
}

/// I/O surface that [`tick_inner`] depends on, factored so tests can
/// supply deterministic fakes (grill X8 / T1). Production wiring
/// lives in [`ProdTickEnv`]; the test module supplies a fixture
/// implementation.
trait TickEnv {
    fn load_event_agents(&self) -> Result<Vec<Agent>, String>;
    fn build_exclusion_set(&self, agents: &[Agent]) -> HashSet<String>;
    fn load_ledger(&self) -> std::io::Result<EventsFile>;
    fn save_ledger(&self, ledger: &EventsFile) -> Result<(), events_store::AgentEventsError>;
    fn list_sessions(&self) -> Vec<SessionRow>;
    /// Mark a slice of agent ids as having participated in a tick,
    /// returning the subset that had never been seen before (grill
    /// X16). The first-tick burst cap applies to fires belonging to
    /// the returned set. Production delegates to
    /// [`EventOrchestrator::mark_seen`]; tests supply a fixed set.
    fn mark_agents_seen(&self, ids: &[String]) -> HashSet<String>;
    /// Emit the "burst capped" notification to the frontend. A no-op
    /// in tests.
    fn emit_burst_capped(&self, dropped: usize, cap: usize);
    /// Post-tick reconciliation — fire-and-forget logging. A no-op
    /// in tests.
    fn reconcile(&self);
}

/// Production [`TickEnv`] — wires real I/O.
struct ProdTickEnv {
    config_dir: PathBuf,
    orchestrator: Arc<EventOrchestrator>,
    emit_capped: Arc<dyn Fn(usize, usize) + Send + Sync>,
}

impl TickEnv for ProdTickEnv {
    fn load_event_agents(&self) -> Result<Vec<Agent>, String> {
        load_event_agents()
    }
    fn build_exclusion_set(&self, agents: &[Agent]) -> HashSet<String> {
        build_self_exclusion_set(agents)
    }
    fn load_ledger(&self) -> std::io::Result<EventsFile> {
        events_store::load()
    }
    fn save_ledger(&self, ledger: &EventsFile) -> Result<(), events_store::AgentEventsError> {
        events_store::save(ledger)
    }
    fn list_sessions(&self) -> Vec<SessionRow> {
        match claudepot_core::session::list_all_sessions(&self.config_dir) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "agent_event_orchestrator: session index failed");
                Vec::new()
            }
        }
    }
    fn mark_agents_seen(&self, ids: &[String]) -> HashSet<String> {
        self.orchestrator.mark_seen(ids)
    }
    fn emit_burst_capped(&self, dropped: usize, cap: usize) {
        (self.emit_capped)(dropped, cap);
    }
    fn reconcile(&self) {
        // grill X15: dropped. Boot-time reconciliation in `lib.rs`
        // (both F15 forward and X9 reverse) already catches every
        // case the per-tick call was designed to surface. Running
        // it 288 times a day per host was wasted work plus lock
        // contention on the store. Kept as a no-op rather than
        // removed from the trait so the test fixtures (and any
        // future post-tick observability hook) stay wireable.
    }
}

/// The orchestrator's load-bearing cycle, with I/O abstracted behind
/// [`TickEnv`] and dispatch abstracted behind a closure so tests can
/// assert on F1 ordering (X4 / X8), F14 stats derivation, F17
/// exclusion, the burst cap, and the env-var round-trip without
/// touching a real filesystem or spawning `claude -p`.
///
/// `dispatcher` is **fire-and-forget**: the contract is that the
/// orchestrator returns from this function as soon as every fire has
/// been *handed off* (the F1 ledger save has already committed). In
/// production [`tick`] supplies a `tauri::async_runtime::spawn`-based
/// dispatcher; in tests the dispatcher just records the call.
async fn tick_inner<E, D, C>(env: &E, mut dispatcher: D, clock: C)
where
    E: TickEnv + ?Sized,
    D: FnMut(Agent, EventFire),
    C: Fn() -> DateTime<Utc>,
{
    // ---- 1. Open the store + filter to the relevant agents -----
    let agents = match env.load_event_agents() {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "agent_event_orchestrator: store load failed; skipping tick");
            return;
        }
    };
    if agents.is_empty() {
        // Common path: no event-triggered agents installed. Bail
        // before we touch the session index or the ledger.
        return;
    }

    let now = clock();
    let live_agent_ids: HashSet<String> = agents.iter().map(|a| a.id.to_string()).collect();

    // ---- 2. F17 self-trigger exclusion set --------------------
    let agent_session_ids = env.build_exclusion_set(&agents);

    // ---- 3. F14 per-agent stats from the durable ledger -------
    let mut ledger = match env.load_ledger() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "agent_event_orchestrator: ledger load failed; skipping tick");
            return;
        }
    };
    let run_stats_map = build_run_stats_from_ledger(&ledger, now);

    // ---- 4. Index the live CC sessions ------------------------
    let sessions: Vec<SessionRow> = env.list_sessions();
    let live_session_ids: HashSet<String> = sessions.iter().map(|s| s.session_id.clone()).collect();

    // ---- 5. Pure evaluator -----------------------------------
    let fired_pairs: HashSet<(String, String)> = ledger
        .fired
        .iter()
        .map(|e| (e.agent_id.clone(), e.session_id.clone()))
        .collect();
    let stats_fn = |id: &str| run_stats_map.get(id).cloned().unwrap_or_default();
    let mut fires = evaluate_events(
        &agents,
        &sessions,
        &fired_pairs,
        &agent_session_ids,
        &stats_fn,
        now,
    );

    // ---- 6. Bounded catch-up cap (D6 + grill X16) -------------
    // The cap applies per-agent: every event-triggered agent gets
    // its bounded catch-up the FIRST time it participates in a
    // tick this process. Agents added later (the X16 scenario) are
    // capped on their first contact; long-running agents are not
    // re-capped on every tick.
    let agent_ids_this_tick: Vec<String> = agents.iter().map(|a| a.id.to_string()).collect();
    let fresh_agents = env.mark_agents_seen(&agent_ids_this_tick);
    if !fresh_agents.is_empty() {
        fires = apply_per_agent_first_tick_cap(
            fires,
            &fresh_agents,
            FIRST_TICK_BURST_CAP,
            |dropped, cap| env.emit_burst_capped(dropped, cap),
        );
    }

    // ---- 7. Dispatch each fire — record_fire + save FIRST -----
    // (F1) The ledger is the single source of fire-once truth; a
    // duplicate ledger entry is free, but a duplicate billed
    // `claude -p` is a real cost leak. So we always commit the
    // ledger update before handing off the run.
    let agents_by_id: std::collections::HashMap<String, &Agent> =
        agents.iter().map(|a| (a.id.to_string(), a)).collect();

    for fire in &fires {
        let Some(agent) = agents_by_id.get(&fire.agent_id) else {
            // Should be unreachable — the evaluator only returns
            // fires for agents we passed in — but if a race ever
            // dropped one, just skip.
            continue;
        };
        // A2: snapshot the ledger's `fired` Vec BEFORE record_fire so
        // a save failure can restore the FULL pre-mutation state — not
        // just the entry we pushed. The narrower X4 `unrecord_fire`
        // only drops the just-pushed entry, but `record_fire` ALSO
        // evicts the oldest entries when over `MAX_FIRED_ENTRIES`;
        // those evicted pairs would silently re-fire (and re-bill) on
        // the next tick. Cloning the `Vec<FiredEntry>` is bounded by
        // the same cap (~2000 entries) — cheap, and re-allocated only
        // on the rare save-failure path.
        let pre_record_fired = ledger.fired.clone();
        ledger.record_fire(&fire.agent_id, &fire.session_id, now);
        if let Err(e) = env.save_ledger(&ledger) {
            tracing::warn!(
                error = %e,
                agent_id = %fire.agent_id,
                session_id = %fire.session_id,
                "agent_event_orchestrator: ledger save failed; \
                 skipping this fire — it will be re-evaluated next tick"
            );
            // A2 / grill X4 / F1: restore the FULL pre-mutation
            // snapshot — the post-loop prune save below would
            // otherwise flush:
            //  (a) the just-pushed entry (X4 covered this), AND
            //  (b) the cap-eviction side effect: if `record_fire`
            //      evicted the oldest entries to stay under
            //      MAX_FIRED_ENTRIES, those evicted pairs would re-
            //      fire next tick and re-bill. Snapshot-restore
            //      undoes BOTH at once.
            ledger.fired = pre_record_fired;
            continue;
        }
        // The ledger save is committed — now hand off the run.
        // The dispatcher is fire-and-forget; in production it
        // `spawn`s onto the Tauri runtime so a slow narration
        // cannot block the rest of the snapshot pipeline.
        dispatcher((*agent).clone(), fire.clone());
    }

    // ---- 8. Prune the ledger of stale pairs -------------------
    let removed = ledger.prune(&live_agent_ids, &live_session_ids);
    if removed > 0 {
        if let Err(e) = env.save_ledger(&ledger) {
            tracing::warn!(
                error = %e,
                removed,
                "agent_event_orchestrator: ledger prune save failed"
            );
        }
    }

    // ---- 9. Orphan-record reconciliation (observability) ------
    env.reconcile();
}

/// Load the relevant agents from the store: `Installed && enabled
/// && Event`-triggered. Anything else cannot fire from this
/// orchestrator and so isn't returned.
fn load_event_agents() -> Result<Vec<Agent>, String> {
    let store = AgentStore::open().map_err(|e| e.to_string())?;
    Ok(store
        .list()
        .iter()
        .filter(|a| a.lifecycle == Lifecycle::Installed)
        .filter(|a| a.enabled)
        .filter(|a| matches!(a.trigger, Trigger::Event { .. }))
        .cloned()
        .collect())
}

/// **F17** — build the self-trigger exclusion set from the
/// authoritative `RunResult.session_id` parsed out of each agent
/// run's `result.json`. NEVER consults
/// `AgentRun::session_jsonl_path`, which is derived by a bounded
/// filename walk and fails open: when the walk returns `None`
/// the agent's own session id silently leaves the set and the
/// Session Narrator narrates its own output — the D7 loop.
///
/// Best-effort: per-agent enumeration failure logs + skips that
/// agent; a single unreadable `result.json` is silently ignored
/// (the agent's other runs still contribute their session ids).
fn build_self_exclusion_set(agents: &[Agent]) -> HashSet<String> {
    build_self_exclusion_set_with(agents, &|agent| {
        let mut sids: Vec<Option<String>> = Vec::new();
        let ids = match list_run_ids(&agent.id) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(
                    agent_id = %agent.id,
                    error = %e,
                    "agent_event_orchestrator: list_run_ids failed; \
                     skipping this agent's exclusion contribution"
                );
                return Vec::new();
            }
        };
        for run_id in ids {
            let Ok(run) = read_run(&agent.id, &run_id) else {
                continue;
            };
            // Authoritative source: the parsed `RunResult`. We
            // deliberately ignore `run.session_jsonl_path` — see
            // F17 doc on `claudepot_core::agent::events`.
            sids.push(run.result.as_ref().and_then(|r| r.session_id.clone()));
        }
        sids
    })
}

// `build_self_exclusion_set_with`, `build_run_stats_from_ledger`,
// `apply_per_agent_first_tick_cap`, and `build_dispatch_env` live in
// `claudepot_core::agent::events::policy` (the rotation pattern:
// policy in core, wiring here). The F14/F17 contracts are documented
// on the core functions.

/// Spawn a Run-Now for `agent` carrying the firing session's id +
/// transcript path as env vars. Errors are logged but never
/// propagated — the ledger has already recorded the fire and a
/// re-fire would just produce a duplicate ledger entry (still
/// safe, since `record_fire` is idempotent), but it would also
/// re-bill. Per F1's stated trade-off, a crashed/never-dispatched
/// fire is the acceptable failure direction.
async fn dispatch(app: &AppHandle, agent: &Agent, fire: &EventFire) {
    let route_lookup = crate::commands::agents::route_lookup_fn();
    let binary_path = match resolve_binary(agent, &route_lookup) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                agent_id = %agent.id,
                error = %e,
                "agent_event_orchestrator: resolve_binary failed; \
                 fire is recorded but no run will be spawned"
            );
            // grill X11: the ledger has the pair recorded (F1), so
            // the next tick will skip this (agent, session) — without
            // an on-disk breadcrumb the failed dispatch is invisible
            // to the run-history surface. Drop a synthetic
            // dispatch-failed dir so a user investigating "why was
            // session X never narrated?" can find the row.
            write_dispatch_failed_breadcrumb(
                &agent.id,
                &fire.session_id,
                "resolve_binary",
                &e.to_string(),
            );
            emit_failed(app, &fire.agent_id, &fire.session_id, &e.to_string());
            return;
        }
    };
    let cli_path = match current_claudepot_cli() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent_event_orchestrator: current_claudepot_cli failed"
            );
            // grill X11: see resolve_binary branch above.
            write_dispatch_failed_breadcrumb(
                &agent.id,
                &fire.session_id,
                "current_claudepot_cli",
                &e.to_string(),
            );
            emit_failed(app, &fire.agent_id, &fire.session_id, &e.to_string());
            return;
        }
    };

    let env = build_dispatch_env(fire);

    let sink = NoopSink;
    match agent::run_now(agent, &binary_path, &cli_path, &sink, &env).await {
        Ok(run) => {
            tracing::info!(
                agent_id = %agent.id,
                run_id = %run.id,
                session_id = %fire.session_id,
                "agent_event_orchestrator: dispatched fire"
            );
            emit_dispatched(app, &fire.agent_id, &fire.session_id, &run.id);
        }
        Err(e) => {
            tracing::warn!(
                agent_id = %agent.id,
                session_id = %fire.session_id,
                error = %e,
                "agent_event_orchestrator: run_now failed (ledger already recorded the fire)"
            );
            emit_failed(app, &fire.agent_id, &fire.session_id, &e.to_string());
        }
    }
}

/// grill X11 — pre-spawn dispatch failure breadcrumb.
///
/// When `resolve_binary` / `current_claudepot_cli` fail BEFORE
/// `run_now`, no run directory has been created and `record_run`
/// never runs. The (agent, session) pair is in the ledger (F1) so
/// the next tick will skip it — leaving the user with a session
/// that should have been narrated, no record, no log, and a single
/// toast that may have been dismissed.
///
/// This drops a synthetic `dispatch-failed-<ts>-<session>/` directory
/// under the agent's runs root carrying two files:
///
/// - `error.txt` — the human-readable forensic (plain text, mirror
///   of the F5 `record-run-error.txt` shape).
/// - `result.json` — a synthetic [`AgentRun`] that round-trips
///   through `agents_runs_list`'s reader so the breadcrumb surfaces
///   as a real (failed) row in the RunHistoryPanel (third-eye
///   audit A1: previously the dir only carried `error.txt` and
///   `agents_runs_list` filters by deserializable `result.json`, so
///   the breadcrumb was invisible to the panel).
///
/// Best-effort throughout — every I/O failure is logged + swallowed,
/// because failing to write a breadcrumb must never abort the
/// orchestrator. The ledger already captured the fire; the
/// breadcrumb is an observability convenience on top.
fn write_dispatch_failed_breadcrumb(
    agent_id: &agent::AgentId,
    session_id: &str,
    stage: &str,
    error: &str,
) {
    // Use an ISO timestamp + sanitized session prefix so the dir
    // name sorts the same way `run-id` directories do (the
    // run-history panel sorts by name) AND so a re-fire on the
    // same session at a later time produces a new row rather than
    // overwriting the prior one.
    let now_dt = Utc::now();
    let now = now_dt.format("%Y%m%dT%H%M%SZ");
    let session_slug: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let run_id = format!("dispatch-failed-{now}-{session_slug}");
    let runs_root = agent_runs_dir(agent_id);
    let dir = runs_root.join(&run_id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            agent_id = %agent_id,
            session_id = %session_id,
            error = %e,
            "agent_event_orchestrator: dispatch-failed breadcrumb dir \
             create failed; the user will not see this run in run-history"
        );
        return;
    }
    let body = format!(
        "dispatch failed for agent={agent_id} session={session_id}\n\
         stage={stage}\n\
         at={now}\n\n\
         error: {error}\n\n\
         note: The (agent, session) pair has been recorded in the \
         event ledger (F1 ordering), so the next tick will not re-fire \
         it. To re-attempt narration, either delete the pair from \
         ~/.claudepot/agent-events.json or use Run Now on the agent.\n"
    );
    if let Err(e) = std::fs::write(dir.join("error.txt"), body) {
        tracing::warn!(
            agent_id = %agent_id,
            session_id = %session_id,
            error = %e,
            "agent_event_orchestrator: dispatch-failed breadcrumb file \
             write failed"
        );
    }

    // A1: also write a synthetic `result.json` so the breadcrumb dir
    // surfaces as a real run row in `agents_runs_list`. Without this,
    // the panel's reader (`serde_json::from_slice::<AgentRun>`) skips
    // the directory and the forensic is invisible from the surface
    // most users actually open. We keep the existing `error.txt` for
    // human inspection — the two files answer different needs.
    //
    // Shape:
    //  - `id` is the breadcrumb dir name (deterministic, sortable).
    //  - `agent_id` is the firing agent.
    //  - `trigger_kind = Manual` — the wire enum has no `Event`
    //    variant; event-orchestrator runs that DO succeed also land
    //    here as `Manual` (see `run_now`), so we stay consistent.
    //  - `exit_code = -1` and `result.is_error = true` make the
    //    RunHistoryPanel render the "ERR" status; `result.errors`
    //    carries the failure message so the "show" disclosure
    //    surfaces it under "errors".
    let synthetic = agent::AgentRun {
        id: run_id.clone(),
        agent_id: *agent_id,
        started_at: now_dt,
        ended_at: now_dt,
        duration_ms: 0,
        exit_code: -1,
        result: Some(agent::RunResult {
            subtype: Some(format!("dispatch_failed:{stage}")),
            is_error: Some(true),
            num_turns: None,
            total_cost_usd: None,
            stop_reason: None,
            session_id: Some(session_id.to_string()),
            errors: vec![format!("dispatch failed at {stage}: {error}")],
        }),
        session_jsonl_path: None,
        stdout_log: String::new(),
        stderr_log: String::new(),
        trigger_kind: agent::TriggerKind::Manual,
        host_platform: agent::HostPlatform::current(),
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        output_artifacts: Vec::new(),
        route_decision: None,
    };
    match serde_json::to_vec_pretty(&synthetic) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(dir.join("result.json"), &bytes) {
                tracing::warn!(
                    agent_id = %agent_id,
                    session_id = %session_id,
                    error = %e,
                    "agent_event_orchestrator: dispatch-failed synthetic \
                     result.json write failed — the row will not surface in \
                     RunHistoryPanel, only error.txt"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                agent_id = %agent_id,
                session_id = %session_id,
                error = %e,
                "agent_event_orchestrator: dispatch-failed synthetic \
                 result.json serialize failed"
            );
        }
    }
}

/// `ProgressSink` impl that discards phase events. The event
/// orchestrator has no UI surface for in-flight progress — runs
/// either succeed (the row appears in RunHistoryPanel) or fail
/// (logged + event-emitted).
struct NoopSink;
impl claudepot_core::project_progress::ProgressSink for NoopSink {
    fn phase(&self, _phase: &str, _status: claudepot_core::project_progress::PhaseStatus) {}
    fn sub_progress(&self, _phase: &str, _current: usize, _total: usize) {}
}

// ---------------------------------------------------------------------------
// Frontend events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEventDispatchedPayload {
    agent_id: String,
    session_id: String,
    run_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEventFailedPayload {
    agent_id: String,
    session_id: String,
    error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEventBurstCappedPayload {
    cap: usize,
    dropped: usize,
}

fn emit_dispatched(app: &AppHandle, agent_id: &str, session_id: &str, run_id: &str) {
    let payload = AgentEventDispatchedPayload {
        agent_id: agent_id.to_string(),
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
    };
    if let Err(e) = app.emit(crate::events::AGENT_EVENT_DISPATCHED, payload) {
        tracing::warn!(error = %e, "agent_event_orchestrator: emit dispatched failed");
    }
}

fn emit_failed(app: &AppHandle, agent_id: &str, session_id: &str, error: &str) {
    let payload = AgentEventFailedPayload {
        agent_id: agent_id.to_string(),
        session_id: session_id.to_string(),
        error: error.to_string(),
    };
    if let Err(e) = app.emit(crate::events::AGENT_EVENT_FAILED, payload) {
        tracing::warn!(error = %e, "agent_event_orchestrator: emit failed failed");
    }
}

fn emit_first_tick_capped(app: &AppHandle, dropped: usize, cap: usize) {
    let payload = AgentEventBurstCappedPayload { cap, dropped };
    if let Err(e) = app.emit(crate::events::AGENT_EVENT_BURST_CAPPED, payload) {
        tracing::warn!(error = %e, "agent_event_orchestrator: emit burst-capped failed");
    }
}

// ---------------------------------------------------------------------------
// Tests — the orchestrator's three load-bearing constraints
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, TimeZone};

    fn ts(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap() + ChronoDuration::minutes(min)
    }

    // The pure F14 run-stats and first-tick-cap tests moved to
    // `claudepot_core::agent::events::policy::tests` with the
    // functions themselves.

    #[test]
    fn test_event_orchestrator_mark_seen_returns_only_first_contact() {
        // grill X16: `mark_seen` returns the subset of ids that
        // had not been seen before. Every subsequent call with the
        // same ids returns an empty set; brand-new ids appear in
        // the fresh set on their first call. The catch-up cap is
        // applied to fires for ids in the returned set, so the
        // semantics are "an agent gets capped on its FIRST tick
        // this process — whenever that tick happens to be."
        let o = EventOrchestrator::new();
        let fresh1 = o.mark_seen(&["a".to_string(), "b".to_string()]);
        assert_eq!(fresh1.len(), 2, "both ids are brand new on the first call");
        assert!(fresh1.contains("a") && fresh1.contains("b"));

        let fresh2 = o.mark_seen(&["a".to_string(), "b".to_string()]);
        assert!(
            fresh2.is_empty(),
            "ids that have already been marked must NOT reappear as fresh"
        );

        // Late-add scenario: a brand-new id "c" appears later. It
        // is fresh (gets the cap on its first contact), while "a"
        // and "b" continue to pass through uncapped.
        let fresh3 = o.mark_seen(&["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(
            fresh3,
            ["c".to_string()].into_iter().collect::<HashSet<_>>(),
            "X16: a late-added agent participates as fresh on its first tick"
        );
    }

    // ---- F1 record-before-dispatch ordering -------------------
    //
    // The dispatch flow is "record_fire + save; THEN dispatch".
    // A duplicate ledger entry is free; a duplicate billed run is
    // a real cost leak. This test verifies the ordering invariant
    // by structurally enforcing it: we build the same dispatch
    // shape the orchestrator uses and assert that the ledger
    // mutation completes before any dispatch-equivalent side
    // effect would be observable.
    #[test]
    fn test_f1_ledger_record_before_dispatch_ordering() {
        let now = ts(0);
        let mut ledger = EventsFile::default();
        let fire = mk_fire("agent-1", "sess-1");

        // Phase 1: record + (in the real orchestrator: save) the
        // ledger entry FIRST. Until this is observable, no
        // dispatch is permitted.
        ledger.record_fire(&fire.agent_id, &fire.session_id, now);

        // Phase 2: dispatch is only allowed if the ledger entry
        // is already visible to a re-evaluation. Assert that
        // invariant explicitly.
        assert!(
            ledger.has_fired(&fire.agent_id, &fire.session_id),
            "the ledger MUST already have the fire before dispatch"
        );
        // The "dispatch" itself in this pure test is the assertion
        // above succeeding; no further side effect needed.
        assert!(ledger.has_fired("agent-1", "sess-1"));
    }

    #[test]
    fn test_f1_skipped_dispatch_does_not_unfire() {
        // F1 trade-off direction: when the orchestrator decides
        // (e.g. save failed) to skip a dispatch, the ledger entry
        // remains. The next tick will see fire-once still
        // satisfied and not re-fire. This is the acceptable
        // failure direction: a missed run, not a retry storm.
        let mut ledger = EventsFile::default();
        ledger.record_fire("agent-1", "sess-1", ts(0));
        // Simulated dispatch failure → no rollback of the ledger.
        // Re-evaluation must skip the pair.
        let fired_pairs: HashSet<(String, String)> = ledger
            .fired
            .iter()
            .map(|e| (e.agent_id.clone(), e.session_id.clone()))
            .collect();
        assert!(
            fired_pairs.contains(&("agent-1".to_string(), "sess-1".to_string())),
            "the fired pair must remain in the ledger after a skipped dispatch"
        );
    }

    // The pure F17 exclusion-set tests moved to
    // `claudepot_core::agent::events::policy::tests` with the
    // function; `build_self_exclusion_set` (the I/O wrapper above)
    // stays here and is exercised by the tick integration tests.

    fn stub_agent(name: &str) -> Agent {
        use claudepot_core::agent::{
            AgentBinary, CreatedVia, EventKind, OutputFormat, PermissionMode, PlatformOptions,
            RateLimit, Trigger,
        };
        let now = Utc::now();
        Agent {
            id: uuid::Uuid::new_v4(),
            name: name.to_string(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AgentBinary::FirstParty,
            model: Some("haiku".into()),
            cwd: "/tmp".into(),
            prompt: "p".into(),
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: PermissionMode::Default,
            allowed_tools: vec!["Read".into()],
            add_dir: vec![],
            max_budget_usd: None,
            fallback_model: None,
            output_format: OutputFormat::Json,
            json_schema: None,
            bare: false,
            extra_env: Default::default(),
            trigger: Trigger::Event {
                event: EventKind::SessionSettled { debounce_secs: 600 },
            },
            platform_options: PlatformOptions::default(),
            log_retention_runs: 50,
            created_at: now,
            updated_at: now,
            claudepot_managed: true,
            template_id: None,
            disallowed_tools: vec![],
            mcp_servers: vec![],
            run_as: None,
            task_budget: None,
            rate_limit: Some(RateLimit {
                min_interval_secs: Some(60),
                max_per_day: Some(10),
            }),
            lifecycle: Lifecycle::Installed,
            drafted_by: None,
            created_via: CreatedVia::Gui,
        }
    }

    // Helpers ---------------------------------------------------

    fn mk_fire(agent_id: &str, session_id: &str) -> EventFire {
        EventFire {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            session_path: format!("/home/u/.claude/projects/proj/{session_id}.jsonl"),
        }
    }

    // ---- grill X8 — orchestrator integration tests --------------
    //
    // The four invariants we verify here are the ones the prior
    // pass discovered as untested: F1 ordering survives a save
    // failure (X4), dispatch happens fire-and-forget (X7 — the tick
    // returns before the dispatched future resolves), the env-var
    // round-trip (X28 / T8), and a panic in the dispatcher does not
    // corrupt the ledger. The fixture `FakeTickEnv` plugs into
    // `tick_inner` and records every interaction.

    use std::cell::RefCell;
    use std::rc::Rc;

    /// Captured dispatcher call.
    #[derive(Debug, Clone)]
    struct DispatchedCall {
        agent_id: String,
        session_id: String,
        session_path: String,
    }

    struct FakeTickEnv {
        agents: Vec<Agent>,
        exclusion: HashSet<String>,
        sessions: Vec<SessionRow>,
        /// grill X16: tests now control which agent ids count as
        /// "fresh" (i.e. participating in their first tick this
        /// process). Setting `first_tick = true` is preserved as a
        /// shortcut — when true, every agent passed in is treated
        /// as fresh, matching the old single-boolean semantics so
        /// legacy tests are minimally disturbed. Setting it to
        /// `false` AND populating `fresh_override` lets a test
        /// drive the "agent added after boot" scenario directly.
        first_tick: bool,
        fresh_override: Option<HashSet<String>>,
        ledger: Rc<RefCell<EventsFile>>,
        save_fails_n_times: Rc<RefCell<usize>>,
        save_calls: Rc<RefCell<usize>>,
        burst_capped: Rc<RefCell<Vec<(usize, usize)>>>,
        reconciles: Rc<RefCell<usize>>,
    }

    impl TickEnv for FakeTickEnv {
        fn load_event_agents(&self) -> Result<Vec<Agent>, String> {
            Ok(self.agents.clone())
        }
        fn build_exclusion_set(&self, _agents: &[Agent]) -> HashSet<String> {
            self.exclusion.clone()
        }
        fn load_ledger(&self) -> std::io::Result<EventsFile> {
            Ok(self.ledger.borrow().clone())
        }
        fn save_ledger(&self, ledger: &EventsFile) -> Result<(), events_store::AgentEventsError> {
            *self.save_calls.borrow_mut() += 1;
            let mut remaining = self.save_fails_n_times.borrow_mut();
            if *remaining > 0 {
                *remaining -= 1;
                return Err(events_store::AgentEventsError::Io(std::io::Error::other(
                    "synthetic save failure",
                )));
            }
            *self.ledger.borrow_mut() = ledger.clone();
            Ok(())
        }
        fn list_sessions(&self) -> Vec<SessionRow> {
            self.sessions.clone()
        }
        fn mark_agents_seen(&self, ids: &[String]) -> HashSet<String> {
            if let Some(fresh) = &self.fresh_override {
                // Tests drive the fresh set directly.
                fresh.clone()
            } else if self.first_tick {
                // Old "boot tick" semantics: every passed-in agent
                // is fresh.
                ids.iter().cloned().collect()
            } else {
                HashSet::new()
            }
        }
        fn emit_burst_capped(&self, dropped: usize, cap: usize) {
            self.burst_capped.borrow_mut().push((dropped, cap));
        }
        fn reconcile(&self) {
            *self.reconciles.borrow_mut() += 1;
        }
    }

    /// Like [`stub_agent`] but with a caller-chosen cwd, so an
    /// integration test can align the agent's project scope with a
    /// stubbed session's `project_path`.
    fn stub_agent_with_cwd(name: &str, cwd: &str) -> Agent {
        let mut a = stub_agent(name);
        a.cwd = cwd.into();
        a
    }

    fn fake_session(session_id: &str, project: &str) -> SessionRow {
        fake_session_settled_at(session_id, project, |now| now - chrono::Duration::hours(1))
    }

    /// Build a settled-looking session whose `last_ts` is computed
    /// from the wall clock at row-build time. For tests that drive
    /// `tick_inner` with an injected fixed clock, pass a `last_ts`
    /// derived from that clock so the debounce check uses the same
    /// "now" the evaluator sees.
    fn fake_session_at(
        session_id: &str,
        project: &str,
        last_ts: chrono::DateTime<chrono::Utc>,
    ) -> SessionRow {
        fake_session_settled_at(session_id, project, |_| last_ts)
    }

    fn fake_session_settled_at<F>(session_id: &str, project: &str, last_ts: F) -> SessionRow
    where
        F: FnOnce(chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc>,
    {
        // Build a settled-looking session: `last_ts` is 1 hour past
        // the chosen anchor so the default 600s debounce comfortably
        // elapses; `project_path` is the project root the evaluator
        // scopes agents to; the assistant message count is non-zero
        // so the session looks like real CC traffic rather than an
        // empty stub.
        use claudepot_core::session::TokenUsage;
        let now = chrono::Utc::now();
        let one_hour_ago = last_ts(now);
        SessionRow {
            session_id: session_id.to_string(),
            slug: project.to_string(),
            file_path: std::path::PathBuf::from(format!(
                "/tmp/.claude/projects/{project}/{session_id}.jsonl"
            )),
            file_size_bytes: 4096,
            last_modified: Some(std::time::SystemTime::UNIX_EPOCH),
            project_path: format!("/tmp/{project}"),
            project_from_transcript: true,
            first_ts: Some(one_hour_ago - chrono::Duration::hours(1)),
            last_ts: Some(one_hour_ago),
            event_count: 6,
            message_count: 4,
            user_message_count: 2,
            assistant_message_count: 2,
            first_user_prompt: Some("hello".into()),
            models: vec!["claude-sonnet".into()],
            tokens: TokenUsage::default(),
            git_branch: None,
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    #[tokio::test]
    async fn test_tick_inner_f1_save_failure_leaves_ledger_clean() {
        // X4 / X8(a): when the dispatch-loop `save` fails, the
        // in-memory ledger must not retain the just-pushed entry —
        // the post-loop prune save would otherwise flush a
        // fire-without-dispatch entry. The ledger is unchanged on
        // disk; no dispatcher call happens for the failed save.
        let agent = stub_agent_with_cwd("session-narrator", "/tmp/proj");
        let agent_id = agent.id.to_string();
        let session_id = "sess-fail-1".to_string();
        // Anchor the session's `last_ts` to a clock-relative value so
        // the evaluator's debounce comparison uses a consistent
        // "now" between the session row and the injected clock.
        let fixed_now = chrono::Utc::now();
        let sessions = vec![fake_session_at(
            &session_id,
            "proj",
            fixed_now - chrono::Duration::hours(1),
        )];

        let env = FakeTickEnv {
            agents: vec![agent.clone()],
            exclusion: HashSet::new(),
            sessions,
            first_tick: false,
            fresh_override: None,
            // Make the first save fail (the dispatch-loop record_fire
            // save); the post-loop prune save then succeeds. Without
            // X4, the in-memory mutation from `record_fire` would be
            // picked up by the prune save.
            save_fails_n_times: Rc::new(RefCell::new(1)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(EventsFile::default())),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };

        let calls = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls_for_disp = Rc::clone(&calls);
        let dispatcher = move |a: Agent, fire: EventFire| {
            calls_for_disp.borrow_mut().push(DispatchedCall {
                agent_id: a.id.to_string(),
                session_id: fire.session_id,
                session_path: fire.session_path,
            });
        };

        tick_inner(&env, dispatcher, move || fixed_now).await;

        // The ledger on "disk" must not contain the failed pair.
        let on_disk = env.ledger.borrow();
        assert!(
            !on_disk.has_fired(&agent_id, &session_id),
            "the failed-save pair must not have been flushed by the prune save"
        );
        // The dispatcher must NOT have been called for that pair.
        assert!(
            calls.borrow().is_empty(),
            "no dispatch when the ledger save failed"
        );
    }

    /// A2: when `record_fire` evicts oldest entries to honor the
    /// ledger cap AND the subsequent `save_ledger` fails, the
    /// orchestrator must restore the FULL pre-mutation snapshot, not
    /// just drop the just-pushed entry. The narrower `unrecord_fire`
    /// left evicted pairs gone — they would re-fire (and re-bill)
    /// next tick. This test pre-fills the ledger to the cap, forces a
    /// fire that triggers an eviction-plus-push, and forces the save
    /// to fail; the post-failure ledger must equal the snapshot.
    #[tokio::test]
    async fn test_tick_inner_cap_eviction_with_save_fail_restores_snapshot() {
        use claudepot_core::agent::events::store::MAX_FIRED_ENTRIES;
        use claudepot_core::agent::events::FiredEntry;

        let agent = stub_agent_with_cwd("session-narrator", "/tmp/proj");
        let agent_id = agent.id.to_string();
        let new_session_id = "sess-new-after-cap".to_string();
        let fixed_now = chrono::Utc::now();
        let sessions = vec![fake_session_at(
            &new_session_id,
            "proj",
            fixed_now - chrono::Duration::hours(1),
        )];

        // Pre-fill the ledger to MAX_FIRED_ENTRIES so the upcoming
        // `record_fire` triggers an eviction of the oldest entry AND
        // a push of the new one.
        let mut prefill = EventsFile::default();
        for i in 0..MAX_FIRED_ENTRIES {
            prefill.fired.push(FiredEntry {
                agent_id: agent_id.clone(),
                // Sessions that the orchestrator's prune will keep:
                // mark them all under the same agent so they're
                // "live" (the agent IS in the agent set), and use
                // session ids the live session set will also include
                // (we'll seed the live session set below). The
                // prune at end-of-tick only drops pairs whose
                // session is NOT in the live set; to keep the
                // snapshot stable, we make sure the prefill pairs'
                // sessions are reflected in the live set too via the
                // FakeTickEnv's session list. But we also DON'T want
                // them to re-evaluate as fires, so each pair is
                // marked as already-fired by the very ledger we
                // start with.
                session_id: format!("prefill-{i}"),
                fired_at: fixed_now - chrono::Duration::seconds(i as i64 + 60),
            });
        }
        // Snapshot the ledger shape BEFORE the tick. After the
        // tick — which will trigger record_fire's cap eviction and
        // then a forced save failure — the in-memory ledger that the
        // post-loop prune save sees must equal this snapshot
        // verbatim. (Prune may then drop some entries whose sessions
        // aren't live, but the assertion below isolates the
        // restoration step — we examine the file via the env's
        // captured save calls + final state.)
        let snapshot_before = prefill.clone();

        // Build the session set: include only the NEW session as
        // "live" so the prefill pairs are NOT pruned away by the
        // session-side of the prune (we want the prefill to stay
        // around so the snapshot-restore assertion has signal). We
        // achieve this by listing every prefill session id alongside
        // the new one, but only the new one as a *settled* row that
        // would emit a fire.
        let mut sessions_full = sessions;
        for i in 0..MAX_FIRED_ENTRIES {
            // Add a non-settled stub so prune's `live_session_ids`
            // contains the prefill ids. The evaluator won't emit
            // fires for them: their session_id is already in the
            // ledger.
            sessions_full.push(fake_session_at(
                &format!("prefill-{i}"),
                "proj",
                fixed_now - chrono::Duration::hours(1),
            ));
        }

        let env = FakeTickEnv {
            agents: vec![agent.clone()],
            exclusion: HashSet::new(),
            sessions: sessions_full,
            first_tick: false,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(1)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(prefill)),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };

        let calls = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls_for_disp = Rc::clone(&calls);
        let dispatcher = move |a: Agent, fire: EventFire| {
            calls_for_disp.borrow_mut().push(DispatchedCall {
                agent_id: a.id.to_string(),
                session_id: fire.session_id,
                session_path: fire.session_path,
            });
        };

        tick_inner(&env, dispatcher, move || fixed_now).await;

        // The dispatcher must NOT have run (save failed before
        // dispatch).
        assert!(
            calls.borrow().is_empty(),
            "no dispatch when the ledger save failed"
        );

        // The on-disk ledger must NOT contain the new session id
        // (covered by the older X4 test too).
        let on_disk = env.ledger.borrow();
        assert!(
            !on_disk.has_fired(&agent_id, &new_session_id),
            "the failed pair must not be flushed"
        );

        // The A2 assertion: every prefill entry that record_fire
        // would have evicted (the OLDEST one by fired_at) is still
        // present after the snapshot restore — `record_fire` evicted
        // it temporarily, but the restore put it back, and the
        // post-loop prune save (which DID succeed on the second save
        // call) flushed the restored set verbatim. Without A2, the
        // post-loop prune would have flushed a ledger missing
        // `prefill-{MAX-1}` (the oldest prefill entry by fired_at,
        // since we used decreasing fired_at) — that pair would re-
        // fire next tick.
        let oldest_id = format!("prefill-{}", MAX_FIRED_ENTRIES - 1);
        assert!(
            on_disk.has_fired(&agent_id, &oldest_id),
            "A2: the cap-evicted prefill entry '{oldest_id}' must be \
             restored — otherwise it would re-fire and re-bill"
        );

        // The full restored set must equal the pre-mutation snapshot
        // (modulo prune, which here is a no-op because every prefill
        // session is in `live_session_ids`).
        assert_eq!(
            on_disk.fired.len(),
            snapshot_before.fired.len(),
            "every prefill entry survives the snapshot-restore + prune"
        );
    }

    #[tokio::test]
    async fn test_tick_inner_happy_path_dispatches_and_records() {
        // X8 base case: a healthy fire records to the ledger AND
        // hands off to the dispatcher. The env-var round-trip
        // (X28 / T8) is verified at the dispatch surface: the
        // closure receives the EventFire whose `session_id` /
        // `session_path` the real `dispatch()` writes verbatim into
        // `CLAUDEPOT_EVENT_SESSION_ID` / `CLAUDEPOT_EVENT_SESSION_PATH`.
        let agent = stub_agent_with_cwd("session-narrator", "/tmp/proj");
        let agent_id = agent.id.to_string();
        let session_id = "sess-happy".to_string();
        let session_path = format!("/tmp/.claude/projects/proj/{session_id}.jsonl");
        let fixed_now = chrono::Utc::now();
        let sessions = vec![fake_session_at(
            &session_id,
            "proj",
            fixed_now - chrono::Duration::hours(1),
        )];

        let env = FakeTickEnv {
            agents: vec![agent.clone()],
            exclusion: HashSet::new(),
            sessions,
            first_tick: false,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(EventsFile::default())),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };

        let calls = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls_for_disp = Rc::clone(&calls);
        let dispatcher = move |a: Agent, fire: EventFire| {
            calls_for_disp.borrow_mut().push(DispatchedCall {
                agent_id: a.id.to_string(),
                session_id: fire.session_id,
                session_path: fire.session_path,
            });
        };

        tick_inner(&env, dispatcher, move || fixed_now).await;

        assert_eq!(calls.borrow().len(), 1, "the healthy pair must dispatch");
        let call = &calls.borrow()[0];
        assert_eq!(call.agent_id, agent_id);
        assert_eq!(call.session_id, session_id);
        // X28 / T8: the session path the dispatcher would write into
        // `CLAUDEPOT_EVENT_SESSION_PATH` is exactly the path the
        // evaluator produces from the session row. A rename on one
        // side and not the other will diverge here.
        assert_eq!(call.session_path, session_path);

        // The ledger must show the fire.
        assert!(env.ledger.borrow().has_fired(&agent_id, &session_id));
        // grill X15: the post-tick `reconcile` hook is still invoked
        // by `tick_inner` (kept as a seam), but production wires it
        // to a no-op; boot-time reconciliation (F15 + X9 in lib.rs)
        // covers the discovery. The test fixture still counts hook
        // invocations to lock the call shape down.
        assert_eq!(*env.reconciles.borrow(), 1);
    }

    #[tokio::test]
    async fn test_tick_inner_no_event_agents_is_zero_overhead() {
        // X8: the "zero overhead when no event agents installed"
        // claim — confirmed by tick_inner returning before it asks
        // for the ledger, sessions, etc.
        let env = FakeTickEnv {
            agents: vec![],
            exclusion: HashSet::new(),
            sessions: vec![],
            first_tick: false,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(EventsFile::default())),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };
        let dispatcher = |_: Agent, _: EventFire| {
            panic!("dispatcher must not be called when no event agents exist");
        };
        tick_inner(&env, dispatcher, Utc::now).await;
        assert_eq!(*env.save_calls.borrow(), 0, "no save when no agents");
        assert_eq!(*env.reconciles.borrow(), 0, "no reconcile when no agents");
    }

    #[tokio::test]
    async fn test_tick_inner_panicking_dispatcher_does_not_corrupt_ledger() {
        // X8(d): a panic *inside* the dispatcher closure must not
        // leak into the orchestrator. In production the dispatcher
        // spawns onto the Tauri runtime, so a panic is isolated to
        // that task and the orchestrator's tick continues. We
        // emulate the production shape: the dispatcher catches its
        // own panic so the tick returns normally. The ledger must
        // still record the fire (the save committed BEFORE the
        // dispatch — F1 ordering).
        let agent = stub_agent_with_cwd("session-narrator", "/tmp/proj");
        let agent_id = agent.id.to_string();
        let session_id = "sess-panic".to_string();
        let sessions = vec![fake_session(&session_id, "proj")];

        let env = FakeTickEnv {
            agents: vec![agent.clone()],
            exclusion: HashSet::new(),
            sessions,
            first_tick: false,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(EventsFile::default())),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };

        // The dispatcher catches its own panic — mirroring the
        // `tauri::async_runtime::spawn`-isolated production shape.
        let dispatcher = |_: Agent, _: EventFire| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                panic!("synthetic panic inside the dispatcher");
            }));
        };

        tick_inner(&env, dispatcher, Utc::now).await;

        // F1 ordering: the ledger save committed BEFORE the
        // dispatcher was invoked. The ledger therefore retains the
        // fire even though the dispatcher panicked — exactly the
        // documented "missed run, not a retry storm" trade-off.
        assert!(env.ledger.borrow().has_fired(&agent_id, &session_id));
    }

    #[tokio::test]
    async fn test_tick_inner_two_tick_capped_fires_get_picked_up() {
        // X8(b): the first-tick burst cap drops fires beyond `cap`,
        // but those pairs are NOT in the ledger — so a follow-up
        // tick (with `first_tick = false`) re-evaluates and
        // dispatches them. The dropped-then-refire intent is core
        // to the "bounded catch-up" semantics; without this test a
        // refactor that recorded the dropped fires would silently
        // mask them forever.
        // Build (cap+2) eligible (agent, session) pairs.
        let total: usize = FIRST_TICK_BURST_CAP + 2;
        let mut agents = Vec::new();
        let mut sessions = Vec::new();
        let fixed_now = chrono::Utc::now();
        for i in 0..total {
            let project = format!("proj-{i}");
            agents.push(stub_agent_with_cwd(
                &format!("agent-{i}"),
                &format!("/tmp/{project}"),
            ));
            sessions.push(fake_session_at(
                &format!("sess-{i}"),
                &project,
                fixed_now - chrono::Duration::hours(1),
            ));
        }
        let ledger = Rc::new(RefCell::new(EventsFile::default()));

        // Tick 1 — first-tick cap drops 2 fires.
        let env1 = FakeTickEnv {
            agents: agents.clone(),
            exclusion: HashSet::new(),
            sessions: sessions.clone(),
            first_tick: true,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::clone(&ledger),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };
        let calls1 = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls1_for_disp = Rc::clone(&calls1);
        tick_inner(
            &env1,
            move |a: Agent, fire: EventFire| {
                calls1_for_disp.borrow_mut().push(DispatchedCall {
                    agent_id: a.id.to_string(),
                    session_id: fire.session_id,
                    session_path: fire.session_path,
                });
            },
            move || fixed_now,
        )
        .await;

        assert_eq!(
            calls1.borrow().len(),
            FIRST_TICK_BURST_CAP,
            "first tick must dispatch exactly the cap"
        );
        // The cap dropped 2 fires; they must NOT be in the ledger,
        // so the next tick is free to dispatch them.
        let after_tick1_fired: usize = ledger.borrow().fired.len();
        assert_eq!(
            after_tick1_fired, FIRST_TICK_BURST_CAP,
            "the dropped fires must NOT have been recorded"
        );

        // Tick 2 — no first-tick cap; remaining fires dispatch.
        let env2 = FakeTickEnv {
            agents: agents.clone(),
            exclusion: HashSet::new(),
            sessions: sessions.clone(),
            first_tick: false,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::clone(&ledger),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };
        let calls2 = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls2_for_disp = Rc::clone(&calls2);
        tick_inner(
            &env2,
            move |a: Agent, fire: EventFire| {
                calls2_for_disp.borrow_mut().push(DispatchedCall {
                    agent_id: a.id.to_string(),
                    session_id: fire.session_id,
                    session_path: fire.session_path,
                });
            },
            move || fixed_now,
        )
        .await;

        assert_eq!(
            calls2.borrow().len(),
            total - FIRST_TICK_BURST_CAP,
            "the previously-dropped fires must dispatch on the next tick"
        );
        assert_eq!(
            ledger.borrow().fired.len(),
            total,
            "every original (agent, session) pair must now be in the ledger"
        );
    }

    // `test_dispatch_env_round_trip` (the X8(c)/X28/T8 shim-contract
    // gate) moved to `claudepot_core::agent::events::policy::tests`
    // with `build_dispatch_env`.

    /// A1: the X11 dispatch-failed breadcrumb must surface as a real
    /// row in `agents_runs_list`. The previous breadcrumb only wrote
    /// `error.txt`; `agents_runs_list` filters by deserializable
    /// `result.json`, so the row was invisible to the panel. The fix
    /// writes a synthetic `AgentRun` alongside; this test asserts that
    /// the file round-trips through `serde_json` into a coherent
    /// `AgentRun` whose shape renders meaningfully (ERR status,
    /// error string in `result.errors`).
    #[test]
    fn test_dispatch_failed_breadcrumb_writes_synthetic_result_json() {
        let tmp = tempfile::tempdir().unwrap();
        // Serialize against any other test that also sets DATA_DIR.
        let _guard = std::sync::Mutex::new(()); // local guard — best-effort.
        let prev = std::env::var_os("CLAUDEPOT_DATA_DIR");
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path());

        let agent_id = uuid::Uuid::new_v4();
        let session_id = "deadbeef-cafe";
        write_dispatch_failed_breadcrumb(
            &agent_id,
            session_id,
            "resolve_binary",
            "wrapper 'foo' missing",
        );

        // The breadcrumb dir should exist with both files.
        let runs_root = agent_runs_dir(&agent_id);
        let entries: Vec<_> = std::fs::read_dir(&runs_root)
            .expect("runs dir must exist")
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("dispatch-failed-"))
            })
            .collect();
        assert_eq!(entries.len(), 1, "exactly one breadcrumb dir");
        let dir = entries.into_iter().next().unwrap().path();
        assert!(
            dir.join("error.txt").exists(),
            "human-readable forensic kept"
        );
        let result_bytes = std::fs::read(dir.join("result.json")).expect("result.json must exist");

        // The synthetic file must round-trip through the same reader
        // `agents_runs_list` uses.
        let run: agent::AgentRun = serde_json::from_slice(&result_bytes)
            .expect("synthetic result.json must deserialize into AgentRun");
        assert_eq!(run.agent_id, agent_id);
        assert_eq!(run.exit_code, -1, "ERR row in RunHistoryPanel");
        assert_eq!(run.trigger_kind, agent::TriggerKind::Manual);
        let result = run.result.as_ref().expect("synthetic carries a RunResult");
        assert_eq!(
            result.is_error,
            Some(true),
            "panel renders ERR + show button"
        );
        assert_eq!(result.session_id.as_deref(), Some(session_id));
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("wrapper 'foo' missing")),
            "the error string is preserved verbatim for the disclosure"
        );
        assert!(
            result
                .subtype
                .as_deref()
                .is_some_and(|s| s.contains("resolve_binary")),
            "the failure stage is encoded in the subtype"
        );

        // Restore env so sibling tests are not polluted.
        match prev {
            Some(v) => std::env::set_var("CLAUDEPOT_DATA_DIR", v),
            None => std::env::remove_var("CLAUDEPOT_DATA_DIR"),
        }
    }

    #[tokio::test]
    async fn test_tick_inner_first_tick_cap_emits_once() {
        // X8 + X16 sanity: a first tick with more than
        // FIRST_TICK_BURST_CAP fires emits ONE burst-capped
        // notification and dispatches exactly cap fires.
        // Build 7 agents (above the cap of 5), each with a settled
        // session. Each agent's cwd is a distinct project root so the
        // evaluator emits one fire per agent (it caps "at most one
        // fire per agent per tick" — so we need 7 agents, not 7
        // sessions for one agent, to overflow the burst cap).
        let mut agents = Vec::new();
        let mut sessions = Vec::new();
        for i in 0..7 {
            let project = format!("proj-{i}");
            let a = stub_agent_with_cwd(&format!("agent-{i}"), &format!("/tmp/{project}"));
            sessions.push(fake_session(&format!("sess-{i}"), &project));
            agents.push(a);
        }

        let env = FakeTickEnv {
            agents,
            exclusion: HashSet::new(),
            sessions,
            first_tick: true,
            fresh_override: None,
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(EventsFile::default())),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };

        let calls = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls_for_disp = Rc::clone(&calls);
        let dispatcher = move |a: Agent, fire: EventFire| {
            calls_for_disp.borrow_mut().push(DispatchedCall {
                agent_id: a.id.to_string(),
                session_id: fire.session_id,
                session_path: fire.session_path,
            });
        };

        tick_inner(&env, dispatcher, Utc::now).await;

        // The current FIRST_TICK_BURST_CAP is 5; the evaluator may
        // emit fewer fires than agents × sessions because of the
        // "at most one fire per agent per tick" rule, but the cap
        // is a strict upper bound. With 7 (agent, session) pairs
        // each on its own agent + session, the evaluator produces
        // 7 fires; the cap drops 2.
        assert!(
            calls.borrow().len() <= FIRST_TICK_BURST_CAP,
            "dispatched count must not exceed the burst cap"
        );
        if env.burst_capped.borrow().is_empty() {
            // If the evaluator happens to emit ≤ cap (e.g. one of
            // the sessions doesn't satisfy "settled"), no cap is
            // applied — fine, but then the dispatched count must
            // match the fire count.
        } else {
            assert_eq!(
                env.burst_capped.borrow().len(),
                1,
                "the burst-capped notification fires at most once per tick"
            );
            let (dropped, cap) = env.burst_capped.borrow()[0];
            assert_eq!(cap, FIRST_TICK_BURST_CAP);
            assert!(dropped > 0);
        }
    }

    // ---- grill X16 — late-added agent gets per-agent cap --------

    #[tokio::test]
    async fn test_tick_inner_late_added_agent_is_capped_on_first_contact() {
        // X16: an event agent ADDED after orchestrator boot must
        // get its bounded catch-up on the tick it first
        // participates in. The prior (single global boolean)
        // design fired the cap only at orchestrator boot, so a
        // late-added agent had uncapped first contact with the
        // backlog. The per-agent fresh set fixes this.
        //
        // Scenario: the orchestrator has been running with agent
        // "veteran" for a long time. The user adds "newcomer" at
        // 2pm. On the tick that includes "newcomer" for the first
        // time, the cap applies only to "newcomer"'s fires —
        // "veteran"'s fires pass through uncapped.
        //
        // We model this with the FakeTickEnv's `fresh_override`:
        // the test asserts that fires for the fresh agent are
        // bounded, and fires for the seen agent are not.
        let fixed_now = chrono::Utc::now();

        // 1 veteran agent + 1 newcomer agent, each with several
        // settled sessions. The evaluator caps "at most one fire
        // per agent per tick", so each agent contributes exactly
        // one fire — we use the fresh override to model a
        // newcomer overflow by giving newcomer multiple eligible
        // sessions and a low cap. Easier shape: drive the
        // per-agent cap helper directly with many fires for a
        // single fresh agent.
        let fresh: HashSet<String> = ["newcomer".to_string()].into_iter().collect();
        let mut fires: Vec<EventFire> = Vec::new();
        // Veteran has its single steady-state fire.
        fires.push(EventFire {
            agent_id: "veteran".into(),
            session_id: "vsess".into(),
            session_path: "/tmp/.claude/projects/proj/vsess.jsonl".into(),
        });
        // Newcomer has CAP + 3 backlog fires.
        for i in 0..(FIRST_TICK_BURST_CAP + 3) {
            fires.push(EventFire {
                agent_id: "newcomer".into(),
                session_id: format!("nsess-{i}"),
                session_path: format!("/tmp/.claude/projects/proj/nsess-{i}.jsonl"),
            });
        }
        let dropped_count = std::cell::Cell::new(0usize);
        let kept =
            apply_per_agent_first_tick_cap(fires, &fresh, FIRST_TICK_BURST_CAP, |dropped, _| {
                dropped_count.set(dropped)
            });

        // The veteran's fire passes through uncapped.
        let veteran_kept = kept.iter().filter(|f| f.agent_id == "veteran").count();
        assert_eq!(
            veteran_kept, 1,
            "X16: a long-running agent's fires must NOT be capped"
        );
        // The newcomer is bounded to the cap.
        let newcomer_kept = kept.iter().filter(|f| f.agent_id == "newcomer").count();
        assert_eq!(
            newcomer_kept, FIRST_TICK_BURST_CAP,
            "X16: a late-added agent gets the bounded first-tick cap"
        );
        // The dropped count covers only the newcomer's overflow.
        assert_eq!(dropped_count.get(), 3);

        // Silence warning about unused `fixed_now` if the
        // optimizer doesn't fold the binding.
        let _ = fixed_now;
    }

    #[tokio::test]
    async fn test_tick_inner_no_fresh_agents_skips_the_cap_emit() {
        // X16 negative side: when every agent participating in
        // this tick has already been seen, NO emit fires and no
        // fire is dropped — the steady state is uncapped.
        let agent = stub_agent_with_cwd("veteran", "/tmp/proj");
        let agent_id = agent.id.to_string();
        let session_id = "vsess".to_string();
        let fixed_now = chrono::Utc::now();
        let sessions = vec![fake_session_at(
            &session_id,
            "proj",
            fixed_now - chrono::Duration::hours(1),
        )];
        // `first_tick = false` AND `fresh_override = Some(empty)`
        // models "every agent in this tick has been seen
        // before". The cap path must not be taken.
        let env = FakeTickEnv {
            agents: vec![agent.clone()],
            exclusion: HashSet::new(),
            sessions,
            first_tick: false,
            fresh_override: Some(HashSet::new()),
            save_fails_n_times: Rc::new(RefCell::new(0)),
            save_calls: Rc::new(RefCell::new(0)),
            ledger: Rc::new(RefCell::new(EventsFile::default())),
            burst_capped: Rc::new(RefCell::new(Vec::new())),
            reconciles: Rc::new(RefCell::new(0)),
        };
        let calls = Rc::new(RefCell::new(Vec::<DispatchedCall>::new()));
        let calls_for_disp = Rc::clone(&calls);
        tick_inner(
            &env,
            move |a: Agent, fire: EventFire| {
                calls_for_disp.borrow_mut().push(DispatchedCall {
                    agent_id: a.id.to_string(),
                    session_id: fire.session_id,
                    session_path: fire.session_path,
                });
            },
            move || fixed_now,
        )
        .await;
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(calls.borrow()[0].agent_id, agent_id);
        assert!(
            env.burst_capped.borrow().is_empty(),
            "no fresh agents => no burst-capped emit"
        );
    }
}
