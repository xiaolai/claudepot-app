//! Scheduled `claude -p` runs — the **Automation** noun.
//!
//! An automation is a `(binary, prompt, schedule, cwd, …)` record
//! that materializes into a per-OS scheduler artifact (launchd
//! plist on macOS, Task Scheduler XML on Windows, systemd-user
//! timer + service unit on Linux). Each run produces a structured
//! `result.json` plus stdout/stderr logs, browsable from the
//! Automations sidebar section.
//!
//! Cardinality and design notes live in
//! `dev-docs/automations-implementation-plan.md`. CLI surface
//! survey (the `claude -p` flag table this module is built on)
//! lives in `dev-docs/agents-cli-surface.md`.
//!
//! v1 scope: cron + manual triggers only. fs-watch / webhook
//! reactive triggers are explicitly v2.
//!
//! ## Module layout
//!
//! - [`error`] — `AutomationError` enum (one boundary error type).
//! - [`types`] — `Automation`, `AutomationRun`, `Trigger`,
//!   `PermissionMode`, `OutputFormat`, `PlatformOptions`, etc.
//! - [`slug`] — name validation (lowercase ASCII alnum + dash,
//!   1–64, unique per store).
//! - [`cron`] — five-field cron parser → `LaunchSlot` enumeration
//!   that scheduler adapters translate to native triggers.
//! - [`env`] — env-var whitelist for user-supplied `extra_env`
//!   plus the curated default `PATH` segments.
//! - [`store`] — `AutomationStore`: JSON read-modify-write over
//!   `~/.claudepot/automations.json`.
//! - [`shim`] — per-OS helper-shim emitter (`.sh` / `.cmd`) used
//!   by every scheduler artifact instead of calling `claude`
//!   directly.

pub mod cron;
pub mod env;
pub mod error;
pub mod install;
pub mod run;
pub mod scheduler;
pub mod shim;
pub mod slug;
pub mod store;
pub mod types;

pub use error::AutomationError;
pub use install::{current_claudepot_cli, install_shim, resolve_binary};
pub use run::{list_run_ids, parse_result_event, read_run, record_run, run_now, RecordInputs};
pub use scheduler::{
    active_scheduler, cron_next_runs, noop::NoopScheduler, RegisteredEntry, Scheduler,
    SchedulerCapabilities,
};
pub use shim::{render_unix, render_windows, ShimInputs};
pub use slug::validate_name;
pub use store::{
    automation_dir, automation_runs_dir, automations_file_path, AutomationPatch,
    AutomationStore,
};
pub use types::{
    Automation, AutomationBinary, AutomationId, AutomationRun, HostPlatform,
    OutputFormat, PermissionMode, PlatformOptions, RunResult, Trigger, TriggerKind,
};
