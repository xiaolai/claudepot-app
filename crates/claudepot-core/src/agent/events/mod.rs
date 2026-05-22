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
//!
//! The runtime bridge — collecting the inputs, dispatching the runs,
//! recording the ledger, the bounded catch-up cap — lives in
//! `src-tauri/src/agent_event_orchestrator.rs`, hooked into
//! `usage_snapshot::run_tick` alongside the rotation + permission
//! orchestrators. Zero overhead when no `Event`-triggered agents
//! exist.

pub mod eval;
pub mod store;

pub use eval::{evaluate, AgentRunStats, EventFire, SkipReason};
pub use store::{
    events_path, load, load_from, load_or_default, save, save_to,
    AgentEventsError, EventsFile, FiredEntry, EVENTS_FILENAME,
    SCHEMA_VERSION,
};
