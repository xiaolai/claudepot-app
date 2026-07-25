//! IPC commands for the Settings → "Claude Code behavior" model
//! allowlist editor.
//!
//! Reads / writes CC's user-level `availableModels` and
//! `enforceAvailableModels` via `claudepot_core::available_models`.
//! Global-only, so there is no `project_root` argument. Per
//! `rules/architecture.md`, NO business logic here — normalization,
//! the empty-list rule, and the enforce-is-inert rule all live in core.

use claudepot_core::available_models::{
    resolve_available_models, set_available_models, AvailableModelsState, ENFORCE_MIN_CC_VERSION,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AvailableModelsDto {
    pub entries: Vec<String>,
    pub enforce: Option<bool>,
    pub key_present: bool,
    /// True when any restriction is actually in force.
    pub restricts_models: bool,
    /// True when `enforceAvailableModels` is doing something. CC
    /// ignores it with an empty list, so the UI shows "set but inert"
    /// rather than implying Default is restricted.
    pub enforce_is_effective: bool,
    /// Minimum CC version that honors the enforce flag, for the
    /// inline note.
    pub enforce_min_cc_version: String,
}

fn dto(state: AvailableModelsState) -> AvailableModelsDto {
    AvailableModelsDto {
        restricts_models: state.restricts_models(),
        enforce_is_effective: state.enforce_is_effective(),
        entries: state.entries,
        enforce: state.enforce,
        key_present: state.key_present,
        enforce_min_cc_version: ENFORCE_MIN_CC_VERSION.to_string(),
    }
}

/// `available_models_state` — read the current allowlist.
#[tauri::command]
pub async fn available_models_state() -> Result<AvailableModelsDto, String> {
    Ok(dto(resolve_available_models()))
}

/// `available_models_set` — replace the allowlist and the enforce flag
/// in one atomic write. Returns the re-resolved state, whose `entries`
/// reflect core's normalization (trimmed, deduped, order preserved) so
/// the editor shows exactly what landed on disk.
#[tauri::command]
pub async fn available_models_set(
    entries: Vec<String>,
    enforce: bool,
) -> Result<AvailableModelsDto, String> {
    set_available_models(&entries, enforce).map_err(|e| e.to_string())?;
    Ok(dto(resolve_available_models()))
}
