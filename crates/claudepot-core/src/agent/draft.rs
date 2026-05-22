//! Building a **draft** agent from a JSON spec — the core of the
//! Phase-2 AI-drafting path.
//!
//! An AI client (via Bash, calling `claudepot agent draft`) hands
//! Claudepot a spec. This module turns that spec into a validated
//! [`Agent`] with `lifecycle = Draft`. A draft is **inert**: it
//! sits in `agents.json` and nothing fires. No scheduler artifact
//! is materialized here — that is the human-only arming step
//! (`AgentStore` + the GUI's `agent_install` command).
//!
//! ## Two accepted input shapes (PRD D2)
//!
//! `agent draft` accepts JSON in either of two shapes and
//! normalizes on ingest:
//!
//! 1. **Claudepot-native** — the same field names the persisted
//!    `Agent` uses (`name`, `prompt`, `permission_mode`,
//!    `allowed_tools`, `trigger`, …).
//! 2. **`AgentDefinition`-shaped** — the SDK subagent shape:
//!    `description` / `prompt` / `tools` / `model` / `mcpServers`.
//!
//! The *persisted* form is always Claudepot-native, anchored to
//! the CLI flag contract — never to the SDK's versioned type. The
//! SDK shape is an input convenience, not an on-disk dependency.
//!
//! This module is pure: no I/O, no env reads. `build_draft` takes
//! the parsed spec plus a clock and returns an `Agent` or an
//! `AgentError`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use super::error::AgentError;
use super::slug::validate_name;
use super::types::{
    Agent, AgentBinary, Lifecycle, McpServerRef, OutputFormat, PermissionMode,
    PlatformOptions, RateLimit, Trigger,
};

/// The normalized, shape-agnostic draft spec. Whichever JSON shape
/// arrives ([`DraftInput`]), it collapses to this before
/// [`build_draft`] turns it into an [`Agent`].
///
/// Only fields meaningful for a *draft* are carried — a draft is
/// constructed, not run, so run-time fields stay at their `Agent`
/// defaults until a human reviews and installs it.
#[derive(Debug, Clone)]
pub struct DraftSpec {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub model: Option<String>,
    pub cwd: String,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub mcp_servers: Vec<McpServerRef>,
    pub output_format: OutputFormat,
    pub run_as: Option<String>,
    pub task_budget: Option<u64>,
    pub rate_limit: Option<RateLimit>,
    pub trigger: Trigger,
    pub extra_env: BTreeMap<String, String>,
}

/// A raw JSON spec accepted by `agent draft`, in *either* of the
/// two PRD-D2 shapes. Serde's `untagged` enum tries each variant
/// in order; the SDK shape is tried first because its
/// `description`-keyed object is the more constrained match (it
/// requires `description` + `prompt`), so a Claudepot-native spec
/// that omits `description` falls through cleanly to the native
/// variant.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DraftInput {
    /// SDK `AgentDefinition`-shaped JSON.
    AgentDefinition(AgentDefinitionInput),
    /// Claudepot-native JSON.
    Native(NativeInput),
}

/// The SDK subagent shape — `description` / `prompt` / `tools` /
/// `model` / `mcpServers`. Claudepot normalizes this to a
/// [`DraftSpec`]; the SDK type is *never* persisted.
///
/// `AgentDefinition` does not carry a Claudepot agent `name`, a
/// `cwd`, or a trigger — those are Claudepot concepts. They are
/// supplied via CLI flags alongside `--from-json`; see
/// [`merge_cli_overrides`].
///
/// `deny_unknown_fields` is load-bearing for the `untagged`
/// dispatch: a Claudepot-native spec carries `name` / `cwd` (which
/// the SDK shape has no field for), so it fails this variant and
/// serde falls through to [`NativeInput`]. Without the deny, a
/// native spec that happens to set `description` would be misread
/// as an `AgentDefinition` and lose its name/cwd.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentDefinitionInput {
    /// SDK `description`. Becomes the agent's `description`.
    pub description: String,
    /// SDK `prompt` — the agent's system/instruction text. Mapped
    /// to Claudepot's `prompt` (the `claude -p` argument).
    pub prompt: String,
    /// SDK `tools` — an allow-list of tool names. Mapped to
    /// `allowed_tools`.
    #[serde(default)]
    pub tools: Vec<String>,
    /// SDK `model`. A bare alias (`haiku`) or a versioned id.
    #[serde(default)]
    pub model: Option<String>,
    /// SDK `mcpServers` — a map of server-name -> opaque config.
    /// Each entry becomes an `McpServerRef::Custom`.
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, serde_json::Value>,
}

/// Claudepot-native JSON spec — the field names match the persisted
/// [`Agent`]. Every field except `name`, `cwd`, and `prompt` has a
/// safe default so a minimal spec is accepted.
#[derive(Debug, Clone, Deserialize)]
pub struct NativeInput {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub cwd: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerRef>,
    #[serde(default = "default_output_format")]
    pub output_format: OutputFormat,
    #[serde(default)]
    pub run_as: Option<String>,
    #[serde(default)]
    pub task_budget: Option<u64>,
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
    /// Optional trigger. Absent => `Trigger::Manual` (the safest
    /// default for a draft — Run-Now only, never an OS artifact).
    #[serde(default)]
    pub trigger: Option<Trigger>,
    #[serde(default)]
    pub extra_env: BTreeMap<String, String>,
}

fn default_permission_mode() -> PermissionMode {
    PermissionMode::Default
}

fn default_output_format() -> OutputFormat {
    OutputFormat::Json
}

/// CLI-supplied fields that don't exist in (or should override) the
/// JSON spec. The SDK `AgentDefinition` shape carries no agent
/// `name` / `cwd` / `trigger`; those must come from flags. For a
/// native spec the flags override whatever the JSON carried.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub trigger: Option<Trigger>,
    /// Replace `allowed_tools` entirely when `Some`.
    pub allowed_tools: Option<Vec<String>>,
    /// Replace `disallowed_tools` entirely when `Some`.
    pub disallowed_tools: Option<Vec<String>>,
    pub run_as: Option<String>,
    pub task_budget: Option<u64>,
    /// Attach Claudepot's own memory MCP server.
    pub attach_memory: bool,
}

impl DraftInput {
    /// Parse a [`DraftInput`] from a JSON document.
    pub fn from_json(raw: &str) -> Result<Self, AgentError> {
        serde_json::from_str(raw).map_err(AgentError::Json)
    }

    /// Collapse either input shape into a normalized [`DraftSpec`],
    /// applying CLI overrides. The SDK shape *requires* `--name`
    /// and `--cwd` flags (it carries neither); a native shape may
    /// have them in JSON and the flags then override.
    pub fn normalize(self, ov: &CliOverrides) -> Result<DraftSpec, AgentError> {
        let mut spec = match self {
            DraftInput::AgentDefinition(d) => normalize_sdk(d),
            DraftInput::Native(n) => normalize_native(n)?,
        };
        merge_cli_overrides(&mut spec, ov);
        // `name` and `cwd` are the two fields the SDK shape can't
        // supply; after overrides they must be non-empty.
        if spec.name.trim().is_empty() {
            return Err(AgentError::MissingField("name"));
        }
        if spec.cwd.trim().is_empty() {
            return Err(AgentError::MissingField("cwd"));
        }
        Ok(spec)
    }
}

/// Normalize an SDK `AgentDefinition` into a partial [`DraftSpec`].
/// `name`/`cwd`/`trigger` are left empty/`Manual`; the caller's
/// [`CliOverrides`] must fill `name` and `cwd`.
fn normalize_sdk(d: AgentDefinitionInput) -> DraftSpec {
    let mcp_servers = d
        .mcp_servers
        .into_iter()
        .map(|(name, config)| McpServerRef::Custom { name, config })
        .collect();
    DraftSpec {
        // `name` must come from a CLI flag — the SDK shape has none.
        name: String::new(),
        display_name: None,
        description: Some(d.description),
        model: d.model,
        // Likewise `cwd` — supplied via `--cwd`.
        cwd: String::new(),
        prompt: d.prompt,
        system_prompt: None,
        append_system_prompt: None,
        // SDK subagents don't carry a permission mode; default to
        // the safe `Default` mode. A human can elevate at install.
        permission_mode: PermissionMode::Default,
        allowed_tools: d.tools,
        disallowed_tools: Vec::new(),
        mcp_servers,
        output_format: OutputFormat::Json,
        run_as: None,
        task_budget: None,
        rate_limit: None,
        // Draft default: Manual — no OS artifact, Run-Now only.
        trigger: Trigger::Manual,
        extra_env: BTreeMap::new(),
    }
}

/// Normalize a Claudepot-native spec into a [`DraftSpec`].
fn normalize_native(n: NativeInput) -> Result<DraftSpec, AgentError> {
    Ok(DraftSpec {
        name: n.name,
        display_name: n.display_name,
        description: n.description,
        model: n.model,
        cwd: n.cwd,
        prompt: n.prompt,
        system_prompt: n.system_prompt,
        append_system_prompt: n.append_system_prompt,
        permission_mode: n.permission_mode,
        allowed_tools: n.allowed_tools,
        disallowed_tools: n.disallowed_tools,
        mcp_servers: n.mcp_servers,
        output_format: n.output_format,
        run_as: n.run_as,
        task_budget: n.task_budget,
        rate_limit: n.rate_limit,
        // Absent trigger => Manual: a draft must never carry a
        // surprise OS-scheduler artifact past arming.
        trigger: n.trigger.unwrap_or(Trigger::Manual),
        extra_env: n.extra_env,
    })
}

/// Apply CLI overrides onto a normalized spec. A `Some` flag value
/// replaces whatever the JSON carried; `None` leaves it.
fn merge_cli_overrides(spec: &mut DraftSpec, ov: &CliOverrides) {
    if let Some(v) = &ov.name {
        spec.name = v.clone();
    }
    if let Some(v) = &ov.cwd {
        spec.cwd = v.clone();
    }
    if let Some(v) = &ov.display_name {
        spec.display_name = Some(v.clone());
    }
    if let Some(v) = &ov.description {
        spec.description = Some(v.clone());
    }
    if let Some(v) = &ov.model {
        spec.model = Some(v.clone());
    }
    if let Some(v) = ov.permission_mode {
        spec.permission_mode = v;
    }
    if let Some(v) = &ov.trigger {
        spec.trigger = v.clone();
    }
    if let Some(v) = &ov.allowed_tools {
        spec.allowed_tools = v.clone();
    }
    if let Some(v) = &ov.disallowed_tools {
        spec.disallowed_tools = v.clone();
    }
    if let Some(v) = &ov.run_as {
        spec.run_as = Some(v.clone());
    }
    if let Some(v) = ov.task_budget {
        spec.task_budget = Some(v);
    }
    if ov.attach_memory
        && !spec
            .mcp_servers
            .iter()
            .any(|m| matches!(m, McpServerRef::ClaudepotMemory))
    {
        spec.mcp_servers.push(McpServerRef::ClaudepotMemory);
    }
}

/// Reject a cron trigger that carries an IANA `timezone` (grill
/// finding F11).
///
/// The `Trigger::Cron.timezone` field is accepted by the wire/JSON
/// shape and persisted, but **no scheduler adapter honors it**:
/// launchd's `StartCalendarInterval`, systemd's `OnCalendar=`, and
/// Task Scheduler all interpret the cron slots in the host's local
/// time, never an arbitrary IANA zone. Honoring it correctly across
/// all three back-ends — which means projecting each cron slot
/// through the named zone *and* re-deriving the slot set on every
/// DST transition (a single "9 AM LA" cron has no fixed UTC offset)
/// — is a substantial cross-platform feature. Until that lands, a
/// silently-ignored timezone is a "load-bearing lie": an agent set
/// to "9 AM LA time" by a New York user fires at 9 AM Eastern with
/// no error.
///
/// The honest interim contract is to **reject** a non-`None`
/// timezone at draft/install validation with a clear message,
/// rather than accept-and-ignore it. A `None` timezone (cron
/// interpreted in host-local time, the only behavior the adapters
/// actually implement) is always accepted.
pub fn validate_trigger_timezone(trigger: &Trigger) -> Result<(), AgentError> {
    if let Trigger::Cron {
        timezone: Some(tz),
        ..
    } = trigger
    {
        return Err(AgentError::InvalidEnv(format!(
            "cron timezone {tz:?} is not supported yet — Claudepot's \
             schedulers interpret cron in the host's local time. Remove \
             the timezone (leave the schedule in local time), or track \
             the timezone-aware-scheduling follow-up."
        )));
    }
    Ok(())
}

/// Validate an agent's working directory. It must be an **absolute**
/// path (`Path::is_absolute` per `.claude/rules/paths.md` — never a
/// `starts_with("/")` check, so Windows drive paths resolve) and free
/// of `..` components. `claude -p` runs in this directory and honors
/// its project-local config (`.mcp.json`, hooks, `CLAUDE.md`); a
/// relative or traversal-laden `cwd` is a code-execution vector
/// (grill finding F4). Existence is intentionally not required — a
/// draft may be authored before its project exists.
pub fn validate_cwd(cwd: &str) -> Result<(), AgentError> {
    let path = std::path::Path::new(cwd);
    if !path.is_absolute() {
        return Err(AgentError::InvalidEnv(format!(
            "cwd must be an absolute path, got {cwd:?}"
        )));
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AgentError::InvalidEnv(format!(
            "cwd must not contain '..' components, got {cwd:?}"
        )));
    }
    Ok(())
}

/// Build a **draft** [`Agent`] from a normalized [`DraftSpec`].
///
/// The resulting record has `lifecycle = Draft` and `drafted_by`
/// set to the supplied actor id. It is validated to the same
/// standard the GUI Add-Agent path enforces (name shape, the
/// `bypassPermissions` cross-field invariant), so a draft can be
/// armed later without surprises.
///
/// This function **never** touches the filesystem and **never**
/// materializes a scheduler artifact. It returns an in-memory
/// record; persisting it (via `AgentStore::add` + `save`) is the
/// caller's job, and even a persisted draft stays inert until a
/// human arms it.
pub fn build_draft(
    spec: DraftSpec,
    drafted_by: &str,
    now: DateTime<Utc>,
) -> Result<Agent, AgentError> {
    // Name shape — same rule the store re-checks on `add`.
    let name = validate_name(&spec.name)?;

    // Cross-field invariant: bypassPermissions demands a non-empty
    // allow-list. Reject at draft time so a human is never asked to
    // arm a structurally-invalid record.
    if matches!(spec.permission_mode, PermissionMode::BypassPermissions)
        && spec.allowed_tools.is_empty()
    {
        return Err(AgentError::InvalidEnv(
            "bypassPermissions requires a non-empty allowed_tools whitelist".into(),
        ));
    }

    // Validate any user-supplied env vars against the whitelist.
    super::env::validate_map(&spec.extra_env)?;

    // The working directory must be absolute and traversal-free —
    // `claude -p` runs there and honors that directory's project-
    // local config, so an unvalidated `cwd` is a code-execution
    // vector (grill finding F4).
    validate_cwd(&spec.cwd)?;

    // A drafted agent may attach only Claudepot's own memory MCP
    // server. A `Custom` MCP server carries an arbitrary `command`
    // that `claude -p --mcp-config` would spawn as a child process;
    // an AI client authoring a draft must not be able to inject one
    // (grill finding F3). `--attach-memory` covers the legitimate
    // case.
    if let Some(McpServerRef::Custom { name, .. }) = spec
        .mcp_servers
        .iter()
        .find(|m| matches!(m, McpServerRef::Custom { .. }))
    {
        return Err(AgentError::InvalidEnv(format!(
            "custom MCP server {name:?} is not allowed in a drafted agent — \
             only the Claudepot memory server may be attached"
        )));
    }

    // A cron trigger's expression must parse — fail the draft now,
    // not at install time.
    if let Trigger::Cron { cron, .. } = &spec.trigger {
        super::cron::expand(cron)?;
    }

    // A cron trigger must not carry an IANA timezone: no scheduler
    // adapter honors it, so accepting it would be a silent lie
    // (grill finding F11). Reject at draft time.
    validate_trigger_timezone(&spec.trigger)?;

    // An event-triggered agent MUST carry a rate limit (PRD D9).
    // Reject at draft time so a human is never asked to arm an
    // unthrottled reactive agent — events × agents × Claude is the
    // dominant cost-runaway risk.
    if spec.trigger.is_event() {
        let has_usable_limit = spec.rate_limit.as_ref().is_some_and(|r| {
            r.min_interval_secs.is_some() || r.max_per_day.is_some()
        });
        if !has_usable_limit {
            return Err(AgentError::InvalidEnv(
                "an event-triggered agent must carry a rate_limit \
                 (a min interval and/or a max per day)"
                    .into(),
            ));
        }
    }

    Ok(Agent {
        id: Uuid::new_v4(),
        name,
        display_name: spec.display_name,
        description: spec.description,
        // A draft is created enabled so that, once a human arms it,
        // the GUI install path registers it immediately. The
        // `Draft` lifecycle — not `enabled` — is what keeps it
        // inert until then.
        enabled: true,
        binary: AgentBinary::FirstParty,
        model: spec.model,
        cwd: spec.cwd,
        prompt: spec.prompt,
        system_prompt: spec.system_prompt,
        append_system_prompt: spec.append_system_prompt,
        permission_mode: spec.permission_mode,
        allowed_tools: spec.allowed_tools,
        add_dir: Vec::new(),
        max_budget_usd: None,
        fallback_model: None,
        output_format: spec.output_format,
        json_schema: None,
        bare: false,
        extra_env: spec.extra_env,
        trigger: spec.trigger,
        platform_options: PlatformOptions::default(),
        log_retention_runs: 50,
        created_at: now,
        updated_at: now,
        claudepot_managed: true,
        template_id: None,
        disallowed_tools: spec.disallowed_tools,
        mcp_servers: spec.mcp_servers,
        run_as: spec.run_as,
        task_budget: spec.task_budget,
        rate_limit: spec.rate_limit,
        // The load-bearing field: a `draft` agent is inert.
        lifecycle: Lifecycle::Draft,
        drafted_by: Some(drafted_by.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn native_minimal_spec_builds_draft() {
        let raw = r#"{
            "name": "nightly-digest",
            "cwd": "/tmp/proj",
            "prompt": "summarize today"
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let agent = build_draft(spec, "claude-code@2026-05-22", now()).unwrap();
        assert_eq!(agent.name, "nightly-digest");
        assert_eq!(agent.cwd, "/tmp/proj");
        assert_eq!(agent.lifecycle, Lifecycle::Draft);
        assert_eq!(
            agent.drafted_by.as_deref(),
            Some("claude-code@2026-05-22")
        );
        // Absent trigger normalizes to Manual — no OS artifact.
        assert!(agent.trigger.is_manual());
    }

    #[test]
    fn sdk_agent_definition_shape_normalizes() {
        // The SDK subagent shape — description/prompt/tools/model.
        // name + cwd come from CLI overrides since the SDK shape
        // carries neither.
        let raw = r#"{
            "description": "Reviews diffs for security issues",
            "prompt": "You are a security reviewer.",
            "tools": ["Read", "Grep"],
            "model": "claude-haiku-4-5"
        }"#;
        let ov = CliOverrides {
            name: Some("sec-review".into()),
            cwd: Some("/tmp/repo".into()),
            ..CliOverrides::default()
        };
        let spec = DraftInput::from_json(raw).unwrap().normalize(&ov).unwrap();
        let agent = build_draft(spec, "claude-code@x", now()).unwrap();
        assert_eq!(agent.name, "sec-review");
        assert_eq!(agent.cwd, "/tmp/repo");
        assert_eq!(agent.prompt, "You are a security reviewer.");
        assert_eq!(agent.model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(agent.allowed_tools, vec!["Read", "Grep"]);
        assert_eq!(
            agent.description.as_deref(),
            Some("Reviews diffs for security issues")
        );
        assert!(agent.mcp_servers.is_empty());
        assert_eq!(agent.lifecycle, Lifecycle::Draft);
    }

    #[test]
    fn sdk_shape_without_name_or_cwd_is_rejected() {
        // The SDK shape carries neither name nor cwd; without the
        // CLI flags to supply them, normalization must fail.
        let raw = r#"{
            "description": "x",
            "prompt": "y"
        }"#;
        let err = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap_err();
        assert!(matches!(err, AgentError::MissingField(_)));
    }

    #[test]
    fn native_shape_takes_precedence_when_description_absent() {
        // A native spec without `description` must NOT be misread
        // as an SDK AgentDefinition (which requires description).
        let raw = r#"{
            "name": "x",
            "cwd": "/tmp",
            "prompt": "p",
            "allowed_tools": ["Read"],
            "permission_mode": "bypassPermissions"
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        assert_eq!(spec.permission_mode, PermissionMode::BypassPermissions);
        let agent = build_draft(spec, "t", now()).unwrap();
        assert_eq!(agent.permission_mode, PermissionMode::BypassPermissions);
    }

    #[test]
    fn bypass_without_tools_rejected_at_draft_time() {
        let raw = r#"{
            "name": "danger",
            "cwd": "/tmp",
            "prompt": "p",
            "permission_mode": "bypassPermissions"
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let err = build_draft(spec, "t", now()).unwrap_err();
        assert!(matches!(err, AgentError::InvalidEnv(_)));
    }

    #[test]
    fn invalid_name_rejected_at_draft_time() {
        let raw = r#"{ "name": "INVALID", "cwd": "/tmp", "prompt": "p" }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let err = build_draft(spec, "t", now()).unwrap_err();
        assert!(matches!(err, AgentError::InvalidName(..)));
    }

    #[test]
    fn cron_trigger_with_bad_expression_rejected() {
        let raw = r#"{
            "name": "x",
            "cwd": "/tmp",
            "prompt": "p",
            "trigger": { "kind": "cron", "cron": "not a cron" }
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        assert!(build_draft(spec, "t", now()).is_err());
    }

    #[test]
    fn cron_trigger_with_timezone_is_rejected() {
        // F11: a cron trigger carrying an IANA timezone must be
        // rejected at draft time — no scheduler adapter honors it,
        // so accepting it would be a silent lie.
        let raw = r#"{
            "name": "tz-agent",
            "cwd": "/tmp",
            "prompt": "p",
            "trigger": {
                "kind": "cron",
                "cron": "0 9 * * *",
                "timezone": "America/Los_Angeles"
            }
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let err = build_draft(spec, "t", now()).unwrap_err();
        match err {
            AgentError::InvalidEnv(m) => {
                assert!(
                    m.contains("timezone"),
                    "error should name the timezone problem, got: {m}"
                );
            }
            other => panic!("expected InvalidEnv, got {other:?}"),
        }
    }

    #[test]
    fn cron_trigger_without_timezone_is_accepted() {
        // The complement: a cron trigger with no timezone (local
        // time, the only behavior the adapters implement) builds.
        let raw = r#"{
            "name": "local-cron",
            "cwd": "/tmp",
            "prompt": "p",
            "trigger": { "kind": "cron", "cron": "0 9 * * *" }
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let agent = build_draft(spec, "t", now()).unwrap();
        assert!(agent.trigger.is_cron());
    }

    #[test]
    fn cli_overrides_replace_json_values() {
        let raw = r#"{
            "name": "json-name",
            "cwd": "/tmp/json",
            "prompt": "p",
            "model": "sonnet"
        }"#;
        let ov = CliOverrides {
            name: Some("flag-name".into()),
            cwd: Some("/tmp/flag".into()),
            model: Some("opus".into()),
            attach_memory: true,
            ..CliOverrides::default()
        };
        let spec = DraftInput::from_json(raw).unwrap().normalize(&ov).unwrap();
        assert_eq!(spec.name, "flag-name");
        assert_eq!(spec.cwd, "/tmp/flag");
        assert_eq!(spec.model.as_deref(), Some("opus"));
        // attach_memory adds the Claudepot memory server.
        assert!(spec
            .mcp_servers
            .iter()
            .any(|m| matches!(m, McpServerRef::ClaudepotMemory)));
    }

    #[test]
    fn attach_memory_is_idempotent() {
        // A native spec that already carries claudepot_memory plus
        // --attach-memory must not double it.
        let raw = r#"{
            "name": "x",
            "cwd": "/tmp",
            "prompt": "p",
            "mcp_servers": [ { "kind": "claudepot_memory" } ]
        }"#;
        let ov = CliOverrides {
            attach_memory: true,
            ..CliOverrides::default()
        };
        let spec = DraftInput::from_json(raw).unwrap().normalize(&ov).unwrap();
        let count = spec
            .mcp_servers
            .iter()
            .filter(|m| matches!(m, McpServerRef::ClaudepotMemory))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn build_draft_never_sets_installed() {
        // The whole security model rests on this: build_draft
        // cannot, by any input, produce an Installed agent.
        let raw = r#"{ "name": "x", "cwd": "/tmp", "prompt": "p" }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let agent = build_draft(spec, "t", now()).unwrap();
        assert_eq!(agent.lifecycle, Lifecycle::Draft);
    }

    #[test]
    fn relative_cwd_is_rejected() {
        let raw = r#"{ "name": "x", "cwd": "relative/dir", "prompt": "p" }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let err = build_draft(spec, "t", now()).unwrap_err();
        assert!(matches!(err, AgentError::InvalidEnv(_)));
    }

    #[test]
    fn cwd_with_parent_dir_component_is_rejected() {
        let raw = r#"{ "name": "x", "cwd": "/home/u/../etc", "prompt": "p" }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        assert!(build_draft(spec, "t", now()).is_err());
    }

    #[test]
    fn custom_mcp_server_is_rejected_in_a_draft() {
        // F3: an AI client must not be able to inject an arbitrary
        // command via a Custom MCP server's config.
        let raw = r#"{
            "name": "x", "cwd": "/tmp", "prompt": "p",
            "mcp_servers": [
                { "kind": "custom", "name": "evil",
                  "config": { "command": "bash", "args": ["-c", "x"] } }
            ]
        }"#;
        let spec = DraftInput::from_json(raw)
            .unwrap()
            .normalize(&CliOverrides::default())
            .unwrap();
        let err = build_draft(spec, "t", now()).unwrap_err();
        assert!(matches!(err, AgentError::InvalidEnv(_)));
    }

    #[test]
    fn sdk_mcp_servers_are_rejected_in_a_draft() {
        // F3: the SDK `mcpServers` map normalizes to Custom servers,
        // which build_draft must also reject — an AI client attaches
        // the memory server via --attach-memory, never an arbitrary
        // custom command.
        let raw = r#"{
            "description": "x", "prompt": "y",
            "mcpServers": { "fs": { "command": "mcp-fs", "args": [] } }
        }"#;
        let ov = CliOverrides {
            name: Some("x".into()),
            cwd: Some("/tmp".into()),
            ..CliOverrides::default()
        };
        let spec = DraftInput::from_json(raw).unwrap().normalize(&ov).unwrap();
        assert!(build_draft(spec, "t", now()).is_err());
    }
}
