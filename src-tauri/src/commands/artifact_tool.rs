//! IPC commands for the Settings → General "Keep companion output
//! local" toggle.
//!
//! Reads / writes CC's user-level `enableArtifact` / `disableArtifact`
//! settings via `claudepot_core::artifact_toggle`. Global-only: CC
//! resolves these keys from user (+ policy / flag) settings, never
//! project layers, so there is no `project_root` argument.
//!
//! Per `rules/architecture.md`, NO business logic here — the
//! resolution + write rules all live in core. `ArtifactState` is a
//! plain bool/enum struct with no path or secret fields, so it crosses
//! to JS directly rather than through a hand-mirrored DTO.

use crate::dto_error::ErrorDto;
use claudepot_core::artifact_toggle::{
    resolve_artifact_enabled, set_artifact_enabled, ArtifactState,
};

/// `artifact_tool_state` — read the current Artifact-tool enablement
/// (env var + `~/.claude/settings.json`).
#[tauri::command]
pub async fn artifact_tool_state() -> Result<ArtifactState, ErrorDto> {
    Ok(resolve_artifact_enabled())
}

/// `artifact_tool_set` — set whether CC's Artifact (cloud-publish)
/// tool is enabled. `enabled = false` keeps companion output local.
/// Returns the re-resolved state so the UI stays consistent.
///
/// The `write setting: ` prefix is gone — `SettingsWriteError` names
/// what failed and the UI names what it was attempting. See
/// `crate::dto_error`.
#[tauri::command]
pub async fn artifact_tool_set(enabled: bool) -> Result<ArtifactState, ErrorDto> {
    set_artifact_enabled(enabled)?;
    Ok(resolve_artifact_enabled())
}
