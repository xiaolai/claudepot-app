//! IPC commands for the Settings → "Claude Code behavior" extended-
//! thinking toggle.
//!
//! Reads / writes CC's user-level `alwaysThinkingEnabled` via
//! `claudepot_core::thinking_toggle`. Global-only (user settings +
//! `MAX_THINKING_TOKENS` env override), so there is no `project_root`
//! argument. Per `rules/architecture.md`, NO business logic here —
//! resolution + write rules live in core. `ThinkingState` is a plain
//! bool/enum struct with no path or secret fields, so it crosses to JS
//! directly rather than through a hand-mirrored DTO.

use crate::dto_error::ErrorDto;
use claudepot_core::thinking_toggle::{
    resolve_thinking_enabled, set_thinking_enabled, ThinkingState,
};

/// `thinking_state` — read the current extended-thinking default
/// (`MAX_THINKING_TOKENS` env + `~/.claude/settings.json`).
#[tauri::command]
pub async fn thinking_state() -> Result<ThinkingState, ErrorDto> {
    Ok(resolve_thinking_enabled())
}

/// `thinking_set` — set the extended-thinking default. `enabled = true`
/// returns to CC's default (key cleared); `false` writes
/// `alwaysThinkingEnabled: false`. Returns the re-resolved state.
///
/// The `write setting: ` prefix this used to prepend is gone —
/// `SettingsWriteError` already names what failed, and the UI owns the
/// framing for what it was attempting. See `crate::dto_error`.
#[tauri::command]
pub async fn thinking_set(enabled: bool) -> Result<ThinkingState, ErrorDto> {
    set_thinking_enabled(enabled)?;
    Ok(resolve_thinking_enabled())
}
