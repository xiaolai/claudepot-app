//! Read + write CC's `alwaysThinkingEnabled` setting — the
//! "Extended thinking by default" toggle.
//!
//! # CC's resolution model (verified against
//! `~/github/claude_code_src/src/utils/thinking.ts:146
//! shouldEnableThinkingByDefault`)
//!
//! ```text
//! if MAX_THINKING_TOKENS (present & non-empty JS-truthy):
//!     enabled = parseInt(MAX_THINKING_TOKENS, 10) > 0     // hard override
//! else if settings.alwaysThinkingEnabled === false:
//!     enabled = false
//! else:
//!     enabled = true                                       // default: on
//! ```
//!
//! The settings type schema (`utils/settings/types.ts:696`) confirms:
//! "When false, thinking is disabled. When absent or true, thinking is
//! enabled automatically for supported models." So the default is
//! represented by *absence*, and CC's own `/config` UI clears the key
//! when it equals the default.
//!
//! We therefore write like CC does:
//! - **On** (default) → clear `alwaysThinkingEnabled`.
//! - **Off** → write `alwaysThinkingEnabled: false`.
//!
//! We never write `true`: it works, but it freezes a default CC
//! intentionally represents by absence, and would survive a future
//! change to that default.
//!
//! Global-only: this is the per-user default for new sessions. Project,
//! policy, flag, and per-process CLI args (`--thinking` /
//! `--max-thinking-tokens`) live outside this surface, exactly as the
//! artifact toggle scopes to user settings only.

use crate::paths::claude_config_dir;
use crate::settings_writer::{
    clear_bool_setting, read_bool_setting, write_bool_setting, SettingsLayer, SettingsWriteError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Setting key in CC's `settings.json`.
pub const ALWAYS_THINKING_KEY: &str = "alwaysThinkingEnabled";
/// Env var that hard-overrides the setting (JS-truthy → present).
pub const MAX_THINKING_TOKENS_ENV: &str = "MAX_THINKING_TOKENS";

/// What decided the effective enablement.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDecisionSource {
    /// `MAX_THINKING_TOKENS` is set — `parseInt(...) > 0` decides.
    EnvMaxThinkingTokens,
    /// `~/.claude/settings.json :: alwaysThinkingEnabled` (true/false).
    UserSettings,
    /// No source set the key — CC's built-in default (enabled) wins.
    Default,
}

/// Aggregate state surfaced by the toggle UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThinkingState {
    /// Whether extended thinking is on by default for new sessions.
    pub effective: bool,
    /// Why `effective` is what it is.
    pub decided_by: ThinkingDecisionSource,
    /// `false` when `MAX_THINKING_TOKENS` is forcing the decision — the
    /// toggle renders disabled with the reason shown.
    pub user_writable: bool,
    /// `~/.claude/settings.json :: alwaysThinkingEnabled`, if present.
    pub user_settings_value: Option<bool>,
    /// Whether `MAX_THINKING_TOKENS` is set (non-empty).
    pub env_max_thinking_tokens_set: bool,
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Mirror JS `parseInt(s, 10)`: skip leading whitespace, optional sign,
/// then consume base-10 digits and stop at the first non-digit. Returns
/// `None` when no digits are present (JS `NaN`) — `NaN > 0` is `false`.
///
/// Uses `i128` and saturates on overflow so an absurdly large token count
/// still reads as positive: JS returns a large *finite* number there (not
/// `NaN`), and the only thing the caller checks is the sign, so a
/// saturated `i128::MAX` keeps `> 0` parity instead of collapsing to
/// `None` (which would wrongly disable thinking).
fn js_parse_int(s: &str) -> Option<i128> {
    let t = s.trim_start();
    let bytes = t.as_bytes();
    let mut i = 0;
    let negative = if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        let neg = bytes[i] == b'-';
        i += 1;
        neg
    } else {
        false
    };
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    // Overflow → saturate on the parsed sign (JS yields ±a huge finite).
    Some(
        t[..i]
            .parse::<i128>()
            .unwrap_or(if negative { i128::MIN } else { i128::MAX }),
    )
}

/// Resolve the extended-thinking default from env + user settings.
/// Pure read over env + filesystem — no side effects.
pub fn resolve_thinking_enabled() -> ThinkingState {
    // CC: `if (process.env.MAX_THINKING_TOKENS)` — JS truthiness, so an
    // empty string counts as unset. `var` (UTF-8) is fine: a non-UTF8
    // token would fail parseInt anyway → disabled, matching NaN > 0.
    let env_raw = std::env::var(MAX_THINKING_TOKENS_ENV)
        .ok()
        .filter(|s| !s.is_empty());
    let user_settings_value =
        read_bool_setting(&user_settings_path(), ALWAYS_THINKING_KEY).unwrap_or(None);

    if let Some(raw) = env_raw {
        let effective = js_parse_int(&raw).is_some_and(|n| n > 0);
        return ThinkingState {
            effective,
            decided_by: ThinkingDecisionSource::EnvMaxThinkingTokens,
            user_writable: false,
            user_settings_value,
            env_max_thinking_tokens_set: true,
        };
    }

    // Only `=== false` disables; `true` and absent both mean enabled.
    if user_settings_value == Some(false) {
        return ThinkingState {
            effective: false,
            decided_by: ThinkingDecisionSource::UserSettings,
            user_writable: true,
            user_settings_value,
            env_max_thinking_tokens_set: false,
        };
    }
    ThinkingState {
        effective: true,
        decided_by: if user_settings_value == Some(true) {
            ThinkingDecisionSource::UserSettings
        } else {
            ThinkingDecisionSource::Default
        },
        user_writable: true,
        user_settings_value,
        env_max_thinking_tokens_set: false,
    }
}

/// Set the extended-thinking default in `~/.claude/settings.json`.
///
/// - `enabled = true` (CC default): removes `alwaysThinkingEnabled`,
///   returning to default-on-by-absence.
/// - `enabled = false`: writes `alwaysThinkingEnabled: false`.
///
/// Writes only the user layer. Does not guard against the env var: a
/// settings write is harmless (it takes effect once the env var is
/// unset) and the UI disables the toggle while the env var forces the
/// decision (`user_writable == false`).
pub fn set_thinking_enabled(enabled: bool) -> Result<(), SettingsWriteError> {
    let anchor = Path::new("");
    if enabled {
        clear_bool_setting(SettingsLayer::User, anchor, ALWAYS_THINKING_KEY)
    } else {
        write_bool_setting(SettingsLayer::User, anchor, ALWAYS_THINKING_KEY, false)
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
        std::env::remove_var(MAX_THINKING_TOKENS_ENV);
        (tmp, lock)
    }

    fn write_user_settings(body: &str) {
        let p = user_settings_path();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
    }

    fn read_user_settings() -> JsonValue {
        serde_json::from_slice(&fs::read(user_settings_path()).unwrap()).unwrap()
    }

    #[test]
    fn default_is_enabled_when_nothing_set() {
        let (_t, _l) = isolated();
        let s = resolve_thinking_enabled();
        assert!(s.effective);
        assert_eq!(s.decided_by, ThinkingDecisionSource::Default);
        assert!(s.user_writable);
        assert_eq!(s.user_settings_value, None);
    }

    #[test]
    fn user_false_disables() {
        let (_t, _l) = isolated();
        set_thinking_enabled(false).unwrap();
        let s = resolve_thinking_enabled();
        assert!(!s.effective);
        assert_eq!(s.decided_by, ThinkingDecisionSource::UserSettings);
        assert_eq!(s.user_settings_value, Some(false));
    }

    #[test]
    fn user_true_is_enabled_via_user_settings() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"alwaysThinkingEnabled":true}"#);
        let s = resolve_thinking_enabled();
        assert!(s.effective);
        assert_eq!(s.decided_by, ThinkingDecisionSource::UserSettings);
    }

    #[test]
    fn enable_clears_key_back_to_default_and_preserves_rest() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"alwaysThinkingEnabled":false,"keep":1}"#);
        set_thinking_enabled(true).unwrap();
        let v = read_user_settings();
        assert!(v.get("alwaysThinkingEnabled").is_none());
        assert_eq!(v["keep"], JsonValue::from(1));
        assert_eq!(
            resolve_thinking_enabled().decided_by,
            ThinkingDecisionSource::Default
        );
    }

    #[test]
    fn env_positive_forces_on_and_locks() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"alwaysThinkingEnabled":false}"#);
        std::env::set_var(MAX_THINKING_TOKENS_ENV, "8000");
        let s = resolve_thinking_enabled();
        assert!(s.effective); // env wins over the user's false
        assert_eq!(s.decided_by, ThinkingDecisionSource::EnvMaxThinkingTokens);
        assert!(!s.user_writable);
        assert!(s.env_max_thinking_tokens_set);
        std::env::remove_var(MAX_THINKING_TOKENS_ENV);
    }

    #[test]
    fn env_zero_forces_off_and_locks() {
        let (_t, _l) = isolated();
        std::env::set_var(MAX_THINKING_TOKENS_ENV, "0");
        let s = resolve_thinking_enabled();
        assert!(!s.effective);
        assert_eq!(s.decided_by, ThinkingDecisionSource::EnvMaxThinkingTokens);
        assert!(!s.user_writable);
        std::env::remove_var(MAX_THINKING_TOKENS_ENV);
    }

    #[test]
    fn env_empty_string_is_not_an_override() {
        let (_t, _l) = isolated();
        std::env::set_var(MAX_THINKING_TOKENS_ENV, "");
        let s = resolve_thinking_enabled();
        assert!(s.effective);
        assert_eq!(s.decided_by, ThinkingDecisionSource::Default);
        assert!(!s.env_max_thinking_tokens_set);
        std::env::remove_var(MAX_THINKING_TOKENS_ENV);
    }

    #[test]
    fn js_parse_int_matches_parseint_semantics() {
        assert_eq!(js_parse_int("8000"), Some(8000));
        assert_eq!(js_parse_int("  16000abc"), Some(16000));
        assert_eq!(js_parse_int("-1"), Some(-1));
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(js_parse_int(""), None);
        // Overflow saturates on sign (JS returns a huge finite, still > 0).
        let huge = "9".repeat(40);
        assert!(js_parse_int(&huge).is_some_and(|n| n > 0));
        assert!(js_parse_int(&format!("-{huge}")).is_some_and(|n| n < 0));
    }

    #[test]
    fn env_absurdly_large_still_forces_on() {
        let (_t, _l) = isolated();
        std::env::set_var(MAX_THINKING_TOKENS_ENV, &"9".repeat(30));
        let s = resolve_thinking_enabled();
        assert!(s.effective); // not disabled by i64 overflow
        assert_eq!(s.decided_by, ThinkingDecisionSource::EnvMaxThinkingTokens);
        std::env::remove_var(MAX_THINKING_TOKENS_ENV);
    }
}
