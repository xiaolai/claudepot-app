//! Read + write CC's `autoDreamEnabled` setting — the "Background
//! memory consolidation" control.
//!
//! # CC's resolution model (verified against
//! `~/github/claude_code_src/src/services/autoDream/config.ts:13
//! isAutoDreamEnabled`)
//!
//! ```text
//! const setting = settings.autoDreamEnabled
//! if setting !== undefined:  return setting          // explicit wins
//! else:                      return growthbook('tengu_onyx_plover').enabled === true
//! ```
//!
//! The absence case falls through to a **GrowthBook server flag**
//! (`tengu_onyx_plover`) that Claudepot cannot observe. A binary switch
//! would therefore lie when the key is absent — it can't know CC's
//! current default. So this control is **three-state**:
//!
//! - **Default** (key absent) → CC decides via the server flag.
//! - **On** → write `autoDreamEnabled: true`.
//! - **Off** → write `autoDreamEnabled: false`.
//!
//! Consolidation additionally requires auto-memory to be on (it reuses
//! the auto-memory directory) plus not-Remote / not-Kairos runtime
//! gates. We surface the auto-memory dependency so the UI can explain
//! why a configured-On consolidation still won't run; the Remote/Kairos
//! gates are runtime-only and not represented here.
//!
//! Global-only: this is the per-user default, matching how the
//! auto-memory global card scopes to `~/.claude/settings.json`.

use crate::paths::claude_config_dir;
use crate::settings_writer::{
    clear_bool_setting, read_bool_setting, resolve_auto_memory_enabled_global, write_bool_setting,
    SettingsLayer, SettingsWriteError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Setting key in CC's `settings.json`.
pub const AUTO_DREAM_KEY: &str = "autoDreamEnabled";

/// The three states the control can hold. Serialized snake_case for the
/// JS wire: `default` / `on` / `off`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AutoDreamMode {
    /// Key absent — CC's server flag (`tengu_onyx_plover`) decides.
    Default,
    /// `autoDreamEnabled: true`.
    On,
    /// `autoDreamEnabled: false`.
    Off,
}

/// Aggregate state surfaced by the card.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoDreamState {
    /// Which of the three states the user setting currently expresses.
    pub mode: AutoDreamMode,
    /// `~/.claude/settings.json :: autoDreamEnabled`, if present.
    pub user_settings_value: Option<bool>,
    /// Whether auto-memory is effectively on (global resolution).
    /// Consolidation can't run without it, so the card disables editing
    /// and states the dependency when this is `false`.
    pub auto_memory_enabled: bool,
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Resolve the auto-dream state from user settings + the auto-memory
/// dependency. Pure read over env + filesystem — no side effects.
pub fn resolve_auto_dream() -> AutoDreamState {
    let user_settings_value =
        read_bool_setting(&user_settings_path(), AUTO_DREAM_KEY).unwrap_or(None);
    let mode = match user_settings_value {
        None => AutoDreamMode::Default,
        Some(true) => AutoDreamMode::On,
        Some(false) => AutoDreamMode::Off,
    };
    AutoDreamState {
        mode,
        user_settings_value,
        auto_memory_enabled: resolve_auto_memory_enabled_global().effective,
    }
}

/// Set the auto-dream default in `~/.claude/settings.json`.
///
/// - `Default` clears the key (CC's server flag decides).
/// - `On` / `Off` write the boolean.
///
/// Writes only the user layer.
pub fn set_auto_dream(mode: AutoDreamMode) -> Result<(), SettingsWriteError> {
    let anchor = Path::new("");
    match mode {
        AutoDreamMode::Default => clear_bool_setting(SettingsLayer::User, anchor, AUTO_DREAM_KEY),
        AutoDreamMode::On => write_bool_setting(SettingsLayer::User, anchor, AUTO_DREAM_KEY, true),
        AutoDreamMode::Off => {
            write_bool_setting(SettingsLayer::User, anchor, AUTO_DREAM_KEY, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path().join("config-dir"));
        fs::create_dir_all(tmp.path().join("config-dir")).unwrap();
        // Auto-memory dependency reads these — keep them clean.
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
        (tmp, lock)
    }

    fn write_user_settings(body: &str) {
        let p = user_settings_path();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
    }

    #[test]
    fn default_when_key_absent() {
        let (_t, _l) = isolated();
        let s = resolve_auto_dream();
        assert_eq!(s.mode, AutoDreamMode::Default);
        assert_eq!(s.user_settings_value, None);
        // Auto-memory defaults on, so the dependency is satisfied.
        assert!(s.auto_memory_enabled);
    }

    #[test]
    fn on_off_round_trip_and_preserve_rest() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"keep":1}"#);

        set_auto_dream(AutoDreamMode::On).unwrap();
        assert_eq!(resolve_auto_dream().mode, AutoDreamMode::On);

        set_auto_dream(AutoDreamMode::Off).unwrap();
        let s = resolve_auto_dream();
        assert_eq!(s.mode, AutoDreamMode::Off);
        assert_eq!(s.user_settings_value, Some(false));

        set_auto_dream(AutoDreamMode::Default).unwrap();
        let v: JsonValue =
            serde_json::from_slice(&fs::read(user_settings_path()).unwrap()).unwrap();
        assert!(v.get("autoDreamEnabled").is_none());
        assert_eq!(v["keep"], JsonValue::from(1));
    }

    #[test]
    fn reports_auto_memory_dependency_disabled() {
        let (_t, _l) = isolated();
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
        let s = resolve_auto_dream();
        assert!(!s.auto_memory_enabled);
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
    }
}
