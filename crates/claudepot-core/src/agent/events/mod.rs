//! Reactive `session-settled` agent triggers — the pure half (PRD §7).
//!
//! This module is to event triggers what `crate::rotation` is to
//! auto-rotation: a **behavior over the existing `agent` noun**, not a
//! new domain noun (see `.claude/rules/architecture.md`). It is
//! pure-Rust, no Tauri dependency, and provides:
//!
//! - [`eval`] — the pure `(agents, sessions, ledger, exclusion-set,
//!   run-stats, now) -> Vec<EventFire>` evaluator. No I/O; the clock
//!   is injected.
//! - [`store`] — atomic load/save of the event-state ledger at
//!   `~/.claudepot/agent-events.json`, recording which
//!   `(agent_id, session_id)` pairs have already fired so each fires
//!   exactly once.
//! - [`policy`] — the pure orchestration policy: F14 stats derivation
//!   from the ledger, the F17 exclusion-set construction (I/O
//!   injected), the per-agent first-tick burst cap, and the dispatch
//!   env contract.
//!
//! The runtime bridge — collecting the inputs, dispatching the runs,
//! recording the ledger, applying the bounded catch-up cap — lives in
//! the in-app event orchestrator (the Tauri layer), hooked into
//! `usage_snapshot::run_tick` alongside the rotation + permission
//! orchestrators. Zero overhead when no `Event`-triggered agents
//! exist.
//!
//! # Constraints the orchestrator MUST honor (grill findings)
//!
//! The orchestrator that wires these pure pieces into `run_tick`
//! now lives at `src-tauri/src/agent_event_orchestrator.rs` and is
//! hooked into `usage_snapshot::run_tick`. The four constraints
//! below remain load-bearing — they are the contract every future
//! re-implementation (test seam, refactor, alternative orchestrator)
//! MUST preserve. The obvious implementation gets each of them
//! wrong, and none surfaces in a unit test of the pure pieces
//! alone (every test in this module injects clean in-memory
//! inputs); the orchestrator's integration tests in
//! `agent_event_orchestrator::tests` lock these invariants down at
//! the wired level.
//!
//! ## F1 — record the fire BEFORE dispatching the run
//!
//! [`evaluate`] deliberately emits *all* eligible `(agent, session)`
//! pairs each tick; fire-once is purely a property of the on-disk
//! ledger ([`store`]). For every [`EventFire`] the orchestrator
//! returns, it MUST `record_fire` + `save` the ledger **before**
//! spawning `claude -p` — never dispatch-then-record. A duplicate
//! ledger entry is free (`record_fire` is idempotent); a duplicate
//! billed `claude -p` is a real cost leak. A crashed dispatch that
//! leaves a "fired" pair which never actually ran is the acceptable
//! failure direction — a missed run is far cheaper than a retry
//! storm.
//!
//! ## F14 — derive rate-limit stats from the DURABLE ledger
//!
//! [`evaluate`] needs an [`AgentRunStats`] per agent
//! (`last_run_started_at` + `runs_in_last_day`) for the D9
//! rate-limiter. The orchestrator MUST derive those stats from a
//! **durable** source — the event-state ledger here, or a
//! sibling run-history file — and **NOT** from the per-agent
//! `result.json` directories under `agent_runs_dir`. Those run
//! directories are pruned at `log_retention_runs` (enforced by
//! `record_run` — grill F12): a high-frequency agent whose oldest
//! runs are pruned would under-count `runs_in_last_day` and exceed
//! `max_per_day`. The ledger already solved exactly this fragility
//! for fire-once with a dedicated durable file; rate-limiting must
//! not be left on prunable data. The ledger's [`FiredEntry`] carries
//! `fired_at` precisely so per-agent fire counts and the most-recent
//! fire instant can be derived from it without touching run dirs.
//!
//! ## F17 — build the self-trigger exclusion set from the
//! authoritative `RunResult.session_id`
//!
//! The D7 self-trigger exclusion (`agent_session_ids` passed to
//! [`evaluate`]) stops an agent narrating its own output. The
//! orchestrator MUST populate that set from the authoritative
//! `RunResult.session_id` parsed out of each run's `result.json`
//! (`crate::agent::run` parses it from `claude -p` stdout) — NOT
//! from `AgentRun::session_jsonl_path`, which is re-derived by a
//! depth-limited filename walk that fails open: when the walk
//! returns `None` (transcript nested too deep, or on an unmounted
//! volume) the agent's own session id silently leaves the exclusion
//! set and the Session Narrator can narrate its own output — the
//! exact D7 infinite loop. The session id from the parsed
//! `RunResult` is exact and never fails open.
//!
//! ## Ledger growth
//!
//! [`store`] already hard-caps the ledger (oldest-first eviction)
//! and offers `prune`; the orchestrator should still call `prune`
//! each tick to drop pairs whose agent or session is gone.

pub mod eval;
pub mod policy;
pub mod store;

pub use eval::{evaluate, AgentRunStats, EventFire, SkipReason};
pub use policy::{
    apply_per_agent_first_tick_cap, build_dispatch_env, build_run_stats_from_ledger,
    build_self_exclusion_set_with, FIRST_TICK_BURST_CAP,
};
pub use store::{
    events_path, load, load_from, load_or_default, save, save_to, AgentEventsError, EventsFile,
    FiredEntry, EVENTS_FILENAME, SCHEMA_VERSION,
};
