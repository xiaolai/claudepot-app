//! Scheduled `claude -p` runs — the **Agent** noun.
//!
//! An agent is a `(binary, prompt, schedule, cwd, …)` record
//! that materializes into a per-OS scheduler artifact (launchd
//! plist on macOS, Task Scheduler XML on Windows, systemd-user
//! timer + service unit on Linux). Each run produces a structured
//! `result.json` plus stdout/stderr logs, browsable from the
//! Agents sidebar section.
//!
//! Cardinality and design notes live in
//! `dev-docs/agents-implementation-plan.md`. CLI surface
//! survey (the `claude -p` flag table this module is built on)
//! lives in `dev-docs/agents-cli-surface.md`.
//!
//! Trigger scope: cron, manual, and the `session-settled` reactive
//! event (PRD §7). fs-watch / webhook / usage-threshold reactive
//! triggers remain PRD-deferred siblings (§13).
//!
//! ## Module layout
//!
//! - [`error`] — `AgentError` enum (one boundary error type).
//! - [`types`] — `Agent`, `AgentRun`, `Trigger`,
//!   `PermissionMode`, `OutputFormat`, `PlatformOptions`, etc.
//! - [`slug`] — name validation (lowercase ASCII alnum + dash,
//!   1–64, unique per store).
//! - [`cron`] — five-field cron parser → `LaunchSlot` enumeration
//!   that scheduler adapters translate to native triggers.
//! - [`env`] — env-var whitelist for user-supplied `extra_env`
//!   plus the curated default `PATH` segments.
//! - [`store`] — `AgentStore`: JSON read-modify-write over
//!   `~/.claudepot/agents.json`. Also the boot-time reconcilers
//!   (both directions).
//! - [`shim`] — per-OS helper-shim emitter (`.sh` / `.cmd`) used
//!   by every scheduler artifact instead of calling `claude`
//!   directly.
//! - [`draft`] — the Phase-2 AI-drafting path: normalize a JSON
//!   spec (Claudepot-native or SDK `AgentDefinition`-shaped) into
//!   an inert `lifecycle = Draft` agent. Also hosts the per-field
//!   byte-cap + control-char validators (grill F18 / X13).
//! - [`events`] — the pure half of the PRD §7 reactive
//!   `session-settled` trigger: the evaluator and the on-disk
//!   ledger. The runtime bridge lives in
//!   `src-tauri/src/agent_event_orchestrator.rs`.
//! - [`templates`] — built-in agent templates (Session Narrator,
//!   etc.) used by the GUI's `agent_add_from_template` path.
//! - [`install_gate`] — the draft → install gate
//!   ([`install_draft`](install_gate::install_draft)) and the
//!   shared install-ordering helper
//!   ([`apply_lifecycle_change`](install_gate::apply_lifecycle_change))
//!   used by every GUI verb that materializes a scheduler artifact.
//! - [`prerun`] — pre-run env + permission preparation shared
//!   between manual Run-Now and scheduled dispatch.
//! - [`run`] — `record_run` / `run_now` / `record_run_for_agent`:
//!   the on-disk run-history surface (`result.json` + logs +
//!   retention pruning).
//! - [`install`] — `install_shim` + `resolve_binary`: per-OS
//!   shim file install on disk plus the Claudepot CLI lookup
//!   used by the shim and the orchestrator.
//! - [`scheduler`] — `Scheduler` trait + per-OS adapters
//!   (`launchd`, `systemd`, `schtasks`) + the no-op test seam.

pub mod cron;
pub mod draft;
pub mod env;
pub mod error;
pub mod events;
pub mod install;
pub mod install_gate;
pub mod prerun;
pub mod run;
pub mod scheduler;
pub mod shim;
pub mod slug;
pub mod store;
pub mod templates;
pub mod types;

pub use draft::{
    build_draft, validate_agent_inputs, validate_cwd, validate_trigger_timezone, CliOverrides,
    DraftInput, DraftSpec,
};
pub use error::AgentError;
pub use events::{
    evaluate as evaluate_events, AgentEventsError, AgentRunStats, EventFire, EventsFile, FiredEntry,
};
pub use install::{current_claudepot_cli, install_shim, resolve_binary};
pub use install_gate::{apply_lifecycle_change, install_draft, InstallOutcome};
pub use run::{
    list_run_ids, parse_result_event, prune_run_dirs, read_run, record_run, record_run_for_agent,
    run_now, RecordInputs,
};
pub use scheduler::{
    active_scheduler, cron_next_runs, noop::NoopScheduler, RegisteredEntry, Scheduler,
    SchedulerCapabilities,
};
pub use shim::{render_unix, render_windows, ShimInputs};
pub use slug::validate_name;
pub use store::{
    agent_dir, agent_runs_dir, agents_file_path, reconcile_installed_agents,
    reconcile_orphan_artifacts, reconcile_orphan_artifacts_now, reconcile_orphan_artifacts_using,
    reconcile_with_scheduler, reconcile_with_scheduler_using, AgentPatch, AgentStore,
    OrphanArtifact, OrphanInstalled,
};
pub use templates::session_narrator;
pub use types::{
    Agent, AgentBinary, AgentId, AgentRun, CreatedVia, EventKind, HostPlatform, Lifecycle,
    McpServerRef, OutputFormat, PermissionMode, PlatformOptions, RateLimit, RunResult, Trigger,
    TriggerKind, DEFAULT_DEBOUNCE_SECS,
};
