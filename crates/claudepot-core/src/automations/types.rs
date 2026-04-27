//! Domain types for automations.
//!
//! `Automation` is the on-disk record. `AutomationRun` is one
//! historical execution. Both are platform-agnostic; per-platform
//! materialization (launchd plist, Task Scheduler XML, systemd
//! units) is the concern of `super::scheduler`, not these types.
//!
//! See `dev-docs/automations-implementation-plan.md` §3 for the
//! authoritative schema description.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for an automation. Mirrors `RouteId`; we use
/// the same Uuid alias so callers don't have to learn a new shape.
pub type AutomationId = Uuid;

/// One scheduled or manually-triggered `claude -p` run definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Automation {
    pub id: AutomationId,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub binary: AutomationBinary,
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
pub enum AutomationBinary {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Five-field cron with optional IANA timezone. Reactive
    /// triggers (fs-watch, webhook) land in v2 as additional
    /// variants here.
    Cron {
        cron: String,
        #[serde(default)]
        timezone: Option<String>,
    },
}

/// Cross-platform behavior toggles. Each scheduler adapter honors
/// what its OS supports; unsupported toggles are silently ignored
/// at the adapter level (and surfaced as greyed-out controls in
/// the UI per `automations_scheduler_capabilities`).
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

/// One historical run of an automation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutomationRun {
    pub id: String,
    pub automation_id: AutomationId,
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
        assert_eq!(PermissionMode::BypassPermissions.as_cli_flag(), "bypassPermissions");
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
    fn automation_round_trip_minimal() {
        let now = chrono::Utc::now();
        let a = Automation {
            id: Uuid::new_v4(),
            name: "morning-pr".into(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AutomationBinary::FirstParty,
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
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: Automation = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
