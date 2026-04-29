//! DTOs for the Automations Tauri surface.
//!
//! Outbound DTOs are projections of `claudepot_core::automations`
//! types into a JS-friendly shape. Inbound DTOs are the user's
//! form input plus a slug for the binary picker. No secrets cross
//! this boundary — automations don't carry credentials.

use claudepot_core::automations::{
    Automation, AutomationBinary, AutomationRun, OutputFormat, PermissionMode, PlatformOptions,
    RunResult, SchedulerCapabilities, Trigger, TriggerKind,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationSummaryDto {
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
    pub created_at: String,
    pub updated_at: String,
}

impl From<&Automation> for AutomationSummaryDto {
    fn from(a: &Automation) -> Self {
        let (binary_kind, binary_route_id) = match &a.binary {
            AutomationBinary::FirstParty => ("first_party".to_string(), None),
            AutomationBinary::Route { route_id } => {
                ("route".to_string(), Some(route_id.to_string()))
            }
        };
        let (trigger_kind, cron, tz) = match &a.trigger {
            Trigger::Cron { cron, timezone } => {
                ("cron".to_string(), Some(cron.clone()), timezone.clone())
            }
        };
        AutomationSummaryDto {
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
            created_at: a.created_at.to_rfc3339(),
            updated_at: a.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationDetailsDto {
    pub summary: AutomationSummaryDto,
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
}

impl From<&Automation> for AutomationDetailsDto {
    fn from(a: &Automation) -> Self {
        AutomationDetailsDto {
            summary: AutomationSummaryDto::from(a),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationCreateDto {
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
    pub cron: String,
    pub timezone: Option<String>,
    pub platform_options: PlatformOptionsDto,
    #[serde(default = "default_log_retention")]
    pub log_retention_runs: u32,
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
pub struct AutomationUpdateDto {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRunDto {
    pub id: String,
    pub automation_id: String,
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
}

impl From<AutomationRun> for AutomationRunDto {
    fn from(r: AutomationRun) -> Self {
        AutomationRunDto {
            id: r.id,
            automation_id: r.automation_id.to_string(),
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
