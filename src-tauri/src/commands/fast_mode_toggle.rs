//! IPC commands for the Settings → "Claude Code behavior" fast-mode
//! toggle.
//!
//! Reads / writes CC's user-level `fastMode` and
//! `fastModePerSessionOptIn` via `claudepot_core::fast_mode_toggle`.
//! Global-only (user settings + `CLAUDE_CODE_DISABLE_FAST_MODE` env
//! override), so there is no `project_root` argument. Per
//! `rules/architecture.md`, NO business logic here — resolution + write
//! rules live in core. `FastModeState` is a plain bool/enum struct with
//! no path or secret fields, so it crosses to JS directly rather than
//! through a hand-mirrored DTO.

use claudepot_core::fast_mode_toggle::{
    resolve_fast_mode, set_fast_mode, set_per_session_opt_in, FastModeState,
    FAST_MODE_INPUT_PER_MTOK, FAST_MODE_MODELS, FAST_MODE_OUTPUT_PER_MTOK,
};
use serde::Serialize;

/// The facts the toggle's copy needs, sourced from core rather than
/// duplicated in the renderer — a hardcoded "$10/$50" in TSX would
/// drift the first time the rate moved.
#[derive(Debug, Serialize)]
pub struct FastModeFacts {
    /// Models fast mode runs on, e.g. `["claude-opus-5", …]`.
    pub models: Vec<String>,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

#[derive(Debug, Serialize)]
pub struct FastModeStateDto {
    #[serde(flatten)]
    pub state: FastModeState,
    pub facts: FastModeFacts,
}

fn dto(state: FastModeState) -> FastModeStateDto {
    FastModeStateDto {
        state,
        facts: FastModeFacts {
            models: FAST_MODE_MODELS.iter().map(|s| s.to_string()).collect(),
            input_per_mtok: FAST_MODE_INPUT_PER_MTOK,
            output_per_mtok: FAST_MODE_OUTPUT_PER_MTOK,
        },
    }
}

/// `fast_mode_state` — read the current fast-mode default
/// (`CLAUDE_CODE_DISABLE_FAST_MODE` env + `~/.claude/settings.json`).
#[tauri::command]
pub async fn fast_mode_state() -> Result<FastModeStateDto, String> {
    Ok(dto(resolve_fast_mode()))
}

/// `fast_mode_set` — set the fast-mode default. `enabled = true` writes
/// `fastMode: true`; `false` clears the key (CC's default). Returns the
/// re-resolved state.
#[tauri::command]
pub async fn fast_mode_set(enabled: bool) -> Result<FastModeStateDto, String> {
    set_fast_mode(enabled).map_err(|e| format!("write setting: {e}"))?;
    Ok(dto(resolve_fast_mode()))
}

/// `fast_mode_set_per_session` — set `fastModePerSessionOptIn`, which
/// makes every new session start with fast mode off regardless of the
/// saved preference.
#[tauri::command]
pub async fn fast_mode_set_per_session(required: bool) -> Result<FastModeStateDto, String> {
    set_per_session_opt_in(required).map_err(|e| format!("write setting: {e}"))?;
    Ok(dto(resolve_fast_mode()))
}
