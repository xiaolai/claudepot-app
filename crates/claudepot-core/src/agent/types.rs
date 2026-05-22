//! Domain types for agents.
//!
//! `Agent` is the on-disk record. `AgentRun` is one
//! historical execution. Both are platform-agnostic; per-platform
//! materialization (launchd plist, Task Scheduler XML, systemd
//! units) is the concern of `super::scheduler`, not these types.
//!
//! See `dev-docs/agents-implementation-plan.md` §3 for the
//! authoritative schema description.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for an agent. Mirrors `RouteId`; we use
/// the same Uuid alias so callers don't have to learn a new shape.
pub type AgentId = Uuid;

/// A reference to an MCP server the agent should attach via
/// `claude --mcp-config`. `ClaudepotMemory` resolves at shim-render
/// time to a stdio entry running `claudepot mcp memory-server`;
/// `Custom` carries a verbatim MCP server config object that the
/// shim drops straight into the `--mcp-config` JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpServerRef {
    /// Claudepot's own memory server. One-click attachable from the
    /// GUI; the shim materializes a stdio entry pointing at the
    /// `claudepot` CLI's `mcp memory-server` subcommand.
    ClaudepotMemory,
    /// A user-supplied MCP server. `config` is an opaque MCP server
    /// config object (the value that would sit under one key of an
    /// `mcpServers` map); the shim does not interpret it.
    Custom {
        name: String,
        config: serde_json::Value,
    },
}

/// Claudepot-enforced rate limit for an agent. Distinct from
/// `task_budget` (a per-run token ceiling passed to `claude`): the
/// rate limit caps run *frequency* and is enforced by Claudepot,
/// not by the model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimit {
    /// Minimum number of seconds that must elapse between two runs.
    #[serde(default)]
    pub min_interval_secs: Option<u64>,
    /// Maximum number of runs allowed in a rolling 24-hour window.
    #[serde(default)]
    pub max_per_day: Option<u32>,
}

/// Agent lifecycle. A `Draft` carries **no automatic or scheduled
/// execution** — no launchd / systemd / Task Scheduler artifact is
/// materialized and the event orchestrator never fires it. It is
/// *not* unconditionally inert: an explicit GUI "Run now" can still
/// execute a draft, and that path now routes through the same
/// human-confirmation surface as install (grill finding F16). The
/// `Draft -> Installed` transition is human-only (enforced by the
/// GUI). Default is `Draft`: a freshly-deserialized record with no
/// `lifecycle` field is conservatively treated as a draft, EXCEPT
/// the v1->v2 store migration which upgrades every pre-existing
/// record to `Installed` (they were already armed).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// No automatic execution: no scheduler artifact, no event
    /// binding. An explicit Run-Now can still execute it.
    #[default]
    Draft,
    /// Armed. The scheduler artifact is live.
    Installed,
}

/// One scheduled or manually-triggered `claude -p` run definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub binary: AgentBinary,
    #[serde(default)]
    pub model: Option<String>,
    pub cwd: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub add_dir: Vec<String>,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default = "default_output_format")]
    pub output_format: OutputFormat,
    #[serde(default)]
    pub json_schema: Option<String>,
    #[serde(default)]
    pub bare: bool,
    #[serde(default)]
    pub extra_env: std::collections::BTreeMap<String, String>,
    pub trigger: Trigger,
    #[serde(default)]
    pub platform_options: PlatformOptions,
    #[serde(default = "default_log_retention")]
    pub log_retention_runs: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_managed")]
    pub claudepot_managed: bool,
    /// Set when this agent was instantiated from a bundled
    /// template. Drives template-aware post-run behavior in
    /// `record_run` (output-artifact discovery, apply-sidecar
    /// parsing, caregiver SMTP delivery). `None` for agents
    /// created via the regular Add Agent flow.
    #[serde(default)]
    pub template_id: Option<String>,

    // ---- Agent-spec fields (Phase 1) ----
    /// Tools the agent is *forbidden* to use — `--disallowed-tools`.
    /// Whitelists (`allowed_tools`) are preferred; this is the
    /// blacklist counterpart for callers who need it.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// MCP servers to attach via `--mcp-config`. Claudepot's own
    /// memory server is one-click attachable; see [`McpServerRef`].
    #[serde(default)]
    pub mcp_servers: Vec<McpServerRef>,
    /// Account email this agent runs as. `None` = run as whatever
    /// account is CLI-active at fire time. Phase 1: per-run
    /// credential injection is not yet wired — a `Some(email)`
    /// value is recorded but the shim still defaults to the active
    /// account (see `shim.rs`).
    #[serde(default)]
    pub run_as: Option<String>,
    /// Per-run token ceiling, passed to `claude --task-budget` so
    /// the model paces itself. Caps one run's spend.
    #[serde(default)]
    pub task_budget: Option<u64>,
    /// Claudepot-enforced run-frequency limit. See [`RateLimit`].
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,

    // ---- Lifecycle (Phase 1) ----
    /// Draft vs. installed. Default `Draft`; only the GUI may set
    /// `Installed`. See [`Lifecycle`].
    #[serde(default)]
    pub lifecycle: Lifecycle,
    /// Actor id recorded when this agent was AI- or
    /// template-drafted (e.g. `claude-code@2026-05-22`,
    /// `template:session-narrator`). Audit trail; `None` for
    /// agents created via the regular Add Agent flow.
    #[serde(default)]
    pub drafted_by: Option<String>,
}

fn default_enabled() -> bool {
    true
}
fn default_output_format() -> OutputFormat {
    OutputFormat::Json
}
fn default_log_retention() -> u32 {
    50
}
fn default_managed() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentBinary {
    /// `which claude` at run time. Honors whatever account is in
    /// the Claudepot CLI slot.
    FirstParty,
    /// A registered third-party route's wrapper binary at
    /// `<claudepot_data_dir>/bin/<wrapper-name>`.
    Route { route_id: Uuid },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    DontAsk,
    Plan,
    Auto,
}

impl PermissionMode {
    /// Flag value passed to `claude --permission-mode`.
    pub fn as_cli_flag(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::BypassPermissions => "bypassPermissions",
            PermissionMode::DontAsk => "dontAsk",
            PermissionMode::Plan => "plan",
            PermissionMode::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    Text,
    Json,
    StreamJson,
}

impl OutputFormat {
    pub fn as_cli_flag(self) -> &'static str {
        match self {
            OutputFormat::Text => "text",
            OutputFormat::Json => "json",
            OutputFormat::StreamJson => "stream-json",
        }
    }
}

/// The signal an [`Trigger::Event`] reacts to.
///
/// Phase 3 (PRD §7) ships exactly one variant: [`EventKind::SessionSettled`].
/// `fs-watch` / `webhook` / `usage-threshold` are PRD-deferred siblings
/// (§13) — do NOT add them here without a PRD update.
///
/// `#[serde(tag = "kind")]` keeps the wire form forward-compatible: a
/// future variant added by a newer build deserializes cleanly here
/// only if this build knows it, so the enum is versioned by its own
/// variant set rather than a schema number.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// A CC/Codex session whose `.jsonl` transcript has been idle
    /// (unchanged) for `debounce_secs` — i.e. the session has
    /// *finished*, not merely *grown*. Fires exactly once per
    /// (agent, session) pair via the event-state ledger.
    SessionSettled {
        /// Seconds of transcript inactivity before the session is
        /// considered settled. Defaults to [`DEFAULT_DEBOUNCE_SECS`]
        /// when absent so older / hand-authored records stay valid.
        #[serde(default = "default_debounce_secs")]
        debounce_secs: u64,
    },
}

/// Default `session-settled` debounce: 10 minutes. A session quiet
/// for this long has almost certainly ended (PRD §7.1).
pub const DEFAULT_DEBOUNCE_SECS: u64 = 600;

fn default_debounce_secs() -> u64 {
    DEFAULT_DEBOUNCE_SECS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Five-field cron with optional IANA timezone.
    Cron {
        cron: String,
        #[serde(default)]
        timezone: Option<String>,
    },
    /// On-demand only — never fires from a scheduler artifact.
    /// Required by template-driven agents (caregiver
    /// heartbeat, on-demand diagnostics) where Run Now is the
    /// sole entry point.
    ///
    /// Scheduler adapters short-circuit on this variant: no
    /// launchd plist, no systemd unit, no Task Scheduler XML
    /// is materialized; `next_runs` returns an empty vec.
    Manual,
    /// Reactive — fires when an in-app event matches (PRD §7).
    /// The Claudepot app process evaluates these in `run_tick`;
    /// they do **not** fire while the app is closed, and there is
    /// **no OS scheduler artifact** (launchd/cron cannot watch for
    /// "session settled"). Scheduler adapters treat this exactly
    /// like [`Trigger::Manual`] — install the shim, register
    /// nothing with the OS; the orchestrator
    /// (`src-tauri/src/agent_event_orchestrator.rs`) is what fires
    /// them.
    Event { event: EventKind },
}

impl Trigger {
    /// True for `Trigger::Manual`. Used by scheduler adapters
    /// to short-circuit registration.
    pub fn is_manual(&self) -> bool {
        matches!(self, Trigger::Manual)
    }

    /// True for `Trigger::Cron { .. }`.
    pub fn is_cron(&self) -> bool {
        matches!(self, Trigger::Cron { .. })
    }

    /// True for `Trigger::Event { .. }`.
    pub fn is_event(&self) -> bool {
        matches!(self, Trigger::Event { .. })
    }

    /// True when the trigger carries **no OS scheduler artifact** —
    /// `Manual` and `Event`. Scheduler adapters short-circuit
    /// registration on this: the shim is installed but launchd /
    /// systemd / Task Scheduler register nothing. `Manual` is
    /// Run-Now only; `Event` is fired by the in-app orchestrator.
    pub fn has_no_os_schedule(&self) -> bool {
        matches!(self, Trigger::Manual | Trigger::Event { .. })
    }
}

/// Cross-platform behavior toggles. Each scheduler adapter honors
/// what its OS supports; unsupported toggles are silently ignored
/// at the adapter level (and surfaced as greyed-out controls in
/// the UI per `agents_scheduler_capabilities`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformOptions {
    #[serde(default)]
    pub wake_to_run: bool,
    #[serde(default = "default_catch_up")]
    pub catch_up_if_missed: bool,
    #[serde(default)]
    pub run_when_logged_out: bool,
}

impl Default for PlatformOptions {
    fn default() -> Self {
        Self {
            wake_to_run: false,
            catch_up_if_missed: true,
            run_when_logged_out: false,
        }
    }
}

fn default_catch_up() -> bool {
    true
}

/// One historical run of an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRun {
    pub id: String,
    // on-disk JSON key kept as "automation_id" (persisted in
    // per-run result.json); renamed by the Phase 1 migration.
    #[serde(rename = "automation_id")]
    pub agent_id: AgentId,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: i64,
    pub exit_code: i32,
    #[serde(default)]
    pub result: Option<RunResult>,
    #[serde(default)]
    pub session_jsonl_path: Option<String>,
    pub stdout_log: String,
    pub stderr_log: String,
    pub trigger_kind: TriggerKind,
    pub host_platform: HostPlatform,
    pub claudepot_version: String,
    /// Files the agent produced under its blueprint's
    /// `output.path_template`. Discovered by `record_run` after
    /// `claude -p` exits. Empty for non-template agents and
    /// for runs whose template generated nothing yet.
    #[serde(default)]
    pub output_artifacts: Vec<OutputArtifact>,
    /// Decision recorded by the pre-run gate (`claudepot
    /// agent _prerun`). `None` when the run skipped the
    /// gate (legacy agents) or when no route was assigned.
    #[serde(default)]
    pub route_decision: Option<RouteDecision>,
}

/// Output produced by a template-driven run, persisted to the
/// run record so the Reports panel and the apply pipeline can
/// locate artifacts without scanning the filesystem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputArtifact {
    pub kind: ArtifactKind,
    pub path: String,
    /// MIME-ish format hint, mirroring the blueprint's
    /// `output.format` field. Common values: `markdown`, `json`,
    /// `text`.
    pub format: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Human-readable narrative — what the user opens via
    /// "View latest report".
    Report,
    /// Structured proposed-changes manifest for apply-pipeline
    /// templates. Read by the apply executor.
    PendingChanges,
    /// Receipt of a completed apply step. Persisted alongside
    /// the report.
    ApplyReceipt,
    /// Email body/subject artifact for caregiver-style
    /// templates that deliver via SMTP.
    Email,
}

/// Decision recorded by the pre-run gate before invoking
/// `claude -p`. The gate runs route-reachability probes and
/// applies the blueprint's `fallback_policy`.
///
/// See `dev-docs/templates-implementation-plan.md` §5.3 for the
/// truth table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteDecision {
    /// Run proceeded against the assigned route (or the default
    /// route if `route_id` is `None`).
    Ran { route_id: Option<String> },
    /// Assigned route was unreachable; fell back to the default
    /// route. Only legal when `privacy != local`.
    Fallback {
        from: String,
        to: Option<String>,
        reason: String,
    },
    /// Run skipped silently (assigned route unreachable + policy
    /// = `skip`, or the route was outright invalid).
    Skipped { reason: String },
    /// Run skipped and a notification was posted (policy =
    /// `alert`, or `privacy = local` and the local route is
    /// unreachable).
    SkippedAlerted { reason: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Scheduled,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostPlatform {
    Macos,
    Windows,
    Linux,
    Other,
}

impl HostPlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            HostPlatform::Macos
        } else if cfg!(target_os = "windows") {
            HostPlatform::Windows
        } else if cfg!(target_os = "linux") {
            HostPlatform::Linux
        } else {
            HostPlatform::Other
        }
    }
}

/// The terminal `result` event from `claude -p --output-format=json`.
/// All fields optional so we tolerate schema drift between CC versions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunResult {
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub num_turns: Option<i64>,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_mode_flags() {
        assert_eq!(
            PermissionMode::BypassPermissions.as_cli_flag(),
            "bypassPermissions"
        );
        assert_eq!(PermissionMode::AcceptEdits.as_cli_flag(), "acceptEdits");
        assert_eq!(PermissionMode::Default.as_cli_flag(), "default");
    }

    #[test]
    fn output_format_flags() {
        assert_eq!(OutputFormat::Json.as_cli_flag(), "json");
        assert_eq!(OutputFormat::StreamJson.as_cli_flag(), "stream-json");
        assert_eq!(OutputFormat::Text.as_cli_flag(), "text");
    }

    #[test]
    fn platform_options_default_catches_up() {
        let p = PlatformOptions::default();
        assert!(!p.wake_to_run);
        assert!(p.catch_up_if_missed);
        assert!(!p.run_when_logged_out);
    }

    #[test]
    fn host_platform_round_trip() {
        // Just exercise the cfg branch on the host so dead-code lint
        // doesn't fire in any single-OS test run.
        let host = HostPlatform::current();
        let json = serde_json::to_string(&host).unwrap();
        let back: HostPlatform = serde_json::from_str(&json).unwrap();
        assert_eq!(host, back);
    }

    #[test]
    fn agent_round_trip_minimal() {
        let now = chrono::Utc::now();
        let a = Agent {
            id: Uuid::new_v4(),
            name: "morning-pr".into(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AgentBinary::FirstParty,
            model: Some("sonnet".into()),
            cwd: "/tmp".into(),
            prompt: "say hi".into(),
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: PermissionMode::DontAsk,
            allowed_tools: vec!["Read".into()],
            add_dir: vec![],
            max_budget_usd: Some(0.5),
            fallback_model: None,
            output_format: OutputFormat::Json,
            json_schema: None,
            bare: false,
            extra_env: Default::default(),
            trigger: Trigger::Cron {
                cron: "0 9 * * *".into(),
                timezone: None,
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
            rate_limit: None,
            lifecycle: Lifecycle::Draft,
            drafted_by: None,
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: Agent = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn lifecycle_defaults_to_draft() {
        assert_eq!(Lifecycle::default(), Lifecycle::Draft);
    }

    #[test]
    fn lifecycle_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&Lifecycle::Installed).unwrap(),
            "\"installed\""
        );
        assert_eq!(
            serde_json::to_string(&Lifecycle::Draft).unwrap(),
            "\"draft\""
        );
    }

    #[test]
    fn mcp_server_ref_round_trip() {
        let memory = McpServerRef::ClaudepotMemory;
        let custom = McpServerRef::Custom {
            name: "fs".into(),
            config: serde_json::json!({ "command": "mcp-fs", "args": ["/tmp"] }),
        };
        for r in [memory, custom] {
            let s = serde_json::to_string(&r).unwrap();
            let back: McpServerRef = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
        // Wire form pin: the tag is `kind` with snake_case values.
        let v = serde_json::to_value(McpServerRef::ClaudepotMemory).unwrap();
        assert_eq!(v["kind"], "claudepot_memory");
    }

    #[test]
    fn event_trigger_round_trips() {
        let t = Trigger::Event {
            event: EventKind::SessionSettled {
                debounce_secs: 900,
            },
        };
        let s = serde_json::to_string(&t).unwrap();
        let back: Trigger = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
        assert!(t.is_event());
        assert!(t.has_no_os_schedule());
        assert!(!t.is_manual());
        assert!(!t.is_cron());
    }

    #[test]
    fn event_trigger_debounce_defaults_when_absent() {
        // A hand-authored / forward-compat record with no
        // `debounce_secs` deserializes to the 10-minute default.
        let raw =
            r#"{"kind":"event","event":{"kind":"session_settled"}}"#;
        let t: Trigger = serde_json::from_str(raw).unwrap();
        match t {
            Trigger::Event {
                event: EventKind::SessionSettled { debounce_secs },
            } => assert_eq!(debounce_secs, DEFAULT_DEBOUNCE_SECS),
            other => panic!("expected SessionSettled, got {other:?}"),
        }
    }

    #[test]
    fn event_trigger_wire_form_is_tagged() {
        let t = Trigger::Event {
            event: EventKind::SessionSettled {
                debounce_secs: DEFAULT_DEBOUNCE_SECS,
            },
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "event");
        assert_eq!(v["event"]["kind"], "session_settled");
        assert_eq!(v["event"]["debounce_secs"], DEFAULT_DEBOUNCE_SECS);
    }

    #[test]
    fn manual_and_event_have_no_os_schedule_cron_does() {
        assert!(Trigger::Manual.has_no_os_schedule());
        assert!(Trigger::Event {
            event: EventKind::SessionSettled {
                debounce_secs: DEFAULT_DEBOUNCE_SECS
            }
        }
        .has_no_os_schedule());
        assert!(!Trigger::Cron {
            cron: "0 9 * * *".into(),
            timezone: None
        }
        .has_no_os_schedule());
    }

    #[test]
    fn rate_limit_round_trip_and_defaults() {
        let rl = RateLimit {
            min_interval_secs: Some(3600),
            max_per_day: Some(24),
        };
        let s = serde_json::to_string(&rl).unwrap();
        let back: RateLimit = serde_json::from_str(&s).unwrap();
        assert_eq!(rl, back);
        // Empty object deserializes to all-None.
        let empty: RateLimit = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, RateLimit::default());
    }
}
