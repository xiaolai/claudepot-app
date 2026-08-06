//! IPC commands for the Settings → "Claude Code behavior" commit/PR
//! attribution control.
//!
//! Reads / writes CC's user-level `attribution` object (+ deprecated
//! `includeCoAuthoredBy` compat guard) via
//! `claudepot_core::attribution_settings`. Global-only. Per
//! `rules/architecture.md`, NO business logic here — the 3-mode write
//! rules (including the enhanced-PR guard) live in core.
//! `AttributionState` carries only user-authored attribution text (not
//! a secret), so it crosses to JS directly rather than through a DTO.

use crate::dto_error::{codes, ErrorDto};
use claudepot_core::attribution_settings::{
    resolve_attribution, set_attribution, AttributionMode, AttributionState,
};
use serde_json::json;

/// `attribution_state` — read the current commit/PR attribution state.
#[tauri::command]
pub async fn attribution_state() -> Result<AttributionState, ErrorDto> {
    Ok(resolve_attribution())
}

/// `attribution_set` — set the attribution mode. `mode` is one of
/// `"default" | "off" | "custom"`. For `"custom"`, `commit` and `pr`
/// carry the literal trailer / PR-body text (empty string allowed).
/// Returns the re-resolved state.
///
/// The `write setting: ` prefix is gone — `SettingsWriteError` names
/// what failed and the UI names what it was attempting. See
/// `crate::dto_error`.
#[tauri::command]
pub async fn attribution_set(
    mode: String,
    commit: Option<String>,
    pr: Option<String>,
) -> Result<AttributionState, ErrorDto> {
    let m = match mode.as_str() {
        "default" => AttributionMode::Default,
        "off" => AttributionMode::Off,
        "custom" => AttributionMode::Custom {
            commit: commit.unwrap_or_default(),
            pr: pr.unwrap_or_default(),
        },
        other => {
            return Err(ErrorDto::with_params(
                codes::ATTRIBUTION_UNKNOWN_MODE,
                json!({ "mode": other }),
                format!("unknown mode {other}; want default|off|custom"),
            ))
        }
    };
    set_attribution(m)?;
    Ok(resolve_attribution())
}
