//! Read + write CC's `enableArtifact` / `disableArtifact` settings —
//! the "keep companion output local" toggle.
//!
//! Claude Code's **Artifact** tool publishes an HTML/Markdown file as
//! a private page in the signed-in account's cloud gallery
//! (`claude.ai/code/artifacts`). Because the gallery is per-account,
//! switching accounts orphans previously-published artifacts. Turning
//! the tool off makes CC keep that companion output as a local file
//! instead — the fix for a multi-account setup.
//!
//! # CC's resolution model (verified against the shipped 2.1.218 binary)
//!
//! ```text
//! Oas()  = env CLAUDE_CODE_DISABLE_ARTIFACT (raw, any non-empty value)
//!          OR settings.disableArtifact === true            → hard OFF
//! enabled = if Oas()               → false
//!           else <availability gate, server-side>
//!           else (enableArtifact ?? true)                  // default: enabled
//! ```
//!
//! `enableArtifact` is resolved from `["policySettings",
//! "flagSettings", "userSettings"]` only — there is **no project /
//! localProject layer**. So this toggle is *global-only* and writes
//! exclusively to `~/.claude/settings.json`.
//!
//! `enableArtifact` is the user-scoped toggle CC's own `/config` UI
//! writes (clearing the key when it equals the default). `disableArtifact`
//! and the env var are a *hard override* that wins over `enableArtifact`.
//! Claudepot therefore writes `enableArtifact` (staying consistent with
//! CC's own UI) and treats `disableArtifact` / the env var as detected
//! overrides.
//!
//! Two CC behaviors we deliberately do **not** model:
//! - The env var uses raw JS truthiness, not `isEnvTruthy` — any
//!   non-empty string (even `"0"`) disables. We match that.
//! - The server-side availability gate (a feature flag + account
//!   entitlement) can force the tool off regardless of settings.
//!   Claudepot can't observe it; a settings toggle controls the
//!   settings-level decision, so `enabled` reflects env + settings only.
//! - `policySettings` / `flagSettings` layers can also override
//!   `enableArtifact`; those are out of scope here exactly as in
//!   `settings_writer` (auto-memory).

use crate::paths::claude_config_dir;
use crate::settings_writer::{
    clear_bool_setting, read_bool_setting, write_bool_setting, SettingsLayer, SettingsWriteError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The user-toggle key CC's own settings UI writes.
pub const ENABLE_ARTIFACT_KEY: &str = "enableArtifact";
/// The hard-override key (also settable via the env var below).
pub const DISABLE_ARTIFACT_KEY: &str = "disableArtifact";
/// Env var that hard-disables the Artifact tool (raw truthiness).
pub const DISABLE_ARTIFACT_ENV: &str = "CLAUDE_CODE_DISABLE_ARTIFACT";

/// What decided the effective enablement.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactDecisionSource {
    /// `CLAUDE_CODE_DISABLE_ARTIFACT` is set (any non-empty value).
    EnvDisable,
    /// `~/.claude/settings.json :: disableArtifact === true`.
    DisableSetting,
    /// `~/.claude/settings.json :: enableArtifact` (true or false).
    EnableSetting,
    /// No source set a value — CC's built-in default (enabled) wins.
    Default,
}

/// Aggregate state surfaced by the toggle UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactState {
    /// Whether CC's Artifact (cloud-publish) tool is enabled at the
    /// env + settings level. `false` means companion output stays
    /// local. Does not account for CC's server-side availability gate.
    pub enabled: bool,
    /// Why `enabled` is what it is.
    pub decided_by: ArtifactDecisionSource,
    /// `false` when the env var is forcing the decision — the toggle
    /// renders disabled with the reason shown. A user-level
    /// `disableArtifact` is still writable (Claudepot can clear it).
    pub user_writable: bool,
    /// `~/.claude/settings.json :: enableArtifact`, if present.
    pub user_enable_value: Option<bool>,
    /// `~/.claude/settings.json :: disableArtifact`, if present.
    pub user_disable_value: Option<bool>,
    /// Whether `CLAUDE_CODE_DISABLE_ARTIFACT` is set (non-empty).
    pub env_disable_set: bool,
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Resolve the artifact-tool enablement from the env var + user
/// settings. Pure read over env + filesystem — no side effects.
pub fn resolve_artifact_enabled() -> ArtifactState {
    // CC reads the env var with raw JS truthiness (`Z.CLAUDE_CODE_DISABLE_ARTIFACT || …`),
    // NOT `isEnvTruthy`: any non-empty value disables, even "0" / "false".
    // `var_os` (not `var`) so a non-UTF8 value still counts — JS sees a
    // non-empty string there too; only unset or empty is falsy.
    let env_disable_set = std::env::var_os(DISABLE_ARTIFACT_ENV).is_some_and(|s| !s.is_empty());

    // A malformed settings.json degrades to `None` on read (quiet, like the
    // auto-memory resolver), so the decision falls through to CC's default.
    // A later `set_artifact_enabled` on the same corrupt file fails loudly
    // rather than clobbering it (see `rmw_settings_bool`).
    let path = user_settings_path();
    let user_enable_value = read_bool_setting(&path, ENABLE_ARTIFACT_KEY).unwrap_or(None);
    let user_disable_value = read_bool_setting(&path, DISABLE_ARTIFACT_KEY).unwrap_or(None);

    let base = ArtifactState {
        enabled: true,
        decided_by: ArtifactDecisionSource::Default,
        user_writable: true,
        user_enable_value,
        user_disable_value,
        env_disable_set,
    };

    if env_disable_set {
        return ArtifactState {
            enabled: false,
            decided_by: ArtifactDecisionSource::EnvDisable,
            user_writable: false,
            ..base
        };
    }
    // Hard override — `disableArtifact === true` wins over enableArtifact.
    if user_disable_value == Some(true) {
        return ArtifactState {
            enabled: false,
            decided_by: ArtifactDecisionSource::DisableSetting,
            ..base
        };
    }
    if let Some(v) = user_enable_value {
        return ArtifactState {
            enabled: v,
            decided_by: ArtifactDecisionSource::EnableSetting,
            ..base
        };
    }
    base
}

/// Set whether CC's Artifact tool is enabled, in `~/.claude/settings.json`.
///
/// - `enabled = false` (keep companion output local): writes
///   `enableArtifact: false` — the same field CC's own settings UI
///   toggles.
/// - `enabled = true` (default / cloud publishing): removes both
///   `enableArtifact` and any `disableArtifact: true`, returning the
///   feature to CC's default (enabled). Clearing `disableArtifact` is
///   required — it hard-overrides `enableArtifact`, so leaving it set
///   would make the toggle appear stuck off.
///
/// Writes only the user layer. Does not guard against the env var: a
/// settings write is harmless (it just takes effect once the env var
/// is unset), and the UI already disables the toggle when the env var
/// is forcing the decision (`user_writable == false`).
pub fn set_artifact_enabled(enabled: bool) -> Result<(), SettingsWriteError> {
    // `SettingsLayer::User` resolves to `~/.claude/settings.json` and
    // ignores the project_root argument entirely.
    let anchor = Path::new("");
    if enabled {
        clear_bool_setting(SettingsLayer::User, anchor, ENABLE_ARTIFACT_KEY)?;
        clear_bool_setting(SettingsLayer::User, anchor, DISABLE_ARTIFACT_KEY)?;
    } else {
        write_bool_setting(SettingsLayer::User, anchor, ENABLE_ARTIFACT_KEY, false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;
    use std::fs;
    use tempfile::TempDir;

    /// Isolate `CLAUDE_CONFIG_DIR` + clear the env override. Holds the
    /// shared data-dir lock so parallel tests don't race the env vars.
    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path().join("config-dir"));
        fs::create_dir_all(tmp.path().join("config-dir")).unwrap();
        std::env::remove_var(DISABLE_ARTIFACT_ENV);
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
    fn default_is_enabled() {
        let (_t, _l) = isolated();
        let s = resolve_artifact_enabled();
        assert!(s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::Default);
        assert!(s.user_writable);
        assert_eq!(s.user_enable_value, None);
        assert_eq!(s.user_disable_value, None);
    }

    #[test]
    fn env_disable_any_nonempty_value_wins_and_locks() {
        let (_t, _l) = isolated();
        // Raw truthiness: even "0" disables (matches CC, not isEnvTruthy).
        std::env::set_var(DISABLE_ARTIFACT_ENV, "0");
        // A user enableArtifact:true must NOT win over the env var.
        write_user_settings(r#"{"enableArtifact":true}"#);
        let s = resolve_artifact_enabled();
        assert!(!s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::EnvDisable);
        assert!(!s.user_writable);
        assert!(s.env_disable_set);
        std::env::remove_var(DISABLE_ARTIFACT_ENV);
    }

    #[test]
    fn empty_env_value_does_not_disable() {
        let (_t, _l) = isolated();
        std::env::set_var(DISABLE_ARTIFACT_ENV, "");
        let s = resolve_artifact_enabled();
        assert!(s.enabled);
        assert!(!s.env_disable_set);
        assert_eq!(s.decided_by, ArtifactDecisionSource::Default);
        std::env::remove_var(DISABLE_ARTIFACT_ENV);
    }

    #[test]
    fn disable_setting_false_falls_through_to_default() {
        let (_t, _l) = isolated();
        // `disableArtifact:false` is NOT a hard-off (CC checks `=== true`);
        // with no enableArtifact it must fall through to the default.
        write_user_settings(r#"{"disableArtifact":false}"#);
        let s = resolve_artifact_enabled();
        assert!(s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::Default);
        assert_eq!(s.user_disable_value, Some(false));
    }

    #[test]
    fn enable_setting_true_alone_is_enable_setting_not_default() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"enableArtifact":true}"#);
        let s = resolve_artifact_enabled();
        assert!(s.enabled);
        // Distinct discriminant from Default — the key is explicitly set.
        assert_eq!(s.decided_by, ArtifactDecisionSource::EnableSetting);
        assert_eq!(s.user_enable_value, Some(true));
    }

    #[test]
    fn disable_setting_true_alone_disables() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"disableArtifact":true}"#);
        let s = resolve_artifact_enabled();
        assert!(!s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::DisableSetting);
        assert!(s.user_writable);
    }

    #[test]
    fn disable_setting_overrides_enable_setting() {
        let (_t, _l) = isolated();
        // Both present — disableArtifact:true is the hard override.
        write_user_settings(r#"{"enableArtifact":true,"disableArtifact":true}"#);
        let s = resolve_artifact_enabled();
        assert!(!s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::DisableSetting);
        // A user-level disableArtifact is still clearable from here.
        assert!(s.user_writable);
    }

    #[test]
    fn enable_setting_false_keeps_output_local() {
        let (_t, _l) = isolated();
        set_artifact_enabled(false).unwrap();
        let s = resolve_artifact_enabled();
        assert!(!s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::EnableSetting);
        assert_eq!(s.user_enable_value, Some(false));
        // We write the soft toggle, never the hard switch.
        assert_eq!(s.user_disable_value, None);
    }

    #[test]
    fn enable_clears_both_keys_back_to_default_and_preserves_rest() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"enableArtifact":false,"disableArtifact":true,"keep":1}"#);
        set_artifact_enabled(true).unwrap();

        let s = resolve_artifact_enabled();
        assert!(s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::Default);

        let v = read_user_settings();
        assert!(v.get("enableArtifact").is_none());
        assert!(v.get("disableArtifact").is_none());
        assert_eq!(v["keep"], JsonValue::from(1));
    }

    #[test]
    fn set_local_preserves_unknown_keys_and_creates_file() {
        let (_t, _l) = isolated();
        // No settings file exists yet — the write must create it.
        set_artifact_enabled(false).unwrap();
        let v = read_user_settings();
        assert_eq!(v["enableArtifact"], JsonValue::from(false));

        // And a second write over an existing file preserves siblings.
        write_user_settings(r#"{"unrelated":42,"enableArtifact":false}"#);
        set_artifact_enabled(false).unwrap();
        let v = read_user_settings();
        assert_eq!(v["unrelated"], JsonValue::from(42));
        assert_eq!(v["enableArtifact"], JsonValue::from(false));
    }

    #[test]
    fn round_trip_local_then_default() {
        let (_t, _l) = isolated();
        set_artifact_enabled(false).unwrap();
        assert!(!resolve_artifact_enabled().enabled);
        set_artifact_enabled(true).unwrap();
        let s = resolve_artifact_enabled();
        assert!(s.enabled);
        assert_eq!(s.decided_by, ArtifactDecisionSource::Default);
    }
}
