//! Serde DTOs + enum converters for the Config command surface.
//!
//! Extracted so `commands_config.rs` stays focused on command bodies
//! and stays under the LOC ceiling. All types here are public so the
//! command module imports them verbatim.

use crate::config_dto::{flatten_files, kind_to_str, scope_kind_label, FileNodeDto};
use claudepot_core::config_view::{
    effective_mcp::McpSimulationMode,
    model::{DetectSource, EditorCandidate, EditorDefaults, Kind, LaunchKind, ScopeNode},
};
use serde::{Deserialize, Serialize};

// ---------- Tree scan DTOs --------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct ConfigTreeDto {
    pub scopes: Vec<ScopeNodeDto>,
    pub cwd: String,
    pub project_root: String,
    /// Platform-correct path to `<cwd>/.claude`, built via `Path::join`
    /// so Windows callers receive `C:\…\.claude` (not a mixed-separator
    /// `C:\…/.claude`). Rendered in the UI as a display-only string;
    /// the backend consumes the `abs_path` of each FileNode directly.
    pub config_home_dir: String,
    pub memory_slug: String,
    pub memory_slug_lossy: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScopeNodeDto {
    pub id: String,
    pub label: String,
    pub scope_type: String,
    pub recursive_count: usize,
    pub files: Vec<FileNodeDto>,
}

impl From<&ScopeNode> for ScopeNodeDto {
    fn from(s: &ScopeNode) -> Self {
        Self {
            id: s.id.clone(),
            label: s.label.clone(),
            scope_type: scope_kind_label(&s.scope),
            recursive_count: s.recursive_count,
            files: flatten_files(&s.children),
        }
    }
}

// ---------- Editors ---------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct EditorCandidateDto {
    pub id: String,
    pub label: String,
    pub binary_path: Option<String>,
    pub bundle_id: Option<String>,
    pub launch_kind: String,
    pub detected_via: String,
    pub supports_kinds: Option<Vec<Kind>>,
}

impl From<&EditorCandidate> for EditorCandidateDto {
    fn from(c: &EditorCandidate) -> Self {
        let launch_kind = match &c.launch {
            LaunchKind::Direct { .. } => "direct",
            LaunchKind::MacosOpenA { .. } => "macos-open-a",
            LaunchKind::EnvEditor => "env-editor",
            LaunchKind::SystemHandler => "system-handler",
        }
        .to_string();
        let detected_via = match &c.detected_via {
            DetectSource::PathBinary { .. } => "path-binary",
            DetectSource::MacosAppBundle { .. } => "macos-app",
            DetectSource::WindowsRegistry { .. } => "windows-registry",
            DetectSource::LinuxDesktopFile { .. } => "linux-desktop-file",
            DetectSource::EnvVar { .. } => "env-var",
            DetectSource::SystemDefault => "system-default",
            DetectSource::UserPicked { .. } => "user-picked",
        }
        .to_string();
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            binary_path: c.binary_path.as_ref().map(|p| p.display().to_string()),
            bundle_id: c.bundle_id.clone(),
            launch_kind,
            detected_via,
            supports_kinds: c.supports_kinds.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditorDefaultsDto {
    pub by_kind: std::collections::BTreeMap<String, String>,
    pub fallback: String,
}

impl From<&EditorDefaults> for EditorDefaultsDto {
    fn from(d: &EditorDefaults) -> Self {
        Self {
            by_kind: d
                .by_kind
                .iter()
                .map(|(k, v)| (kind_to_str(k).to_string(), v.clone()))
                .collect(),
            fallback: d.fallback.clone(),
        }
    }
}

// ---------- Content search --------------------------------------------

#[derive(Deserialize, Clone, Debug)]
pub struct SearchQueryDto {
    pub text: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub scope_filter: Option<Vec<String>>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchHitDto {
    pub search_id: String,
    pub node_id: String,
    pub line_number: u32,
    pub snippet: String,
    pub match_count_in_file: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchSummaryDto {
    pub search_id: String,
    pub total_hits: u32,
    pub capped: bool,
    pub skipped_large: u32,
    pub cancelled: bool,
}

impl SearchSummaryDto {
    /// Error details land in a trace log; the client sees `cancelled`.
    /// Kept as a no-op passthrough so future telemetry can attach
    /// without reshuffling call sites.
    pub fn with_error(self, _msg: &str) -> Self {
        self
    }
}

// ---------- Effective settings ----------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct ProvenanceLeafDto {
    /// Dotted JSON path — `"a.b[2].c"`.
    pub path: String,
    pub winner: String,
    pub contributors: Vec<String>,
    pub suppressed: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveSettingsDto {
    /// Fully merged JSON, with secrets masked.
    pub merged: serde_json::Value,
    /// One entry per primitive leaf.
    pub provenance: Vec<ProvenanceLeafDto>,
    /// Winning policy origin ("remote" | "mdm_admin" | …) or None.
    pub policy_winner: Option<String>,
    pub policy_errors: Vec<PolicyErrorDto>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PolicyErrorDto {
    pub origin: String,
    pub message: String,
}

// ---------- Effective MCP ---------------------------------------------

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum McpSimulationModeDto {
    Interactive,
    NonInteractive,
    SkipPermissions,
}

impl From<McpSimulationModeDto> for McpSimulationMode {
    fn from(d: McpSimulationModeDto) -> Self {
        match d {
            McpSimulationModeDto::Interactive => McpSimulationMode::Interactive,
            McpSimulationModeDto::NonInteractive => McpSimulationMode::NonInteractive,
            McpSimulationModeDto::SkipPermissions => McpSimulationMode::SkipPermissions,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveMcpDto {
    pub enterprise_lockout: bool,
    pub servers: Vec<EffectiveMcpServerDto>,
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveMcpServerDto {
    pub name: String,
    pub source_scope: String,
    pub contributors: Vec<String>,
    pub approval: String,
    /// E.g. "enable_all_project_mcp", "non_interactive_with_project_source_enabled"
    pub approval_reason: Option<String>,
    pub blocked_by: Option<String>,
    /// Server config JSON with secrets masked.
    pub masked: serde_json::Value,
}

/// Render a JSON path segment list as a dotted string.
pub fn render_path(segs: &[claudepot_core::config_view::model::JsonPathSeg]) -> String {
    use claudepot_core::config_view::model::JsonPathSeg;
    let mut out = String::new();
    for (i, seg) in segs.iter().enumerate() {
        match seg {
            JsonPathSeg::Key(k) => {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(k);
            }
            JsonPathSeg::Index(idx) => {
                out.push_str(&format!("[{idx}]"));
            }
        }
    }
    out
}

/// Stringify an MCP auto-approval reason for the JS boundary.
pub fn auto_reason_label(
    r: &claudepot_core::config_view::effective_mcp::AutoApprovalReason,
) -> String {
    use claudepot_core::config_view::effective_mcp::AutoApprovalReason;
    match r {
        AutoApprovalReason::EnableAllProjectMcp => "enable_all_project_mcp".into(),
        AutoApprovalReason::NonInteractiveWithProjectSourceEnabled => {
            "non_interactive_with_project_source_enabled".into()
        }
        AutoApprovalReason::SkipPermissionsWithProjectSourceEnabled => {
            "skip_permissions_with_project_source_enabled".into()
        }
    }
}
