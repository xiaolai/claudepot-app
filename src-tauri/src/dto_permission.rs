//! DTOs for the per-project permission surface.
//!
//! Mirrors the `dto_*` sharding convention. No secrets cross here —
//! permission modes are public CC settings values.

use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::{
    IgnoredValue, PermissionDecisionSource, PermissionState,
};
use claudepot_core::settings_writer::SettingsLayer;
use serde::Serialize;

/// One project's permission state, as the dashboard renders it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPermissionDto {
    /// Canonical project root (the row identity).
    pub project_path: String,
    /// `permissions.defaultMode` CC will actually use, as a wire
    /// string (`default` / `bypassPermissions` / …).
    pub effective_mode: String,
    /// Which settings layer decided `effective_mode`.
    pub decided_by: String,
    /// True only for `bypassPermissions` — which, since CC 2.1.257,
    /// only user or managed settings can produce.
    pub is_elevated: bool,
    /// A `bypassPermissions` / `auto` value sitting in a project-scope
    /// file that CC ignores. The pane renders it as a stale key with a
    /// repair action, never as "elevated".
    pub ignored_value: Option<IgnoredValueDto>,
    /// The active Claudepot grant for this project, if one is in
    /// effect.
    pub active_grant: Option<GrantDto>,
    /// Whether Claude Code's `PreToolUse` hook entry is present and
    /// points at this binary. `false` with an active grant means the
    /// grant is inert — the one state the pane must never show as
    /// "active".
    pub hook_installed: bool,
    /// First CC release that ignores `bypassPermissions` from project
    /// files; the pane quotes it rather than hardcoding a number.
    pub project_scope_ignores_since: String,
}

/// A stale `defaultMode` value CC will not honour.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredValueDto {
    /// `local_project` or `project`. Only the first is Claudepot-
    /// writable, so only the first gets a "remove" action.
    pub layer: String,
    pub mode: String,
}

/// A live grant — time-boxed or sticky.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantDto {
    /// Epoch-ms the grant was created.
    pub granted_at_ms: i64,
    /// Epoch-ms the grant lapses. `null` means the grant is **sticky**
    /// — never auto-expired; the user removes it explicitly via the
    /// Permissions UI.
    pub expires_at_ms: Option<i64>,
}

impl From<&Grant> for GrantDto {
    fn from(g: &Grant) -> Self {
        Self {
            granted_at_ms: g.granted_at.timestamp_millis(),
            expires_at_ms: g.expires_at.map(|t| t.timestamp_millis()),
        }
    }
}

impl From<&IgnoredValue> for IgnoredValueDto {
    fn from(v: &IgnoredValue) -> Self {
        Self {
            layer: layer_str(v.layer).to_string(),
            mode: v.mode.as_wire_str().to_string(),
        }
    }
}

/// Build a [`ProjectPermissionDto`] from a resolved state + optional
/// active grant.
pub fn project_permission_dto(
    project_path: String,
    state: &PermissionState,
    active_grant: Option<&Grant>,
    hook_installed: bool,
) -> ProjectPermissionDto {
    ProjectPermissionDto {
        project_path,
        effective_mode: state.effective.as_wire_str().to_string(),
        decided_by: decision_source_str(state.decided_by).to_string(),
        is_elevated: state.effective.is_elevated(),
        ignored_value: state.ignored.as_ref().map(IgnoredValueDto::from),
        active_grant: active_grant.map(GrantDto::from),
        hook_installed,
        project_scope_ignores_since: claudepot_core::permission::PROJECT_SCOPE_IGNORES_SINCE
            .to_string(),
    }
}

fn decision_source_str(s: PermissionDecisionSource) -> &'static str {
    match s {
        PermissionDecisionSource::LocalProjectSettings => "local_project_settings",
        PermissionDecisionSource::ProjectSettings => "project_settings",
        PermissionDecisionSource::UserSettings => "user_settings",
        PermissionDecisionSource::Default => "default",
        PermissionDecisionSource::ProjectScopeIgnored => "project_scope_ignored",
    }
}

fn layer_str(l: SettingsLayer) -> &'static str {
    match l {
        SettingsLayer::LocalProject => "local_project",
        SettingsLayer::Project => "project",
        SettingsLayer::User => "user",
    }
}
