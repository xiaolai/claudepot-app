//! IPC commands for the Global → Memory "Background memory
//! consolidation" card.
//!
//! Reads / writes CC's user-level `autoDreamEnabled` via
//! `claudepot_core::auto_dream`. Three-state (Default/On/Off) because
//! the absent case falls through to a GrowthBook server flag Claudepot
//! can't observe. Global-only, so there is no `project_root` argument.
//! Per `rules/architecture.md`, NO business logic here. `AutoDreamState`
//! is a plain enum/bool struct with no path or secret fields, so it
//! crosses to JS directly rather than through a hand-mirrored DTO.

use claudepot_core::auto_dream::{
    resolve_auto_dream, set_auto_dream, AutoDreamMode, AutoDreamState,
};

/// `auto_dream_state` — read the current consolidation state
/// (`~/.claude/settings.json` + the auto-memory dependency).
#[tauri::command]
pub async fn auto_dream_state() -> Result<AutoDreamState, String> {
    Ok(resolve_auto_dream())
}

/// `auto_dream_set` — set the consolidation default. `mode` is one of
/// `"default" | "on" | "off"`. Returns the re-resolved state.
#[tauri::command]
pub async fn auto_dream_set(mode: String) -> Result<AutoDreamState, String> {
    let m = match mode.as_str() {
        "default" => AutoDreamMode::Default,
        "on" => AutoDreamMode::On,
        "off" => AutoDreamMode::Off,
        other => return Err(format!("unknown mode {other}; want default|on|off")),
    };
    set_auto_dream(m).map_err(|e| format!("write setting: {e}"))?;
    Ok(resolve_auto_dream())
}
