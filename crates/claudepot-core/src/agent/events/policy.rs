//! Pure orchestration policy — the Tauri-free half of the event
//! orchestrator's decisions, sibling of [`super::eval`].
//!
//! The runtime bridge (`src-tauri/src/agent_event_orchestrator.rs`)
//! collects the real inputs (store, ledger, session index) and
//! dispatches `claude -p` runs; everything here is pure: core types
//! in, decisions out, clocks and I/O injected. This is the rotation
//! pattern from `.claude/rules/architecture.md` — policy in core,
//! wiring in the bridge.
//!
//! The F14 / F17 constraints documented on the functions below are
//! load-bearing; see the module doc of [`super`] (`agent::events`)
//! for the full contract.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Duration as ChronoDuration, Utc};

use crate::agent::types::Agent;

use super::eval::{AgentRunStats, EventFire};
use super::store::EventsFile;

/// Hard cap on the first-tick burst. The CC session index a long-
/// idle Claudepot installation re-discovers can contain dozens of
/// "settled" sessions that all became eligible while Claudepot was
/// closed; without a cap the very first tick after launch would
/// fire one run per settled session, in one go, for every event-
/// triggered agent. The cap bounds blast radius to ~5 billed
/// `claude -p` runs across the whole machine the first time an
/// agent participates in a tick. Subsequent ticks for that agent
/// run uncapped — the steady state is at most one fire per agent
/// per tick (the evaluator already enforces this).
pub const FIRST_TICK_BURST_CAP: usize = 5;

/// **F17** — pure exclusion-set construction with pluggable I/O.
/// The `read_run_session_ids` closure returns the parsed
/// `RunResult.session_id` (one slot per run, `None` for runs that
/// produced no session id). The closure is the I/O seam used by the
/// orchestrator's `build_self_exclusion_set` wrapper; tests pass
/// a fixture closure to verify the F17 invariant without touching
/// the filesystem.
pub fn build_self_exclusion_set_with(
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
pub fn build_run_stats_from_ledger(
    ledger: &EventsFile,
    now: DateTime<Utc>,
) -> HashMap<String, AgentRunStats> {
    let one_day_ago = now - ChronoDuration::days(1);
    let mut out: HashMap<String, AgentRunStats> = HashMap::new();
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

/// Apply the per-agent first-tick burst cap (grill X16).
///
/// `fresh_agents` is the subset of agents that have NEVER
/// participated in a tick this process before — agents added since
/// orchestrator boot are tracked here just like boot-time agents.
/// The cap bounds the **total fires emitted by fresh agents** on
/// this tick (the "first contact blast radius" the cap was always
/// designed to bound — the old global cap was the same number,
/// just scoped to boot-time). Fires belonging to already-seen
/// agents pass through uncapped: the evaluator's "at most one fire
/// per agent per tick" guard suffices for the steady state.
///
/// Returns the kept fires in input order. The emit closure is
/// invoked at most once per call with the total dropped count.
/// Tests pass a fixture closure to observe without touching the
/// Tauri `AppHandle`.
pub fn apply_per_agent_first_tick_cap<F>(
    fires: Vec<EventFire>,
    fresh_agents: &HashSet<String>,
    cap: usize,
    emit: F,
) -> Vec<EventFire>
where
    F: FnOnce(usize, usize),
{
    // Fast path: if no fire belongs to a fresh agent, nothing to
    // cap.
    let any_fresh = fires.iter().any(|f| fresh_agents.contains(&f.agent_id));
    if !any_fresh {
        return fires;
    }
    let mut kept = Vec::with_capacity(fires.len());
    let mut fresh_kept = 0usize;
    let mut dropped = 0usize;
    for fire in fires {
        let is_fresh = fresh_agents.contains(&fire.agent_id);
        if is_fresh {
            if fresh_kept >= cap {
                dropped += 1;
                continue;
            }
            fresh_kept += 1;
        }
        kept.push(fire);
    }
    if dropped > 0 {
        tracing::warn!(
            cap,
            dropped,
            fresh_agents = fresh_agents.len(),
            "agent events: per-agent first-tick burst capped"
        );
        emit(dropped, cap);
    }
    kept
}

/// Build the `extra_env` map a session-settled dispatch passes to
/// `claude -p`. Two keys, both verbatim from the [`EventFire`]:
///
/// - `CLAUDEPOT_EVENT_SESSION_ID` — the CC session UUID.
/// - `CLAUDEPOT_EVENT_SESSION_PATH` — absolute transcript path.
///
/// Factored out so X8/T8 can lock the contract down without spawning
/// a real shim. A rename on one side (here, or in the shim that
/// reads it) ships green only if **both** sides update; the
/// test in `tests::test_dispatch_env_round_trip` is the gate.
pub fn build_dispatch_env(fire: &EventFire) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert(
        "CLAUDEPOT_EVENT_SESSION_ID".to_string(),
        fire.session_id.clone(),
    );
    env.insert(
        "CLAUDEPOT_EVENT_SESSION_PATH".to_string(),
        fire.session_path.clone(),
    );
    env
}

// ---------------------------------------------------------------------------
// Tests — migrated with the policy functions from
// `agent_event_orchestrator::tests` (the orchestrator keeps the wired
// `tick_inner` integration tests).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::FiredEntry;
    use crate::agent::Lifecycle;
    use chrono::TimeZone;

    fn ts(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap() + ChronoDuration::minutes(min)
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

    // ---- grill X16 — late-added agent gets per-agent cap --------

    #[test]
    fn test_late_added_agent_is_capped_on_first_contact() {
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
        let exclusion = build_self_exclusion_set_with(&[a.clone(), b.clone()], &|agent| {
            if agent.name == "agent-1" {
                vec![Some("sess-A".to_string()), Some("sess-B".to_string())]
            } else {
                vec![Some("sess-C".to_string())]
            }
        });
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

    #[test]
    fn test_dispatch_env_round_trip() {
        // X8(c) / X28 / T8: `CLAUDEPOT_EVENT_SESSION_ID` and
        // `CLAUDEPOT_EVENT_SESSION_PATH` are the contract between
        // the orchestrator and the shim that `claude -p` runs. The
        // shim reads these env vars verbatim; a rename on one side
        // and not the other would ship green without this gate.
        let fire = EventFire {
            agent_id: "a1".into(),
            session_id: "deadbeef-cafe".into(),
            session_path: "/home/u/.claude/projects/proj/deadbeef-cafe.jsonl".into(),
        };
        let env = build_dispatch_env(&fire);
        assert_eq!(
            env.get("CLAUDEPOT_EVENT_SESSION_ID").map(String::as_str),
            Some("deadbeef-cafe")
        );
        assert_eq!(
            env.get("CLAUDEPOT_EVENT_SESSION_PATH").map(String::as_str),
            Some("/home/u/.claude/projects/proj/deadbeef-cafe.jsonl")
        );
        assert_eq!(env.len(), 2, "no other env vars are injected");
    }

    fn stub_agent(name: &str) -> Agent {
        use crate::agent::{
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

    /// Test-only adapter that drops the emit side-effect — pure
    /// logic. Routes through the production helper so the cap
    /// behavior is the same in tests and prod. The per-agent cap
    /// (grill X16) treats every distinct `agent_id` in the input as
    /// "fresh" for the legacy tests below, so the older
    /// "global cap" semantics are preserved when each fire belongs
    /// to a different agent.
    fn apply_first_tick_cap_no_emit(fires: Vec<EventFire>, cap: usize) -> Vec<EventFire> {
        let fresh: HashSet<String> = fires.iter().map(|f| f.agent_id.clone()).collect();
        apply_per_agent_first_tick_cap(fires, &fresh, cap, |_, _| {})
    }
}
