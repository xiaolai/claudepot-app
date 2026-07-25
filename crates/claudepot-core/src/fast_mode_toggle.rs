//! Read + write CC's `fastMode` setting — the "Fast mode" toggle.
//!
//! Fast mode runs Claude Opus at a higher-speed API configuration:
//! same model, same quality, up to ~2.5× faster, at **$10/$50 per
//! MTok instead of $5/$25**. On subscription plans it bills to usage
//! credits rather than the plan's included usage, so it is the one CC
//! behavior toggle that spends money directly. The UI says so.
//!
//! # CC's resolution model
//!
//! Verified against CC's published fast-mode reference
//! (`code.claude.com/docs/en/fast-mode`) rather than the source
//! checkout, which predates the feature.
//!
//! ```text
//! if CLAUDE_CODE_DISABLE_FAST_MODE (set, non-empty):
//!     enabled = false                              // hard override
//! else if settings.fastMode === true:
//!     enabled = true
//! else:
//!     enabled = false                              // default: off
//! ```
//!
//! The default is **off**, represented by absence — the mirror image
//! of `alwaysThinkingEnabled`, whose default is on. So we write the
//! key only to turn fast mode ON and clear it to turn it off, which is
//! the same "don't freeze a default CC represents by absence" rule
//! [`crate::thinking_toggle`] follows, pointed the other way.
//!
//! # Per-session opt-in
//!
//! `fastModePerSessionOptIn: true` makes every new session start with
//! fast mode off regardless of the saved preference; the user re-arms
//! it with `/fast`. It is a separate key with a separate default (off)
//! and is surfaced as its own switch, because the two answer different
//! questions: *do I want fast mode* versus *do I want it to persist*.
//!
//! # What this toggle does not know
//!
//! Fast mode also requires usage credits to be enabled on the account,
//! and Team/Enterprise owners must turn it on org-wide. Neither is
//! visible from disk, so a toggle that reads "on" here can still be
//! refused at runtime by CC. The UI states the requirement rather than
//! pretending to verify it.
//!
//! Global-only, matching the other CC behavior toggles: this is the
//! per-user default for new sessions.

use crate::paths::claude_config_dir;
use crate::settings_writer::{
    clear_bool_setting, read_bool_setting, write_bool_setting, SettingsLayer, SettingsWriteError,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Setting key in CC's `settings.json`.
pub const FAST_MODE_KEY: &str = "fastMode";
/// Setting key for "start every session with fast mode off".
pub const FAST_MODE_PER_SESSION_KEY: &str = "fastModePerSessionOptIn";
/// Env var that hard-disables fast mode regardless of settings.
pub const DISABLE_FAST_MODE_ENV: &str = "CLAUDE_CODE_DISABLE_FAST_MODE";

/// Models fast mode runs on. Opus 4.7 was removed on 2026-07-24: CC
/// still *treats* it as a fast-mode model, but the API rejects the
/// resulting requests, so it is deliberately not listed here.
pub const FAST_MODE_MODELS: &[&str] = &["claude-opus-5", "claude-opus-4-8"];

/// Fast-mode input rate, USD per million tokens.
pub const FAST_MODE_INPUT_PER_MTOK: f64 = 10.0;
/// Fast-mode output rate, USD per million tokens.
pub const FAST_MODE_OUTPUT_PER_MTOK: f64 = 50.0;

/// What decided the effective enablement.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FastModeDecisionSource {
    /// `CLAUDE_CODE_DISABLE_FAST_MODE` is set — fast mode is off and
    /// the toggle is read-only.
    EnvDisabled,
    /// `~/.claude/settings.json :: fastMode`.
    UserSettings,
    /// No source set the key — CC's built-in default (off) wins.
    Default,
}

/// Aggregate state surfaced by the toggle UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FastModeState {
    /// Whether fast mode is on by default for new sessions.
    pub effective: bool,
    /// Why `effective` is what it is.
    pub decided_by: FastModeDecisionSource,
    /// `false` when the env var is forcing the decision — the toggle
    /// renders disabled with the reason shown inline.
    pub user_writable: bool,
    /// `~/.claude/settings.json :: fastMode`, if present.
    pub user_settings_value: Option<bool>,
    /// `~/.claude/settings.json :: fastModePerSessionOptIn`. When true,
    /// every session starts with fast mode off no matter what
    /// `effective` says.
    pub per_session_opt_in: bool,
    /// Whether `CLAUDE_CODE_DISABLE_FAST_MODE` is set (non-empty).
    pub env_disabled: bool,
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Resolve the fast-mode default from env + user settings. Pure read
/// over env + filesystem — no side effects.
pub fn resolve_fast_mode() -> FastModeState {
    let env_disabled = std::env::var(DISABLE_FAST_MODE_ENV)
        .ok()
        .is_some_and(|s| !s.is_empty());
    let path = user_settings_path();
    let user_settings_value = read_bool_setting(&path, FAST_MODE_KEY).unwrap_or(None);
    let per_session_opt_in =
        read_bool_setting(&path, FAST_MODE_PER_SESSION_KEY).unwrap_or(None) == Some(true);

    if env_disabled {
        return FastModeState {
            effective: false,
            decided_by: FastModeDecisionSource::EnvDisabled,
            user_writable: false,
            user_settings_value,
            per_session_opt_in,
            env_disabled: true,
        };
    }

    // Only `=== true` enables; `false` and absent both mean off.
    let effective = user_settings_value == Some(true);
    FastModeState {
        effective,
        decided_by: if user_settings_value.is_some() {
            FastModeDecisionSource::UserSettings
        } else {
            FastModeDecisionSource::Default
        },
        user_writable: true,
        user_settings_value,
        per_session_opt_in,
        env_disabled: false,
    }
}

/// Set the fast-mode default in `~/.claude/settings.json`.
///
/// - `enabled = true`: writes `fastMode: true`.
/// - `enabled = false` (CC default): removes the key, returning to
///   default-off-by-absence rather than freezing a `false`.
///
/// Writes only the user layer. Does not guard against the env var: the
/// write is harmless (it takes effect once the var is unset) and the
/// UI disables the toggle while the var forces the decision
/// (`user_writable == false`).
pub fn set_fast_mode(enabled: bool) -> Result<(), SettingsWriteError> {
    let anchor = std::path::Path::new("");
    if enabled {
        write_bool_setting(SettingsLayer::User, anchor, FAST_MODE_KEY, true)
    } else {
        clear_bool_setting(SettingsLayer::User, anchor, FAST_MODE_KEY)
    }
}

/// Set `fastModePerSessionOptIn`. `false` clears the key (the default)
/// rather than writing an explicit `false`.
pub fn set_per_session_opt_in(required: bool) -> Result<(), SettingsWriteError> {
    let anchor = std::path::Path::new("");
    if required {
        write_bool_setting(SettingsLayer::User, anchor, FAST_MODE_PER_SESSION_KEY, true)
    } else {
        clear_bool_setting(SettingsLayer::User, anchor, FAST_MODE_PER_SESSION_KEY)
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
        std::env::remove_var(DISABLE_FAST_MODE_ENV);
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
    fn default_is_off_when_nothing_set() {
        let (_t, _l) = isolated();
        let s = resolve_fast_mode();
        assert!(!s.effective);
        assert_eq!(s.decided_by, FastModeDecisionSource::Default);
        assert!(s.user_writable);
        assert_eq!(s.user_settings_value, None);
        assert!(!s.per_session_opt_in);
    }

    #[test]
    fn explicit_true_enables() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"fastMode":true}"#);
        let s = resolve_fast_mode();
        assert!(s.effective);
        assert_eq!(s.decided_by, FastModeDecisionSource::UserSettings);
    }

    #[test]
    fn explicit_false_disables_and_is_attributed_to_settings() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"fastMode":false}"#);
        let s = resolve_fast_mode();
        assert!(!s.effective);
        assert_eq!(s.decided_by, FastModeDecisionSource::UserSettings);
        assert_eq!(s.user_settings_value, Some(false));
    }

    #[test]
    fn env_var_forces_off_and_locks_the_toggle() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"fastMode":true}"#);
        std::env::set_var(DISABLE_FAST_MODE_ENV, "1");
        let s = resolve_fast_mode();
        assert!(!s.effective, "env var must win over the setting");
        assert_eq!(s.decided_by, FastModeDecisionSource::EnvDisabled);
        assert!(!s.user_writable);
        assert!(s.env_disabled);
        // The user's saved preference is still reported so the UI can
        // say "your setting is on, but the environment disables it".
        assert_eq!(s.user_settings_value, Some(true));
        std::env::remove_var(DISABLE_FAST_MODE_ENV);
    }

    #[test]
    fn an_empty_env_var_counts_as_unset() {
        // Mirrors CC's JS truthiness check, where "" is falsy.
        let (_t, _l) = isolated();
        write_user_settings(r#"{"fastMode":true}"#);
        std::env::set_var(DISABLE_FAST_MODE_ENV, "");
        let s = resolve_fast_mode();
        assert!(s.effective);
        assert!(!s.env_disabled);
        std::env::remove_var(DISABLE_FAST_MODE_ENV);
    }

    #[test]
    fn per_session_opt_in_is_read_independently() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"fastMode":true,"fastModePerSessionOptIn":true}"#);
        let s = resolve_fast_mode();
        assert!(s.effective);
        assert!(s.per_session_opt_in);
    }

    #[test]
    fn enabling_writes_the_key() {
        let (_t, _l) = isolated();
        set_fast_mode(true).unwrap();
        assert_eq!(read_user_settings()["fastMode"], JsonValue::Bool(true));
    }

    #[test]
    fn disabling_clears_the_key_rather_than_writing_false() {
        // The default is off-by-absence. Writing an explicit `false`
        // would freeze a default CC represents by omission.
        let (_t, _l) = isolated();
        write_user_settings(r#"{"fastMode":true,"keep":1}"#);
        set_fast_mode(false).unwrap();
        let v = read_user_settings();
        assert!(v.get("fastMode").is_none(), "key should be removed");
        assert_eq!(v["keep"], JsonValue::from(1), "siblings preserved");
    }

    #[test]
    fn per_session_opt_in_round_trips() {
        let (_t, _l) = isolated();
        set_per_session_opt_in(true).unwrap();
        assert_eq!(
            read_user_settings()["fastModePerSessionOptIn"],
            JsonValue::Bool(true)
        );
        assert!(resolve_fast_mode().per_session_opt_in);

        set_per_session_opt_in(false).unwrap();
        assert!(read_user_settings()
            .get("fastModePerSessionOptIn")
            .is_none());
        assert!(!resolve_fast_mode().per_session_opt_in);
    }

    #[test]
    fn writing_preserves_unrelated_settings() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"theme":"dark","alwaysThinkingEnabled":false}"#);
        set_fast_mode(true).unwrap();
        let v = read_user_settings();
        assert_eq!(v["theme"], JsonValue::from("dark"));
        assert_eq!(v["alwaysThinkingEnabled"], JsonValue::Bool(false));
        assert_eq!(v["fastMode"], JsonValue::Bool(true));
    }

    #[test]
    fn the_fast_mode_model_list_is_priced_by_the_rate_table() {
        // The UI names these models as the ones fast mode runs on. If
        // one falls out of the rate table the copy is stale.
        for id in FAST_MODE_MODELS {
            assert!(
                crate::session_live::pricing::periods_for_id(id).is_some(),
                "{id} is advertised for fast mode but isn't priced"
            );
        }
    }

    #[test]
    fn fast_mode_costs_more_than_the_standard_opus_rate() {
        // Guards the toggle's cost warning: if standard Opus ever rose
        // to meet the fast-mode rate the warning would be wrong.
        let standard = crate::session_live::pricing::rates_for("claude-opus-5").unwrap();
        assert!(FAST_MODE_INPUT_PER_MTOK > standard.input_per_million_usd);
        assert!(FAST_MODE_OUTPUT_PER_MTOK > standard.output_per_million_usd);
    }
}
