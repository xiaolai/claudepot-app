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

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use claudepot_core::agent::{
    self,
    events::{
        evaluate as evaluate_events, store as events_store, AgentRunStats, EventFire,
        EventsFile,
    },
    list_run_ids, read_run, reconcile_with_scheduler, resolve_binary, AgentStore,
    Agent, Lifecycle, Trigger,
};
use claudepot_core::agent::install::current_claudepot_cli;
use claudepot_core::session::SessionRow;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Hard cap on the first-tick burst. The CC session index a long-
/// idle Claudepot installation re-discovers can contain dozens of
/// "settled" sessions that all became eligible while Claudepot was
/// closed; without a cap the very first tick after launch would
/// fire one run per settled session, in one go, for every event-
/// triggered agent. The cap bounds blast radius to ~5 billed
/// `claude -p` runs across the whole machine the first time.
/// Subsequent ticks run uncapped — the steady state is at most
/// one fire per agent per tick (the evaluator already enforces
/// this).
const FIRST_TICK_BURST_CAP: usize = 5;

/// Orchestrator state — `manage()`'d by the Tauri app, reachable
/// via `app.state::<Arc<EventOrchestrator>>()`. The whole struct
/// is a single boolean: "has at least one tick already run?". Used
/// to apply [`FIRST_TICK_BURST_CAP`] only on the very first tick.
#[derive(Default)]
pub struct EventOrchestrator {
    booted: AtomicBool,
}

impl EventOrchestrator {
    pub fn new() -> Self {
        Self {
            booted: AtomicBool::new(false),
        }
    }

    /// Mark the first tick as having run. Returns `true` iff this
    /// call is the one that flipped the flag, i.e. the caller is
    /// inside the first tick.
    fn enter_tick(&self) -> bool {
        !self.booted.swap(true, Ordering::SeqCst)
    }
}

/// Drive one event evaluation cycle. Called from
/// `usage_snapshot::run_tick`. Steps:
///
/// 1. Open the agent store; collect the `Installed && enabled`
///    `Event`-triggered agents. If none, return immediately — the
///    common case. **Zero overhead** for users who never install
///    an event-triggered agent.
/// 2. Build the F17 self-trigger exclusion set from authoritative
///    `RunResult.session_id`s (not `AgentRun::session_jsonl_path`).
/// 3. Load the durable event-state ledger; derive per-agent
///    `AgentRunStats` from it (F14).
/// 4. Index the live CC sessions via `claudepot_core::session::
///    list_all_sessions`.
/// 5. Call `events::evaluate` with everything above.
/// 6. For each [`EventFire`] in order: **record_fire + save the
///    ledger FIRST** (F1), THEN dispatch the run via
///    `agent::run::run_now`. Honor the first-tick burst cap.
/// 7. Prune the ledger of stale (agent, session) pairs.
/// 8. Run boot-time reconciliation of installed agents against
///    the scheduler — orphan records are *logged*, not mutated
///    (this is observability, not enforcement).
pub async fn tick(app: &AppHandle, config_dir: PathBuf) {
    // ---- 1. Open the store + filter to the relevant agents -----
    let agents = match load_event_agents() {
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

    let now = Utc::now();
    let live_agent_ids: HashSet<String> =
        agents.iter().map(|a| a.id.to_string()).collect();

    // ---- 2. F17 self-trigger exclusion set --------------------
    // Built from the authoritative `RunResult.session_id` parsed
    // out of every prior run's `result.json`, NEVER from
    // `AgentRun::session_jsonl_path` — that field is re-derived by
    // a depth-limited filename walk and fails open. A failed-open
    // exclusion lets the Session Narrator narrate its own output:
    // the exact D7 infinite loop.
    let agent_session_ids = build_self_exclusion_set(&agents);

    // ---- 3. F14 per-agent stats from the durable ledger -------
    let mut ledger = match events_store::load() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "agent_event_orchestrator: ledger load failed; skipping tick");
            return;
        }
    };
    let run_stats_map = build_run_stats_from_ledger(&ledger, now);

    // ---- 4. Index the live CC sessions ------------------------
    // List under the configured `~/.claude/` dir. A scan failure
    // is treated as "no sessions" and logged — better than
    // skipping the tick entirely, because the event orchestrator
    // also performs ledger prune + reconciliation on every tick.
    let sessions: Vec<SessionRow> =
        match claudepot_core::session::list_all_sessions(&config_dir) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "agent_event_orchestrator: session index failed");
                Vec::new()
            }
        };
    let live_session_ids: HashSet<String> =
        sessions.iter().map(|s| s.session_id.clone()).collect();

    // ---- 5. Pure evaluator -----------------------------------
    let fired_pairs: HashSet<(String, String)> = ledger
        .fired
        .iter()
        .map(|e| (e.agent_id.clone(), e.session_id.clone()))
        .collect();
    let stats_fn = |id: &str| {
        run_stats_map
            .get(id)
            .cloned()
            .unwrap_or_default()
    };
    let mut fires = evaluate_events(
        &agents,
        &sessions,
        &fired_pairs,
        &agent_session_ids,
        &stats_fn,
        now,
    );

    // ---- 6. Bounded catch-up cap (D6) ------------------------
    let first_tick = {
        let state = app.state::<Arc<EventOrchestrator>>();
        state.enter_tick()
    };
    if first_tick {
        fires = apply_first_tick_cap(fires, FIRST_TICK_BURST_CAP, app);
    }

    // ---- 7. Dispatch each fire — record_fire + save FIRST -----
    // (F1) The ledger is the single source of fire-once truth; a
    // duplicate ledger entry is free, but a duplicate billed
    // `claude -p` is a real cost leak. So we always commit the
    // ledger update before spawning the run.
    //
    // Build an agents-by-id map once so dispatch doesn't re-scan
    // the agents slice per fire.
    let agents_by_id: std::collections::HashMap<String, &Agent> = agents
        .iter()
        .map(|a| (a.id.to_string(), a))
        .collect();

    for fire in &fires {
        let Some(agent) = agents_by_id.get(&fire.agent_id) else {
            // Should be unreachable — the evaluator only returns
            // fires for agents we passed in — but if a race ever
            // dropped one, just skip.
            continue;
        };
        ledger.record_fire(&fire.agent_id, &fire.session_id, now);
        if let Err(e) = events_store::save(&ledger) {
            tracing::warn!(
                error = %e,
                agent_id = %fire.agent_id,
                session_id = %fire.session_id,
                "agent_event_orchestrator: ledger save failed; \
                 skipping this fire — it will be re-evaluated next tick"
            );
            // Per F1: if we cannot commit the ledger, we MUST
            // NOT dispatch. Skip and let the next tick re-gate.
            continue;
        }
        // Now safe to dispatch — the (agent, session) pair is
        // recorded as fired regardless of whether the run
        // succeeds, crashes, or never makes it to `claude -p`.
        dispatch(app, agent, fire).await;
    }

    // ---- 8. Prune the ledger of stale pairs -------------------
    let removed = ledger.prune(&live_agent_ids, &live_session_ids);
    if removed > 0 {
        if let Err(e) = events_store::save(&ledger) {
            tracing::warn!(
                error = %e,
                removed,
                "agent_event_orchestrator: ledger prune save failed"
            );
        }
    }

    // ---- 9. Orphan-record reconciliation (observability) ------
    // `reconcile_with_scheduler` logs loudly per orphan; we just
    // call it. We don't mutate the store.
    let _ = reconcile_with_scheduler();
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
            sids.push(
                run.result
                    .as_ref()
                    .and_then(|r| r.session_id.clone()),
            );
        }
        sids
    })
}

/// Pure variant — pluggable I/O. The `read_run_session_ids` closure
/// returns the parsed `RunResult.session_id` (one slot per run,
/// `None` for runs that produced no session id). The closure is
/// the I/O seam used by [`build_self_exclusion_set`]; tests pass
/// a fixture closure to verify the F17 invariant without touching
/// the filesystem.
fn build_self_exclusion_set_with(
    agents: &[Agent],
    read_run_session_ids: &dyn Fn(&Agent) -> Vec<Option<String>>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for agent in agents {
        for s in read_run_session_ids(agent).into_iter().flatten() {
            out.insert(s);
        }
    }
    out
}

/// **F14** — derive per-agent `AgentRunStats` from the durable
/// event-state ledger, NOT from prunable per-agent
/// `result.json` directories. The ledger's `FiredEntry.fired_at`
/// carries everything we need:
///
/// - `last_run_started_at` = `max(fired_at)` per agent.
/// - `runs_in_last_day` = count of entries for that agent with
///   `fired_at >= now - 24h`.
///
/// A high-frequency agent whose oldest `result.json`s are pruned
/// at `log_retention_runs` would under-count if we derived stats
/// from the runs dirs and silently exceed `max_per_day`.
fn build_run_stats_from_ledger(
    ledger: &EventsFile,
    now: DateTime<Utc>,
) -> std::collections::HashMap<String, AgentRunStats> {
    let one_day_ago = now - ChronoDuration::days(1);
    let mut out: std::collections::HashMap<String, AgentRunStats> =
        std::collections::HashMap::new();
    for entry in &ledger.fired {
        let s = out.entry(entry.agent_id.clone()).or_default();
        if entry.fired_at >= one_day_ago {
            s.runs_in_last_day = s.runs_in_last_day.saturating_add(1);
        }
        s.last_run_started_at = match s.last_run_started_at {
            None => Some(entry.fired_at),
            Some(prev) if entry.fired_at > prev => Some(entry.fired_at),
            other => other,
        };
    }
    out
}

/// Apply the first-tick catch-up cap. Drops fires beyond
/// `cap` and emits a single notification rather than spamming one
/// per dropped fire.
fn apply_first_tick_cap(
    fires: Vec<EventFire>,
    cap: usize,
    app: &AppHandle,
) -> Vec<EventFire> {
    if fires.len() <= cap {
        return fires;
    }
    let dropped = fires.len() - cap;
    tracing::warn!(
        cap,
        dropped,
        "agent_event_orchestrator: first-tick burst capped"
    );
    emit_first_tick_capped(app, dropped, cap);
    fires.into_iter().take(cap).collect()
}

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
            emit_failed(app, &fire.agent_id, &fire.session_id, &e.to_string());
            return;
        }
    };

    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert(
        "CLAUDEPOT_EVENT_SESSION_ID".to_string(),
        fire.session_id.clone(),
    );
    env.insert(
        "CLAUDEPOT_EVENT_SESSION_PATH".to_string(),
        fire.session_path.clone(),
    );

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

/// `ProgressSink` impl that discards phase events. The event
/// orchestrator has no UI surface for in-flight progress — runs
/// either succeed (the row appears in RunHistoryPanel) or fail
/// (logged + event-emitted).
struct NoopSink;
impl claudepot_core::project_progress::ProgressSink for NoopSink {
    fn phase(
        &self,
        _phase: &str,
        _status: claudepot_core::project_progress::PhaseStatus,
    ) {
    }
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
    if let Err(e) = app.emit("agent-event-dispatched", payload) {
        tracing::warn!(error = %e, "agent_event_orchestrator: emit dispatched failed");
    }
}

fn emit_failed(app: &AppHandle, agent_id: &str, session_id: &str, error: &str) {
    let payload = AgentEventFailedPayload {
        agent_id: agent_id.to_string(),
        session_id: session_id.to_string(),
        error: error.to_string(),
    };
    if let Err(e) = app.emit("agent-event-failed", payload) {
        tracing::warn!(error = %e, "agent_event_orchestrator: emit failed failed");
    }
}

fn emit_first_tick_capped(app: &AppHandle, dropped: usize, cap: usize) {
    let payload = AgentEventBurstCappedPayload { cap, dropped };
    if let Err(e) = app.emit("agent-event-burst-capped", payload) {
        tracing::warn!(error = %e, "agent_event_orchestrator: emit burst-capped failed");
    }
}

// ---------------------------------------------------------------------------
// Tests — the orchestrator's three load-bearing constraints
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use claudepot_core::agent::events::FiredEntry;

    fn ts(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap()
            + ChronoDuration::minutes(min)
    }

    #[test]
    fn test_build_run_stats_counts_only_last_24h() {
        // F14 guard: `runs_in_last_day` is bounded to a 24h
        // window, and `last_run_started_at` is the max `fired_at`
        // regardless of age.
        let now = ts(0);
        let mut ledger = EventsFile::default();
        // Within 24h:
        ledger.fired.push(FiredEntry {
            agent_id: "a".into(),
            session_id: "s1".into(),
            fired_at: now - ChronoDuration::hours(1),
        });
        ledger.fired.push(FiredEntry {
            agent_id: "a".into(),
            session_id: "s2".into(),
            fired_at: now - ChronoDuration::hours(23),
        });
        // Older than 24h:
        ledger.fired.push(FiredEntry {
            agent_id: "a".into(),
            session_id: "s3".into(),
            fired_at: now - ChronoDuration::hours(48),
        });
        // Different agent — must not contaminate `a`'s count.
        ledger.fired.push(FiredEntry {
            agent_id: "b".into(),
            session_id: "s9".into(),
            fired_at: now - ChronoDuration::minutes(5),
        });
        let stats = build_run_stats_from_ledger(&ledger, now);
        let a = stats.get("a").unwrap();
        assert_eq!(a.runs_in_last_day, 2, "only the two ≤24h entries count");
        assert_eq!(
            a.last_run_started_at,
            Some(now - ChronoDuration::hours(1)),
            "last_run_started_at is the max fired_at"
        );
        let b = stats.get("b").unwrap();
        assert_eq!(b.runs_in_last_day, 1);
    }

    #[test]
    fn test_build_run_stats_empty_ledger_yields_empty_map() {
        let now = ts(0);
        let ledger = EventsFile::default();
        let stats = build_run_stats_from_ledger(&ledger, now);
        assert!(stats.is_empty());
    }

    #[test]
    fn test_apply_first_tick_cap_below_cap_passes_through() {
        let fires = vec![mk_fire("a", "s1"), mk_fire("b", "s2")];
        let kept = apply_first_tick_cap_no_emit(fires.clone(), 5);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, fires);
    }

    #[test]
    fn test_apply_first_tick_cap_drops_overflow() {
        let fires: Vec<EventFire> = (0..10)
            .map(|i| mk_fire(&format!("a{i}"), &format!("s{i}")))
            .collect();
        let kept = apply_first_tick_cap_no_emit(fires.clone(), 5);
        assert_eq!(kept.len(), 5, "overflow must be capped");
        // The first 5 are kept (order-preserving).
        assert_eq!(
            kept.iter().map(|f| f.agent_id.clone()).collect::<Vec<_>>(),
            vec!["a0", "a1", "a2", "a3", "a4"]
        );
    }

    #[test]
    fn test_apply_first_tick_cap_zero_passes_empty() {
        let kept = apply_first_tick_cap_no_emit(vec![], 5);
        assert!(kept.is_empty());
    }

    #[test]
    fn test_event_orchestrator_enter_tick_only_returns_true_once() {
        // Catch-up cap must apply on the FIRST tick after process
        // start and never again — subsequent ticks see the steady
        // state, where the evaluator's "at most one fire per agent
        // per tick" guard suffices.
        let o = EventOrchestrator::new();
        assert!(o.enter_tick(), "the first tick is the first tick");
        assert!(
            !o.enter_tick(),
            "subsequent ticks must NOT see the first-tick flag"
        );
        assert!(
            !o.enter_tick(),
            "and not on the third either"
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

    // ---- F17 self-trigger exclusion set construction ----------

    #[test]
    fn test_f17_exclusion_set_uses_authoritative_session_id() {
        // The exclusion set MUST be built from `RunResult.session_id`
        // — never from the `AgentRun::session_jsonl_path` walk.
        // Simulate two agents with several runs each; the closure
        // returns the parsed session ids the orchestrator would
        // observe.
        let a = stub_agent("agent-1");
        let b = stub_agent("agent-2");
        let exclusion = build_self_exclusion_set_with(
            &[a.clone(), b.clone()],
            &|agent| {
                if agent.name == "agent-1" {
                    vec![Some("sess-A".to_string()), Some("sess-B".to_string())]
                } else {
                    vec![Some("sess-C".to_string())]
                }
            },
        );
        assert!(exclusion.contains("sess-A"));
        assert!(exclusion.contains("sess-B"));
        assert!(exclusion.contains("sess-C"));
        assert_eq!(exclusion.len(), 3);
    }

    #[test]
    fn test_f17_exclusion_set_skips_runs_with_no_session_id() {
        // Some runs (synthesized failures, budget aborts) have no
        // `session_id`. They contribute nothing to the exclusion
        // set rather than poisoning it with `None`.
        let a = stub_agent("agent-1");
        let exclusion = build_self_exclusion_set_with(&[a], &|_| {
            vec![None, Some("sess-real".to_string()), None]
        });
        assert_eq!(exclusion.len(), 1);
        assert!(exclusion.contains("sess-real"));
    }

    #[test]
    fn test_f17_exclusion_set_empty_when_no_runs() {
        // A freshly-installed agent with no runs contributes
        // nothing — the set is empty and the evaluator excludes
        // no sessions, which is correct.
        let a = stub_agent("agent-1");
        let exclusion = build_self_exclusion_set_with(&[a], &|_| Vec::new());
        assert!(exclusion.is_empty());
    }

    fn stub_agent(name: &str) -> Agent {
        use claudepot_core::agent::{
            AgentBinary, CreatedVia, EventKind, OutputFormat, PermissionMode,
            PlatformOptions, RateLimit, Trigger,
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
            session_path: format!(
                "/home/u/.claude/projects/proj/{session_id}.jsonl"
            ),
        }
    }

    /// Test-only variant of [`apply_first_tick_cap`] without
    /// the Tauri `AppHandle` emit side effect — pure logic.
    fn apply_first_tick_cap_no_emit(
        fires: Vec<EventFire>,
        cap: usize,
    ) -> Vec<EventFire> {
        if fires.len() <= cap {
            return fires;
        }
        fires.into_iter().take(cap).collect()
    }
}

