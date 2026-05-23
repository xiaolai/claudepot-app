//! Pure `session-settled` evaluator (PRD §7).
//!
//! Mirrors `rotation::eval`'s shape exactly: a pure function with the
//! clock injected, no I/O, fully unit-testable. It maps
//! `(sessions, ledger, event-agents, run-stats, exclusion-set, now)`
//! to the list of `(agent, session)` pairs that should fire.
//!
//! The in-app event orchestrator (the Tauri layer) collects every
//! input — indexes the sessions, loads the ledger, walks each
//! agent's run history, builds the agent-produced-session exclusion
//! set — and then calls [`evaluate`]. All filesystem and Tauri
//! concerns live there; this module is pure.
//!
//! A session fires for an agent when **all** of these hold:
//!
//! 1. **Settled** — the session has been idle (transcript unchanged)
//!    for at least the trigger's `debounce_secs`.
//! 2. **Fire-once** — the `(agent_id, session_id)` pair is not
//!    already in the ledger.
//! 3. **Self-trigger exclusion (D7)** — the session was NOT produced
//!    by an agent run. Without this the Session Narrator narrates
//!    its own output forever.
//! 4. **Rate-limit (D9)** — the agent's `rate_limit` permits another
//!    run now (min interval elapsed; max-per-day not exceeded).
//! 5. **Scope** — the settled session's project matches the agent's
//!    `cwd` project, so an agent narrates sessions in its own
//!    project, not every session on the machine.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::agent::types::{Agent, EventKind, RateLimit, Trigger};
use crate::path_utils::simplify_windows_path;
use crate::session::SessionRow;

/// One (agent, session) pair the evaluator decided should fire.
/// Carries everything the orchestrator needs to dispatch the run +
/// record the ledger entry without re-querying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFire {
    /// The `Event`-triggered agent to run.
    pub agent_id: String,
    /// CC/Codex `session_id` of the settled session.
    pub session_id: String,
    /// Absolute transcript path of the settled session — passed to
    /// the run as `CLAUDEPOT_EVENT_SESSION_PATH`.
    pub session_path: String,
}

/// Per-agent run statistics the rate-limiter needs. The orchestrator
/// derives these from the agent's on-disk run history; the evaluator
/// stays pure by receiving them pre-computed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRunStats {
    /// When the agent most recently *started* a run, if ever.
    pub last_run_started_at: Option<DateTime<Utc>>,
    /// How many runs the agent started in the rolling 24h window
    /// ending at `now`.
    pub runs_in_last_day: u32,
}

/// Why an `Event`-triggered agent could not fire for a settled
/// session. Returned only for forensics / tests — the orchestrator
/// acts solely on [`EventFire`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Min-interval rate limit blocked the fire.
    RateLimitedMinInterval { secs_since_last: i64 },
    /// Max-per-day rate limit blocked the fire.
    RateLimitedMaxPerDay { runs_today: u32 },
}

/// Pure evaluator. Returns the (agent, session) pairs that should
/// fire this tick. Tests inject `now`.
///
/// **At most one `EventFire` per agent per tick.** The evaluator is
/// pure — within one tick it cannot observe the runs it would itself
/// authorize, so it cannot let one agent fire for several sessions at
/// once without blowing past `max_per_day` / `min_interval` (the D9
/// cost-runaway guard). Each agent therefore fires for the single
/// **oldest-settled** eligible session; the remaining settled
/// sessions fire on later ticks, each re-gated by the rate limit.
/// Oldest-first is also deterministic and fair — the session idle
/// longest is narrated first.
///
/// Arguments:
/// - `agents` — the **installed** agents carrying an `Event` trigger.
///   The caller filters to `lifecycle == Installed && enabled`; a
///   draft must never fire (PRD §8.2). Cron/Manual agents are
///   silently ignored if passed.
/// - `sessions` — every indexed session row.
/// - `fired_pairs` — the set of `(agent_id, session_id)` pairs that
///   have already fired, from the event-state ledger.
/// - `agent_session_ids` — `session_id`s produced by agent runs.
///   These are excluded from evaluation (self-trigger exclusion).
/// - `run_stats` — `(agent_id, stats)` lookup for the rate-limiter.
/// - `now` — the wall clock.
pub fn evaluate(
    agents: &[Agent],
    sessions: &[SessionRow],
    fired_pairs: &HashSet<(String, String)>,
    agent_session_ids: &HashSet<String>,
    run_stats: &dyn Fn(&str) -> AgentRunStats,
    now: DateTime<Utc>,
) -> Vec<EventFire> {
    let mut out = Vec::new();
    for agent in agents {
        // Only `session-settled` event agents are evaluated here.
        let debounce_secs = match &agent.trigger {
            Trigger::Event {
                event: EventKind::SessionSettled { debounce_secs },
            } => *debounce_secs,
            _ => continue,
        };
        let agent_id = agent.id.to_string();

        // (4) Rate-limit (D9) — checked once per agent. Because the
        //     agent fires at most once per tick (below), a single
        //     check against its prior run stats is exact: there are
        //     no intra-tick fires for the stats to miss. A throttled
        //     agent fires for no session this tick.
        let stats = run_stats(&agent_id);
        if rate_limit_blocks(agent.rate_limit.as_ref(), &stats, now).is_some() {
            continue;
        }

        let agent_project = normalize_project(&agent.cwd);

        // Of every eligible settled session, fire for the single
        // oldest one. Eligibility, in cheapest-first order:
        //   (3) self-trigger exclusion (D7) — a session produced by
        //       an agent run never fires, or the Session Narrator
        //       would narrate its own output forever;
        //   (1) settled — transcript idle for >= debounce_secs;
        //   (5) scope — the session's project matches the agent cwd;
        //   (2) fire-once — the pair is not already in the ledger.
        let mut oldest: Option<&SessionRow> = None;
        for session in sessions {
            if agent_session_ids.contains(&session.session_id) {
                continue;
            }
            if !is_settled(session, debounce_secs, now) {
                continue;
            }
            if normalize_project(&session.project_path) != agent_project {
                continue;
            }
            if fired_pairs.contains(&(agent_id.clone(), session.session_id.clone())) {
                continue;
            }
            // Track the oldest-settled eligible session. Ties keep
            // the first seen, so the result is deterministic.
            match oldest {
                Some(best) if last_activity(best) <= last_activity(session) => {}
                _ => oldest = Some(session),
            }
        }

        if let Some(session) = oldest {
            out.push(EventFire {
                agent_id: agent_id.clone(),
                session_id: session.session_id.clone(),
                session_path: session.file_path.to_string_lossy().into_owned(),
            });
        }
    }
    out
}

/// A session is **settled** when its last activity is at least
/// `debounce_secs` in the past. "Last activity" prefers the
/// server-side `last_ts`; when the transcript carried no parseable
/// timestamp it falls back to the file's mtime (`last_modified`).
/// A session with neither is never considered settled — we cannot
/// prove it has stopped growing.
///
/// F20: the comparison runs on `u64` so a huge `debounce_secs`
/// never wraps. The earlier `idle_secs >= debounce_secs as i64`
/// shape silently wrapped `u64::MAX as i64` to `-1`, flipping an
/// extreme debounce into "fire immediately". A negative idle
/// duration (clock skew) cannot satisfy any positive debounce, so
/// it is returned as `false` directly.
pub fn is_settled(session: &SessionRow, debounce_secs: u64, now: DateTime<Utc>) -> bool {
    let last_activity = match last_activity(session) {
        Some(t) => t,
        None => return false,
    };
    let idle_secs = (now - last_activity).num_seconds();
    if idle_secs < 0 {
        return false;
    }
    // Safe: `idle_secs >= 0` was just checked, so the `as u64`
    // narrowing here is lossless.
    (idle_secs as u64) >= debounce_secs
}

/// Resolve a session's "last activity" instant.
fn last_activity(session: &SessionRow) -> Option<DateTime<Utc>> {
    if let Some(ts) = session.last_ts {
        return Some(ts);
    }
    // Fall back to the file mtime. `SystemTime` -> `DateTime<Utc>`.
    session.last_modified.map(|st| DateTime::<Utc>::from(st))
}

/// Returns `Some(reason)` when the agent's rate limit forbids a run
/// right now, `None` when a run is permitted.
fn rate_limit_blocks(
    limit: Option<&RateLimit>,
    stats: &AgentRunStats,
    now: DateTime<Utc>,
) -> Option<SkipReason> {
    let limit = limit?;
    if let Some(min_interval) = limit.min_interval_secs {
        if let Some(last) = stats.last_run_started_at {
            let secs_since_last = (now - last).num_seconds();
            // F20: compare on `u64` so a huge `min_interval` never
            // wraps. A negative `secs_since_last` (clock skew /
            // future-dated run history) is treated as "no interval
            // has elapsed at all" — block the fire rather than let
            // a sign flip slip past the limit.
            let elapsed_ok = secs_since_last >= 0 && (secs_since_last as u64) >= min_interval;
            if !elapsed_ok {
                return Some(SkipReason::RateLimitedMinInterval { secs_since_last });
            }
        }
    }
    if let Some(max_per_day) = limit.max_per_day {
        if stats.runs_in_last_day >= max_per_day {
            return Some(SkipReason::RateLimitedMaxPerDay {
                runs_today: stats.runs_in_last_day,
            });
        }
    }
    None
}

/// Normalize a project path for the scope comparison. We do not
/// canonicalize against the filesystem here (the evaluator is pure
/// and a project may be on an unmounted volume); instead we strip
/// the Windows verbatim prefix and a single trailing separator so
/// `/a/b` and `/a/b/` compare equal. Comparison is OS-appropriate:
/// case-insensitive on Windows/macOS where the filesystem is, exact
/// on Linux.
fn normalize_project(path: &str) -> String {
    let simplified = simplify_windows_path(path);
    let trimmed = simplified.trim_end_matches('/').trim_end_matches('\\');
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{
        Agent, AgentBinary, Lifecycle, OutputFormat, PermissionMode, PlatformOptions,
    };
    use crate::session::TokenUsage;
    use chrono::{Duration, TimeZone};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap()
    }

    /// An `Event`-triggered (session-settled) agent rooted at `cwd`.
    fn event_agent(cwd: &str, debounce_secs: u64, rl: Option<RateLimit>) -> Agent {
        let now = fixed_now();
        Agent {
            id: Uuid::new_v4(),
            name: "narrator".into(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AgentBinary::FirstParty,
            model: Some("claude-haiku-4-5".into()),
            cwd: cwd.into(),
            prompt: "narrate".into(),
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
                event: EventKind::SessionSettled { debounce_secs },
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
            rate_limit: rl,
            lifecycle: Lifecycle::Installed,
            drafted_by: None,
            created_via: crate::agent::types::CreatedVia::Gui,
        }
    }

    /// A session row with `last_ts` set, rooted at `project_path`.
    fn session(session_id: &str, project_path: &str, last_ts: Option<DateTime<Utc>>) -> SessionRow {
        SessionRow {
            session_id: session_id.into(),
            slug: "slug".into(),
            file_path: PathBuf::from(format!("/home/u/.claude/projects/slug/{session_id}.jsonl")),
            file_size_bytes: 1024,
            last_modified: None,
            project_path: project_path.into(),
            project_from_transcript: true,
            first_ts: last_ts,
            last_ts,
            event_count: 10,
            message_count: 4,
            user_message_count: 2,
            assistant_message_count: 2,
            first_user_prompt: Some("hello".into()),
            models: vec!["claude-opus-4-7".into()],
            tokens: TokenUsage::default(),
            git_branch: None,
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    /// A `run_stats` closure backed by a map; missing agents get the
    /// default (never run, zero today).
    fn stats_fn(map: HashMap<String, AgentRunStats>) -> impl Fn(&str) -> AgentRunStats {
        move |id: &str| map.get(id).cloned().unwrap_or_default()
    }

    #[test]
    fn settled_session_in_scope_fires() {
        let agent = event_agent(
            "/home/u/proj",
            600,
            Some(RateLimit {
                min_interval_secs: Some(3600),
                max_per_day: Some(10),
            }),
        );
        // Session last active 15 min ago — past the 10-min debounce.
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let fires = evaluate(
            std::slice::from_ref(&agent),
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].agent_id, agent.id.to_string());
        assert_eq!(fires[0].session_id, "sess-1");
    }

    #[test]
    fn session_still_active_does_not_fire() {
        let agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        // Session last active 2 min ago — within the 10-min debounce.
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(2)),
        );
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert!(fires.is_empty(), "a still-active session must not fire");
    }

    #[test]
    fn settled_detection_uses_mtime_when_no_last_ts() {
        let agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        let mut s = session("sess-1", "/home/u/proj", None);
        // No last_ts; fall back to file mtime 20 min ago.
        let mtime: std::time::SystemTime = (fixed_now() - Duration::minutes(20)).into();
        s.last_modified = Some(mtime);
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert_eq!(fires.len(), 1, "mtime fallback should mark it settled");
    }

    #[test]
    fn session_with_no_timestamp_at_all_never_settles() {
        let agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        let s = session("sess-1", "/home/u/proj", None); // no last_ts, no mtime
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert!(
            fires.is_empty(),
            "a session with no provable last activity must not fire"
        );
    }

    #[test]
    fn already_fired_pair_does_not_fire_again() {
        // FIRE-ONCE: a (agent, session) pair recorded in the ledger
        // is never re-fired, even though the session is still
        // settled and in scope.
        let agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let mut fired = HashSet::new();
        fired.insert((agent.id.to_string(), "sess-1".to_string()));
        let fires = evaluate(
            &[agent],
            &[s],
            &fired,
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert!(fires.is_empty(), "a ledger-recorded pair must not re-fire");
    }

    #[test]
    fn agent_produced_session_is_excluded() {
        // SELF-TRIGGER EXCLUSION (D7): a session produced by an
        // agent run is never evaluated. Without this the Session
        // Narrator narrates its own output forever.
        let agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        let s = session(
            "sess-agent-output",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let mut agent_sessions = HashSet::new();
        agent_sessions.insert("sess-agent-output".to_string());
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &agent_sessions,
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert!(
            fires.is_empty(),
            "an agent-produced session must never fire an event agent"
        );
    }

    #[test]
    fn session_out_of_project_scope_does_not_fire() {
        let agent = event_agent("/home/u/proj-a", 600, Some(RateLimit::default()));
        // Session belongs to a different project.
        let s = session(
            "sess-1",
            "/home/u/proj-b",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert!(
            fires.is_empty(),
            "a session outside the agent's project must not fire it"
        );
    }

    #[test]
    fn project_scope_tolerates_trailing_separator() {
        // `/home/u/proj` and `/home/u/proj/` are the same project.
        let agent = event_agent("/home/u/proj/", 600, Some(RateLimit::default()));
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert_eq!(fires.len(), 1);
    }

    #[test]
    fn min_interval_rate_limit_blocks_fire() {
        // RATE-LIMIT (D9): the agent ran 30 min ago and its
        // min_interval is 1h — it must not fire again yet.
        let agent = event_agent(
            "/home/u/proj",
            600,
            Some(RateLimit {
                min_interval_secs: Some(3600),
                max_per_day: None,
            }),
        );
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let mut map = HashMap::new();
        map.insert(
            agent.id.to_string(),
            AgentRunStats {
                last_run_started_at: Some(fixed_now() - Duration::minutes(30)),
                runs_in_last_day: 1,
            },
        );
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(map),
            fixed_now(),
        );
        assert!(
            fires.is_empty(),
            "min-interval rate limit must block the fire"
        );
    }

    #[test]
    fn max_per_day_rate_limit_blocks_fire() {
        let agent = event_agent(
            "/home/u/proj",
            600,
            Some(RateLimit {
                min_interval_secs: None,
                max_per_day: Some(5),
            }),
        );
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let mut map = HashMap::new();
        map.insert(
            agent.id.to_string(),
            AgentRunStats {
                last_run_started_at: Some(fixed_now() - Duration::hours(3)),
                runs_in_last_day: 5,
            },
        );
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(map),
            fixed_now(),
        );
        assert!(
            fires.is_empty(),
            "max-per-day rate limit must block the fire"
        );
    }

    #[test]
    fn rate_limit_permits_fire_when_interval_elapsed() {
        let agent = event_agent(
            "/home/u/proj",
            600,
            Some(RateLimit {
                min_interval_secs: Some(3600),
                max_per_day: Some(10),
            }),
        );
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let mut map = HashMap::new();
        // Last run 2h ago — past the 1h min interval.
        map.insert(
            agent.id.to_string(),
            AgentRunStats {
                last_run_started_at: Some(fixed_now() - Duration::hours(2)),
                runs_in_last_day: 2,
            },
        );
        let fires = evaluate(
            &[agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(map),
            fixed_now(),
        );
        assert_eq!(fires.len(), 1, "an elapsed interval permits the fire");
    }

    #[test]
    fn cron_and_manual_agents_are_ignored() {
        // A non-event agent passed in by mistake is silently
        // skipped — `evaluate` only acts on `Trigger::Event`.
        let mut cron_agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        cron_agent.trigger = Trigger::Cron {
            cron: "0 9 * * *".into(),
            timezone: None,
        };
        let s = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let fires = evaluate(
            &[cron_agent],
            &[s],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert!(fires.is_empty());
    }

    #[test]
    fn multiple_settled_sessions_fire_oldest_first_one_per_tick() {
        // One agent, two settled sessions in one tick → exactly ONE
        // fire, for the oldest-settled session (s2, idle 20 min, vs
        // s1's 15). The other fires on a later tick. This is the
        // guard against a rate-limited agent firing N runs at once.
        let agent = event_agent("/home/u/proj", 600, Some(RateLimit::default()));
        let s1 = session(
            "sess-1",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(15)),
        );
        let s2 = session(
            "sess-2",
            "/home/u/proj",
            Some(fixed_now() - Duration::minutes(20)),
        );
        let fires = evaluate(
            &[agent],
            &[s1, s2],
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert_eq!(fires.len(), 1);
        assert_eq!(
            fires[0].session_id, "sess-2",
            "the oldest-settled session fires first"
        );
    }

    #[test]
    fn rate_limited_agent_never_overshoots_in_one_tick() {
        // F2 regression guard: a never-run agent with `max_per_day`
        // and many settled sessions must fire AT MOST ONCE this tick.
        // The pure evaluator cannot see the runs it would itself
        // authorize, so it authorizes exactly one and lets later
        // ticks re-gate the rest. Before the fix this fired 50 runs.
        let agent = event_agent(
            "/home/u/proj",
            600,
            Some(RateLimit {
                min_interval_secs: None,
                max_per_day: Some(5),
            }),
        );
        let sessions: Vec<SessionRow> = (0..50)
            .map(|i| {
                session(
                    &format!("sess-{i}"),
                    "/home/u/proj",
                    Some(fixed_now() - Duration::minutes(15)),
                )
            })
            .collect();
        let fires = evaluate(
            &[agent],
            &sessions,
            &HashSet::new(),
            &HashSet::new(),
            &stats_fn(HashMap::new()),
            fixed_now(),
        );
        assert_eq!(
            fires.len(),
            1,
            "a never-run agent must not fire 50 runs in one tick"
        );
    }
}
