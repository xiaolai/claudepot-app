//! DTOs for the Agents Tauri surface.
//!
//! Outbound DTOs are projections of `claudepot_core::agent`
//! types into a JS-friendly shape. Inbound DTOs are the user's
//! form input plus a slug for the binary picker. No secrets cross
//! this boundary — agents don't carry credentials.

use claudepot_core::agent::{
    Agent, AgentBinary, AgentRun, CreatedVia, Lifecycle, McpServerRef, OutputFormat,
    PermissionMode, PlatformOptions, RateLimit, RunResult, SchedulerCapabilities, Trigger,
    TriggerKind,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummaryDto {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub binary_kind: String,
    pub binary_route_id: Option<String>,
    pub model: Option<String>,
    pub cwd: String,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub max_budget_usd: Option<f64>,
    pub trigger_kind: String,
    pub cron: Option<String>,
    pub timezone: Option<String>,
    /// Set only when `trigger_kind == "event"`. v1 value:
    /// `"session_settled"` (PRD §7.1).
    #[serde(default)]
    pub event_kind: Option<String>,
    /// Debounce window for a `session_settled` event trigger, in
    /// seconds. `None` for non-event triggers.
    #[serde(default)]
    pub event_debounce_secs: Option<u64>,
    /// `"draft"` or `"installed"`. Read-only — the GUI arms an
    /// agent (draft -> installed); see Phase 2 of the Agents PRD.
    pub lifecycle: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&Agent> for AgentSummaryDto {
    fn from(a: &Agent) -> Self {
        let (binary_kind, binary_route_id) = match &a.binary {
            AgentBinary::FirstParty => ("first_party".to_string(), None),
            AgentBinary::Route { route_id } => ("route".to_string(), Some(route_id.to_string())),
        };
        let (trigger_kind, cron, tz) = match &a.trigger {
            Trigger::Cron { cron, timezone } => {
                ("cron".to_string(), Some(cron.clone()), timezone.clone())
            }
            Trigger::Manual => ("manual".to_string(), None, None),
            // `event` triggers carry no cron/timezone; the
            // event-specific shape is exposed via `event_kind` /
            // `event_debounce_secs` below.
            Trigger::Event { .. } => ("event".to_string(), None, None),
        };
        let (event_kind, event_debounce_secs) = match &a.trigger {
            Trigger::Event {
                event: claudepot_core::agent::EventKind::SessionSettled { debounce_secs },
            } => (Some("session_settled".to_string()), Some(*debounce_secs)),
            _ => (None, None),
        };
        AgentSummaryDto {
            id: a.id.to_string(),
            name: a.name.clone(),
            display_name: a.display_name.clone(),
            description: a.description.clone(),
            enabled: a.enabled,
            binary_kind,
            binary_route_id,
            model: a.model.clone(),
            cwd: a.cwd.clone(),
            permission_mode: a.permission_mode.as_cli_flag().to_string(),
            allowed_tools: a.allowed_tools.clone(),
            max_budget_usd: a.max_budget_usd,
            trigger_kind,
            cron,
            timezone: tz,
            event_kind,
            event_debounce_secs,
            lifecycle: lifecycle_str(a.lifecycle).to_string(),
            created_at: a.created_at.to_rfc3339(),
            updated_at: a.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDetailsDto {
    pub summary: AgentSummaryDto,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub add_dir: Vec<String>,
    pub fallback_model: Option<String>,
    pub output_format: String,
    pub json_schema: Option<String>,
    pub bare: bool,
    pub extra_env: std::collections::BTreeMap<String, String>,
    pub platform_options: PlatformOptionsDto,
    pub log_retention_runs: u32,
    // ---- Agent-spec fields (Phase 1) ----
    pub disallowed_tools: Vec<String>,
    pub mcp_servers: Vec<McpServerRefDto>,
    pub run_as: Option<String>,
    pub task_budget: Option<u64>,
    pub rate_limit: Option<RateLimitDto>,
    /// Audit field: who drafted this agent. Read-only. Free-text;
    /// `created_via` is the trustworthy signal.
    pub drafted_by: Option<String>,
    /// Immutable audit signal stamped by the code path that
    /// produced this agent. Wire form is the snake_case
    /// [`CreatedVia`] variant: `"gui"` / `"cli_draft"` / `"template"`.
    /// `#[serde(default)]` so older clients without the field
    /// continue to deserialize (default `"gui"`).
    #[serde(default = "default_created_via")]
    pub created_via: String,
}

fn default_created_via() -> String {
    "gui".to_string()
}

fn created_via_str(c: CreatedVia) -> &'static str {
    match c {
        CreatedVia::Gui => "gui",
        CreatedVia::CliDraft => "cli_draft",
        CreatedVia::Template => "template",
    }
}

impl From<&Agent> for AgentDetailsDto {
    fn from(a: &Agent) -> Self {
        AgentDetailsDto {
            summary: AgentSummaryDto::from(a),
            prompt: a.prompt.clone(),
            system_prompt: a.system_prompt.clone(),
            append_system_prompt: a.append_system_prompt.clone(),
            add_dir: a.add_dir.clone(),
            fallback_model: a.fallback_model.clone(),
            output_format: a.output_format.as_cli_flag().to_string(),
            json_schema: a.json_schema.clone(),
            bare: a.bare,
            extra_env: a.extra_env.clone(),
            platform_options: PlatformOptionsDto {
                wake_to_run: a.platform_options.wake_to_run,
                catch_up_if_missed: a.platform_options.catch_up_if_missed,
                run_when_logged_out: a.platform_options.run_when_logged_out,
            },
            log_retention_runs: a.log_retention_runs,
            disallowed_tools: a.disallowed_tools.clone(),
            mcp_servers: a.mcp_servers.iter().map(McpServerRefDto::from).collect(),
            run_as: a.run_as.clone(),
            task_budget: a.task_budget,
            rate_limit: a.rate_limit.as_ref().map(RateLimitDto::from),
            drafted_by: a.drafted_by.clone(),
            created_via: created_via_str(a.created_via).to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformOptionsDto {
    #[serde(default)]
    pub wake_to_run: bool,
    #[serde(default)]
    pub catch_up_if_missed: bool,
    #[serde(default)]
    pub run_when_logged_out: bool,
}

impl From<PlatformOptionsDto> for PlatformOptions {
    fn from(d: PlatformOptionsDto) -> Self {
        PlatformOptions {
            wake_to_run: d.wake_to_run,
            catch_up_if_missed: d.catch_up_if_missed,
            run_when_logged_out: d.run_when_logged_out,
        }
    }
}

/// Wire form of [`McpServerRef`]. `kind = "claudepot_memory"` carries
/// nothing else; `kind = "custom"` carries a name + an opaque config
/// object. Mirrors the `#[serde(tag = "kind")]` core enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpServerRefDto {
    ClaudepotMemory,
    Custom {
        name: String,
        config: serde_json::Value,
    },
}

impl From<&McpServerRef> for McpServerRefDto {
    fn from(r: &McpServerRef) -> Self {
        match r {
            McpServerRef::ClaudepotMemory => McpServerRefDto::ClaudepotMemory,
            McpServerRef::Custom { name, config } => McpServerRefDto::Custom {
                name: name.clone(),
                config: config.clone(),
            },
        }
    }
}

impl From<McpServerRefDto> for McpServerRef {
    fn from(d: McpServerRefDto) -> Self {
        match d {
            McpServerRefDto::ClaudepotMemory => McpServerRef::ClaudepotMemory,
            McpServerRefDto::Custom { name, config } => McpServerRef::Custom { name, config },
        }
    }
}

/// Wire form of [`RateLimit`]. Both fields optional; an all-null
/// value means "no rate limit".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitDto {
    #[serde(default)]
    pub min_interval_secs: Option<u64>,
    #[serde(default)]
    pub max_per_day: Option<u32>,
}

impl From<&RateLimit> for RateLimitDto {
    fn from(r: &RateLimit) -> Self {
        RateLimitDto {
            min_interval_secs: r.min_interval_secs,
            max_per_day: r.max_per_day,
        }
    }
}

impl From<RateLimitDto> for RateLimit {
    fn from(d: RateLimitDto) -> Self {
        RateLimit {
            min_interval_secs: d.min_interval_secs,
            max_per_day: d.max_per_day,
        }
    }
}

/// Stringify [`Lifecycle`] for the wire (`"draft"` / `"installed"`).
fn lifecycle_str(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Draft => "draft",
        Lifecycle::Installed => "installed",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCreateDto {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub binary_kind: String, // "first_party" or "route"
    pub binary_route_id: Option<String>,
    pub model: Option<String>,
    pub cwd: String,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub add_dir: Vec<String>,
    pub max_budget_usd: Option<f64>,
    pub fallback_model: Option<String>,
    pub output_format: String, // "text" | "json" | "stream-json"
    pub json_schema: Option<String>,
    pub bare: bool,
    pub extra_env: std::collections::BTreeMap<String, String>,
    /// Kind of trigger to install. Defaults to `"cron"` so existing
    /// call sites stay unchanged. `"manual"` builds a
    /// [`Trigger::Manual`] agent (no scheduler artifact, only
    /// Run-Now). `"event"` builds a `Trigger::Event` agent and
    /// requires `event_kind` + `event_debounce_secs`.
    #[serde(default)]
    pub trigger_kind: Option<String>,
    pub cron: String,
    pub timezone: Option<String>,
    /// Event variant for `trigger_kind == "event"`. v1 value:
    /// `"session_settled"` (PRD §7.1). Ignored otherwise.
    #[serde(default)]
    pub event_kind: Option<String>,
    /// Debounce window (seconds) for a `session_settled` event
    /// trigger. Ignored otherwise.
    #[serde(default)]
    pub event_debounce_secs: Option<u64>,
    pub platform_options: PlatformOptionsDto,
    #[serde(default = "default_log_retention")]
    pub log_retention_runs: u32,
    /// Set when this agent was instantiated from a bundled
    /// template. Drives template-aware post-run behavior.
    #[serde(default)]
    pub template_id: Option<String>,
    // ---- Agent-spec fields (Phase 1) ----
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerRefDto>,
    #[serde(default)]
    pub run_as: Option<String>,
    #[serde(default)]
    pub task_budget: Option<u64>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitDto>,
    /// Audit field. Set by the (Phase 2) `agent draft` CLI verb /
    /// template instantiation; the regular Add Agent flow leaves it
    /// `None`.
    #[serde(default)]
    pub drafted_by: Option<String>,
}

fn default_log_retention() -> u32 {
    50
}

/// Patch shape: omit a field (or send `null`) to leave it unchanged;
/// send a value to overwrite. Single `Option<T>` per field — the
/// previous `Option<Option<T>>` shape was indistinguishable from the
/// outer-only path under default serde and broke TS round-trips.
/// To "explicitly clear" an optional field via this DTO, the caller
/// sends an appropriate empty value (e.g. an empty string), and the
/// patch builder converts it on the way to the store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentUpdateDto {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub add_dir: Option<Vec<String>>,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
    #[serde(default)]
    pub json_schema: Option<String>,
    #[serde(default)]
    pub bare: Option<bool>,
    #[serde(default)]
    pub extra_env: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub platform_options: Option<PlatformOptionsDto>,
    #[serde(default)]
    pub log_retention_runs: Option<u32>,
    // ---- Agent-spec fields (Phase 1) ----
    #[serde(default)]
    pub disallowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_servers: Option<Vec<McpServerRefDto>>,
    /// Empty string clears `run_as` to `None`; a non-empty email
    /// pins the account. Omitted = leave unchanged.
    #[serde(default)]
    pub run_as: Option<String>,
    /// `Some(0)` clears the budget; any positive value sets it.
    /// Omitted = leave unchanged.
    #[serde(default)]
    pub task_budget: Option<u64>,
    /// A populated value sets the rate limit; an all-null value
    /// clears it. Omitted = leave unchanged.
    #[serde(default)]
    pub rate_limit: Option<RateLimitDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunDto {
    pub id: String,
    pub agent_id: String,
    pub started_at: String,
    pub ended_at: String,
    pub duration_ms: i64,
    pub exit_code: i32,
    pub result: Option<RunResultDto>,
    pub session_jsonl_path: Option<String>,
    pub stdout_log: String,
    pub stderr_log: String,
    pub trigger_kind: String,
    pub host_platform: String,
    pub claudepot_version: String,
    /// Files the run produced under its blueprint's output path.
    /// Empty for non-template agents and for runs whose
    /// template generated nothing yet.
    #[serde(default)]
    pub output_artifacts: Vec<OutputArtifactDto>,
    /// Pre-run gate decision recorded by `_prerun`.
    #[serde(default)]
    pub route_decision: Option<RouteDecisionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputArtifactDto {
    pub kind: String,
    pub path: String,
    pub format: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteDecisionDto {
    Ran {
        route_id: Option<String>,
    },
    Fallback {
        from: String,
        to: Option<String>,
        reason: String,
    },
    Skipped {
        reason: String,
    },
    SkippedAlerted {
        reason: String,
    },
}

impl From<AgentRun> for AgentRunDto {
    fn from(r: AgentRun) -> Self {
        use claudepot_core::agent::types::ArtifactKind;
        use claudepot_core::routes::RouteDecision;
        let output_artifacts = r
            .output_artifacts
            .into_iter()
            .map(|a| OutputArtifactDto {
                kind: match a.kind {
                    ArtifactKind::Report => "report",
                    ArtifactKind::PendingChanges => "pending_changes",
                    ArtifactKind::ApplyReceipt => "apply_receipt",
                    ArtifactKind::Email => "email",
                }
                .to_string(),
                path: a.path,
                format: a.format,
                bytes: a.bytes,
            })
            .collect();
        let route_decision = r.route_decision.map(|d| match d {
            RouteDecision::Ran { route_id } => RouteDecisionDto::Ran { route_id },
            RouteDecision::Fallback { from, to, reason } => {
                RouteDecisionDto::Fallback { from, to, reason }
            }
            RouteDecision::Skipped { reason } => RouteDecisionDto::Skipped { reason },
            RouteDecision::SkippedAlerted { reason } => RouteDecisionDto::SkippedAlerted { reason },
        });
        AgentRunDto {
            id: r.id,
            agent_id: r.agent_id.to_string(),
            started_at: r.started_at.to_rfc3339(),
            ended_at: r.ended_at.to_rfc3339(),
            duration_ms: r.duration_ms,
            exit_code: r.exit_code,
            result: r.result.map(RunResultDto::from),
            session_jsonl_path: r.session_jsonl_path,
            stdout_log: r.stdout_log,
            stderr_log: r.stderr_log,
            trigger_kind: match r.trigger_kind {
                TriggerKind::Scheduled => "scheduled".to_string(),
                TriggerKind::Manual => "manual".to_string(),
            },
            host_platform: format!("{:?}", r.host_platform).to_lowercase(),
            claudepot_version: r.claudepot_version,
            output_artifacts,
            route_decision,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResultDto {
    pub subtype: Option<String>,
    pub is_error: Option<bool>,
    pub num_turns: Option<i64>,
    pub total_cost_usd: Option<f64>,
    pub stop_reason: Option<String>,
    pub session_id: Option<String>,
    pub errors: Vec<String>,
}

impl From<RunResult> for RunResultDto {
    fn from(r: RunResult) -> Self {
        RunResultDto {
            subtype: r.subtype,
            is_error: r.is_error,
            num_turns: r.num_turns,
            total_cost_usd: r.total_cost_usd,
            stop_reason: r.stop_reason,
            session_id: r.session_id,
            errors: r.errors,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerCapabilitiesDto {
    pub wake_to_run: bool,
    pub catch_up_if_missed: bool,
    pub run_when_logged_out: bool,
    pub native_label: String,
    pub artifact_dir: Option<String>,
}

impl From<SchedulerCapabilities> for SchedulerCapabilitiesDto {
    fn from(c: SchedulerCapabilities) -> Self {
        SchedulerCapabilitiesDto {
            wake_to_run: c.wake_to_run,
            catch_up_if_missed: c.catch_up_if_missed,
            run_when_logged_out: c.run_when_logged_out,
            native_label: c.native_label.to_string(),
            artifact_dir: c.artifact_dir,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronValidationDto {
    pub valid: bool,
    pub error: Option<String>,
    /// Next 5 fire times (RFC3339, UTC). Empty when invalid.
    pub next_runs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameValidationDto {
    pub valid: bool,
    pub error: Option<String>,
    pub already_taken: bool,
}

pub fn parse_permission_mode(s: &str) -> Option<PermissionMode> {
    match s {
        "default" => Some(PermissionMode::Default),
        "acceptEdits" => Some(PermissionMode::AcceptEdits),
        "bypassPermissions" => Some(PermissionMode::BypassPermissions),
        "dontAsk" => Some(PermissionMode::DontAsk),
        "plan" => Some(PermissionMode::Plan),
        "auto" => Some(PermissionMode::Auto),
        _ => None,
    }
}

pub fn parse_output_format(s: &str) -> Option<OutputFormat> {
    match s {
        "text" => Some(OutputFormat::Text),
        "json" => Some(OutputFormat::Json),
        "stream-json" => Some(OutputFormat::StreamJson),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire-format pin: the renderer matches on the literal kinds
    /// "report" / "pending_changes" / "apply_receipt" / "email".
    /// If a new ArtifactKind is added in core, the From impl must
    /// add an arm — this test fails before the bug reaches
    /// AgentsSection's report-button rendering.
    #[test]
    fn artifact_kind_renders_snake_case_strings() {
        use chrono::Utc;
        use claudepot_core::agent::types::{
            AgentRun, ArtifactKind, HostPlatform, OutputArtifact, TriggerKind,
        };
        use claudepot_core::routes::RouteDecision;

        let make = |kind: ArtifactKind| AgentRun {
            id: format!("r-{}", Utc::now().timestamp_micros()),
            agent_id: uuid::Uuid::new_v4(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            duration_ms: 0,
            exit_code: 0,
            result: None,
            session_jsonl_path: None,
            stdout_log: String::new(),
            stderr_log: String::new(),
            trigger_kind: TriggerKind::Manual,
            host_platform: HostPlatform::Macos,
            claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
            output_artifacts: vec![OutputArtifact {
                kind,
                path: "/tmp/r.md".to_string(),
                format: "markdown".to_string(),
                bytes: 1024,
            }],
            route_decision: Some(RouteDecision::Ran { route_id: None }),
        };

        let cases = [
            (ArtifactKind::Report, "report"),
            (ArtifactKind::PendingChanges, "pending_changes"),
            (ArtifactKind::ApplyReceipt, "apply_receipt"),
            (ArtifactKind::Email, "email"),
        ];
        for (kind, expected) in cases {
            let dto = AgentRunDto::from(make(kind));
            assert_eq!(dto.output_artifacts[0].kind, expected);
            assert_eq!(dto.output_artifacts[0].path, "/tmp/r.md");
            assert_eq!(dto.output_artifacts[0].bytes, 1024);
        }
    }

    #[test]
    fn route_decision_serializes_with_kind_tag_snake_case() {
        let cases = [
            (
                RouteDecisionDto::Ran {
                    route_id: Some("r1".into()),
                },
                "ran",
            ),
            (
                RouteDecisionDto::Fallback {
                    from: "r1".into(),
                    to: None,
                    reason: "ctx".into(),
                },
                "fallback",
            ),
            (
                RouteDecisionDto::Skipped {
                    reason: "no route".into(),
                },
                "skipped",
            ),
            (
                RouteDecisionDto::SkippedAlerted {
                    reason: "alerted".into(),
                },
                "skipped_alerted",
            ),
        ];
        for (dto, expected_kind) in cases {
            let json = serde_json::to_value(&dto).unwrap();
            assert_eq!(json["kind"], expected_kind);
        }
    }

    #[test]
    fn run_dto_omits_trigger_kind_manual_label_consistently() {
        // Manual + Scheduled are both surfaced as lowercase strings.
        // This pins the literal that AgentCard / RunHistoryPanel
        // assume when filtering / labeling rows.
        use chrono::Utc;
        use claudepot_core::agent::types::{AgentRun, HostPlatform, TriggerKind};

        let mut r = AgentRun {
            id: "r1".into(),
            agent_id: uuid::Uuid::new_v4(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            duration_ms: 0,
            exit_code: 0,
            result: None,
            session_jsonl_path: None,
            stdout_log: String::new(),
            stderr_log: String::new(),
            trigger_kind: TriggerKind::Manual,
            host_platform: HostPlatform::Macos,
            claudepot_version: "0.0.0".into(),
            output_artifacts: vec![],
            route_decision: None,
        };
        assert_eq!(AgentRunDto::from(r.clone()).trigger_kind, "manual");
        r.trigger_kind = TriggerKind::Scheduled;
        assert_eq!(AgentRunDto::from(r).trigger_kind, "scheduled");
    }
}
