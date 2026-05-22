//! CLI surface for the Agents feature.
//!
//! The `agent` command carries two kinds of verb:
//!
//! * **Plumbing** — `_record-run` (leading underscore, `hide`d):
//!   invoked by the per-agent helper shim after `claude -p` exits.
//!   See [`record_run`].
//! * **The AI-drafting path (Phase 2)** — `draft` / `list` /
//!   `show`: a user-/AI-facing surface that lets an AI client
//!   *propose* an agent and lets anyone *read* the agent store.
//!   See [`draft`] and [`inspect`].
//!
//! ## The draft / install / edit gate — the security spine (D8)
//!
//! This CLI is deliberately **incomplete by design**. It exposes:
//!
//! * `draft` — *create* a `lifecycle = Draft` agent. A draft is
//!   inert: it sits in `agents.json`, no scheduler artifact is
//!   materialized, nothing fires.
//! * `list` / `show` — *read* the agent store.
//!
//! It does **not** expose — and must never grow — an `install`
//! verb or an `edit` verb. An AI client driving this CLI via Bash
//! can therefore only *create drafts* and *read*. It can never:
//!
//! * arm an agent (draft -> installed + materialize the scheduler
//!   artifact) — that is a human action in the Claudepot GUI;
//! * mutate an already-armed (`Installed`) agent — to change an
//!   installed agent, the AI drafts a *new* replacement and a
//!   human installs it.
//!
//! That asymmetry IS the gate. A scheduled `claude -p` (possibly
//! `bypassPermissions`) baked into a launchd plist is structurally
//! a persistence mechanism; keeping arming and edit-of-armed
//! human-only means an AI can author but never silently arm or
//! re-arm one. Adding an `install`/`edit` verb here would
//! dismantle the gate — do not.
//!
//! Submodules reach this entry file's private helpers via
//! `use super::*;`.

use anyhow::Result;
use claudepot_core::agent::Agent;

mod draft;
mod inspect;
mod record_run;

pub use draft::{draft_cmd, DraftArgs};
pub use inspect::{list_cmd, show_cmd};
pub use record_run::record_run_cmd;

// ---------- shared formatters ----------

/// One-line trigger summary for the `list` table and `show`.
pub(super) fn trigger_summary(agent: &Agent) -> String {
    use claudepot_core::agent::{EventKind, Trigger};
    match &agent.trigger {
        Trigger::Cron { cron, timezone } => match timezone {
            Some(tz) => format!("cron {cron} ({tz})"),
            None => format!("cron {cron}"),
        },
        Trigger::Manual => "manual".to_string(),
        Trigger::Event { event } => match event {
            EventKind::SessionSettled { debounce_secs } => {
                format!("event session-settled ({debounce_secs}s)")
            }
        },
    }
}

/// Render one agent as a pretty JSON value. Shared by `draft`
/// (`--json` confirmation), `list`, and `show` so the on-the-wire
/// shape never drifts between verbs.
pub(super) fn agent_to_json(agent: &Agent) -> serde_json::Value {
    serde_json::json!({
        "id": agent.id.to_string(),
        "name": agent.name,
        "display_name": agent.display_name,
        "description": agent.description,
        "enabled": agent.enabled,
        "lifecycle": match agent.lifecycle {
            claudepot_core::agent::Lifecycle::Draft => "draft",
            claudepot_core::agent::Lifecycle::Installed => "installed",
        },
        "drafted_by": agent.drafted_by,
        "model": agent.model,
        "cwd": agent.cwd,
        "prompt": agent.prompt,
        "system_prompt": agent.system_prompt,
        "append_system_prompt": agent.append_system_prompt,
        "permission_mode": agent.permission_mode.as_cli_flag(),
        "allowed_tools": agent.allowed_tools,
        "disallowed_tools": agent.disallowed_tools,
        "output_format": agent.output_format.as_cli_flag(),
        "mcp_servers": agent.mcp_servers,
        "run_as": agent.run_as,
        "task_budget": agent.task_budget,
        "rate_limit": agent.rate_limit,
        "trigger": agent.trigger,
        "trigger_summary": trigger_summary(agent),
        "created_at": agent.created_at.to_rfc3339(),
        "updated_at": agent.updated_at.to_rfc3339(),
    })
}

/// Print the result of a verb either as human text (via `human`)
/// or as a single JSON value (`value`). Centralized so every verb
/// honors `--json` identically per `rules/commands.md`.
pub(super) fn emit(json: bool, value: serde_json::Value, human: &str) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{human}");
    }
    Ok(())
}
