//! DTOs for the per-project permission surface.
//!
//! Mirrors the `dto_*` sharding convention. No secrets cross here —
//! permission modes are public CC settings values.

use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::{PermissionDecisionSource, PermissionState};
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
    /// True only for `bypassPermissions` — the dashboard flags these.
    pub is_elevated: bool,
    /// The active Claudepot grant for this project, if one is in
    /// effect. `None` for an un-elevated project *or* a project the
    /// user elevated by hand-editing settings (elevated, but not
    /// Claudepot-managed — the UI distinguishes the two).
    pub active_grant: Option<GrantDto>,
}

/// A live time-boxed grant.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantDto {
    /// The mode Claudepot set (almost always `bypassPermissions`).
    pub granted_mode: String,
    /// What the layer held before the grant; `null` means the key was
    /// absent and revert will clear it.
    pub previous_mode: Option<String>,
    /// Epoch-ms the grant was created.
    pub granted_at_ms: i64,
    /// Epoch-ms the orchestrator will auto-revert.
    pub expires_at_ms: i64,
}

impl From<&Grant> for GrantDto {
    fn from(g: &Grant) -> Self {
        Self {
            granted_mode: g.granted_mode.as_wire_str().to_string(),
            previous_mode: g.previous_mode.as_ref().map(|m| m.as_wire_str().to_string()),
            granted_at_ms: g.granted_at.timestamp_millis(),
            expires_at_ms: g.expires_at.timestamp_millis(),
        }
    }
}

/// Build a [`ProjectPermissionDto`] from a resolved state + optional
/// active grant.
pub fn project_permission_dto(
    project_path: String,
    state: &PermissionState,
    active_grant: Option<&Grant>,
) -> ProjectPermissionDto {
    ProjectPermissionDto {
        project_path,
        effective_mode: state.effective.as_wire_str().to_string(),
        decided_by: decision_source_str(state.decided_by).to_string(),
        is_elevated: state.effective.is_elevated(),
        active_grant: active_grant.map(GrantDto::from),
    }
}

fn decision_source_str(s: PermissionDecisionSource) -> &'static str {
    match s {
        PermissionDecisionSource::LocalProjectSettings => "local_project_settings",
        PermissionDecisionSource::ProjectSettings => "project_settings",
        PermissionDecisionSource::UserSettings => "user_settings",
        PermissionDecisionSource::Default => "default",
    }
}
