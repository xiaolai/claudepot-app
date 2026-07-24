//! Read + write CC's commit/PR **attribution** — whether Claude's
//! "Co-Authored-By" / "Generated with Claude Code" text lands on
//! commits and PRs, and what it says.
//!
//! # CC's resolution model (verified against
//! `~/github/claude_code_src/src/utils/attribution.ts`)
//!
//! Commit + non-enhanced PR text (`getAttributionTexts`, lines 82-97):
//! ```text
//! if settings.attribution:
//!     commit = attribution.commit ?? defaultCommit   // "" stays ""
//!     pr     = attribution.pr     ?? defaultPr
//! else if settings.includeCoAuthoredBy === false:
//!     commit = pr = ""
//! else:
//!     defaults
//! ```
//!
//! Enhanced PR body (`getEnhancedPRAttribution`, lines 316-327):
//! ```text
//! if settings.attribution?.pr:            // TRUTHY — "" does NOT match
//!     return attribution.pr
//! if settings.includeCoAuthoredBy === false:
//!     return ""
//! ... otherwise builds the default enhanced attribution ...
//! ```
//!
//! The enhanced path is the load-bearing subtlety: an empty
//! `attribution.pr` is falsy, so it *falls through* and CC would still
//! generate PR attribution — unless the deprecated `includeCoAuthoredBy
//! === false` guard is also present. So turning attribution fully off
//! requires **both** keys. `includeCoAuthoredBy` is deprecated but still
//! honored, and here it is a correctness guard, not legacy cruft.
//!
//! Empty `commit` needs no guard: `getAttributionTexts` uses nullish
//! coalescing, so `""` is preserved as-is.
//!
//! # The three modes we write (one atomic `mutate_settings`)
//!
//! - **Default** → remove both `attribution` and `includeCoAuthoredBy`
//!   (CC uses its default trailer).
//! - **Off** → `attribution = {commit:"", pr:""}` AND
//!   `includeCoAuthoredBy = false` (suppresses every path).
//! - **Custom{commit, pr}** → `attribution = {commit, pr}`; set
//!   `includeCoAuthoredBy = false` *iff* `pr` is empty (the enhanced-PR
//!   guard), else remove it.
//!
//! Both keys move in a single read-modify-write so a crash can't leave a
//! half-applied, mixed-semantics file.
//!
//! Global-only: writes `~/.claude/settings.json` (the user layer).

use crate::paths::claude_config_dir;
use crate::settings_writer::{mutate_settings, SettingsLayer, SettingsWriteError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use std::path::Path;

/// Setting keys in CC's `settings.json`.
pub const ATTRIBUTION_KEY: &str = "attribution";
pub const INCLUDE_CO_AUTHORED_BY_KEY: &str = "includeCoAuthoredBy";

/// The write intent. `Custom` carries the literal commit + PR strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributionMode {
    Default,
    Off,
    Custom { commit: String, pr: String },
}

/// The classified state for display, serialized snake_case:
/// `default` / `off` / `custom`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AttributionModeKind {
    Default,
    Off,
    Custom,
}

/// Aggregate state surfaced by the control.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttributionState {
    /// Which of the three states the current settings express.
    pub mode: AttributionModeKind,
    /// `attribution.commit` if present (so a Custom editor can prefill).
    pub commit: Option<String>,
    /// `attribution.pr` if present.
    pub pr: Option<String>,
    /// `includeCoAuthoredBy` if present.
    pub include_co_authored_by: Option<bool>,
}

/// Read the user settings object. A missing / empty / malformed /
/// non-object file degrades to an empty map (quiet, like the other
/// resolvers) — a later `set_attribution` on a corrupt file fails loudly
/// rather than clobbering it (see `mutate_settings`).
fn read_user_settings_object() -> Map<String, JsonValue> {
    let path = claude_config_dir().join("settings.json");
    match std::fs::read(&path) {
        Ok(bytes) if !bytes.is_empty() => match serde_json::from_slice::<JsonValue>(&bytes) {
            Ok(JsonValue::Object(m)) => m,
            _ => Map::new(),
        },
        _ => Map::new(),
    }
}

fn string_field(obj: &Map<String, JsonValue>, key: &str) -> Option<String> {
    obj.get(key).and_then(JsonValue::as_str).map(str::to_string)
}

/// Tri-state for one string field inside the `attribution` object,
/// mirroring how CC's `attribution.<field> ?? default` distinguishes an
/// ABSENT field (falls back to CC's default text) from a PRESENT empty
/// string (suppresses that text). Conflating the two — as an
/// `unwrap_or("")` would — misreads `{}` (all defaults) as fully off.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldState {
    /// Key missing, or present but not a string → CC uses its default.
    Absent,
    /// Present and `""` → suppressed.
    Empty,
    /// Present and non-empty → custom text.
    NonEmpty,
}

fn field_state(obj: &Map<String, JsonValue>, key: &str) -> FieldState {
    match obj.get(key).and_then(JsonValue::as_str) {
        None => FieldState::Absent,
        Some("") => FieldState::Empty,
        Some(_) => FieldState::NonEmpty,
    }
}

/// Resolve the current attribution state for display.
///
/// Classification (respecting CC's per-field `?? default` fallback):
/// - `attribution` absent, `includeCoAuthoredBy === false` → **Off**
///   (legacy suppression form).
/// - `attribution` absent otherwise → **Default**.
/// - `attribution` present, both `commit` and `pr` present-and-empty →
///   **Off** (both texts suppressed).
/// - `attribution` present, both fields absent (an empty `{}`) →
///   **Default** (CC fills both from its defaults).
/// - anything else (a non-empty field, or a mix of absent/empty) →
///   **Custom**.
pub fn resolve_attribution() -> AttributionState {
    let obj = read_user_settings_object();
    let include = obj
        .get(INCLUDE_CO_AUTHORED_BY_KEY)
        .and_then(JsonValue::as_bool);

    let attribution = obj.get(ATTRIBUTION_KEY).and_then(JsonValue::as_object);
    let commit = attribution.and_then(|a| string_field(a, "commit"));
    let pr = attribution.and_then(|a| string_field(a, "pr"));

    let mode = match attribution {
        None => {
            if include == Some(false) {
                AttributionModeKind::Off
            } else {
                AttributionModeKind::Default
            }
        }
        Some(a) => match (field_state(a, "commit"), field_state(a, "pr")) {
            (FieldState::Empty, FieldState::Empty) => AttributionModeKind::Off,
            (FieldState::Absent, FieldState::Absent) => AttributionModeKind::Default,
            _ => AttributionModeKind::Custom,
        },
    };

    AttributionState {
        mode,
        commit,
        pr,
        include_co_authored_by: include,
    }
}

/// Apply the attribution mode in a single atomic write to
/// `~/.claude/settings.json`. Preserves every unrelated key.
pub fn set_attribution(mode: AttributionMode) -> Result<(), SettingsWriteError> {
    let anchor = Path::new("");
    mutate_settings(SettingsLayer::User, anchor, move |map| match mode {
        AttributionMode::Default => {
            map.remove(ATTRIBUTION_KEY);
            map.remove(INCLUDE_CO_AUTHORED_BY_KEY);
        }
        AttributionMode::Off => {
            map.insert(ATTRIBUTION_KEY.to_string(), attribution_object("", ""));
            map.insert(
                INCLUDE_CO_AUTHORED_BY_KEY.to_string(),
                JsonValue::Bool(false),
            );
        }
        AttributionMode::Custom { commit, pr } => {
            let pr_empty = pr.is_empty();
            map.insert(
                ATTRIBUTION_KEY.to_string(),
                attribution_object(&commit, &pr),
            );
            if pr_empty {
                map.insert(
                    INCLUDE_CO_AUTHORED_BY_KEY.to_string(),
                    JsonValue::Bool(false),
                );
            } else {
                map.remove(INCLUDE_CO_AUTHORED_BY_KEY);
            }
        }
    })
}

fn attribution_object(commit: &str, pr: &str) -> JsonValue {
    let mut o = Map::new();
    o.insert("commit".to_string(), JsonValue::String(commit.to_string()));
    o.insert("pr".to_string(), JsonValue::String(pr.to_string()));
    JsonValue::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path().join("config-dir"));
        fs::create_dir_all(tmp.path().join("config-dir")).unwrap();
        (tmp, lock)
    }

    fn settings_path() -> std::path::PathBuf {
        claude_config_dir().join("settings.json")
    }

    fn write_settings(body: &str) {
        fs::write(settings_path(), body).unwrap();
    }

    fn read_settings() -> JsonValue {
        serde_json::from_slice(&fs::read(settings_path()).unwrap()).unwrap()
    }

    #[test]
    fn default_when_nothing_set() {
        let (_t, _l) = isolated();
        let s = resolve_attribution();
        assert_eq!(s.mode, AttributionModeKind::Default);
        assert_eq!(s.commit, None);
        assert_eq!(s.pr, None);
    }

    #[test]
    fn off_writes_both_keys_and_classifies_off() {
        let (_t, _l) = isolated();
        write_settings(r#"{"keep":1}"#);
        set_attribution(AttributionMode::Off).unwrap();

        let v = read_settings();
        assert_eq!(v["attribution"]["commit"], JsonValue::from(""));
        assert_eq!(v["attribution"]["pr"], JsonValue::from(""));
        assert_eq!(v["includeCoAuthoredBy"], JsonValue::Bool(false));
        assert_eq!(v["keep"], JsonValue::from(1)); // preserved

        let s = resolve_attribution();
        assert_eq!(s.mode, AttributionModeKind::Off);
    }

    #[test]
    fn custom_with_pr_removes_the_guard() {
        let (_t, _l) = isolated();
        // Start with the guard present to prove it's cleared.
        write_settings(r#"{"includeCoAuthoredBy":false}"#);
        set_attribution(AttributionMode::Custom {
            commit: "Co-Authored-By: Me <me@x>".to_string(),
            pr: "Generated with AI".to_string(),
        })
        .unwrap();

        let v = read_settings();
        assert_eq!(
            v["attribution"]["commit"],
            JsonValue::from("Co-Authored-By: Me <me@x>")
        );
        assert_eq!(v["attribution"]["pr"], JsonValue::from("Generated with AI"));
        assert!(v.get("includeCoAuthoredBy").is_none()); // guard removed (pr non-empty)

        assert_eq!(resolve_attribution().mode, AttributionModeKind::Custom);
    }

    #[test]
    fn custom_with_empty_pr_keeps_the_guard() {
        let (_t, _l) = isolated();
        set_attribution(AttributionMode::Custom {
            commit: "Co-Authored-By: Me <me@x>".to_string(),
            pr: String::new(),
        })
        .unwrap();

        let v = read_settings();
        assert_eq!(v["attribution"]["pr"], JsonValue::from(""));
        // Empty pr → enhanced-PR path needs the deprecated guard.
        assert_eq!(v["includeCoAuthoredBy"], JsonValue::Bool(false));
        // commit is non-empty → still classified Custom, not Off.
        assert_eq!(resolve_attribution().mode, AttributionModeKind::Custom);
    }

    #[test]
    fn default_removes_both_keys_and_preserves_rest() {
        let (_t, _l) = isolated();
        write_settings(
            r#"{"attribution":{"commit":"","pr":""},"includeCoAuthoredBy":false,"keep":2}"#,
        );
        set_attribution(AttributionMode::Default).unwrap();

        let v = read_settings();
        assert!(v.get("attribution").is_none());
        assert!(v.get("includeCoAuthoredBy").is_none());
        assert_eq!(v["keep"], JsonValue::from(2));
        assert_eq!(resolve_attribution().mode, AttributionModeKind::Default);
    }

    #[test]
    fn legacy_include_false_alone_classifies_off() {
        let (_t, _l) = isolated();
        write_settings(r#"{"includeCoAuthoredBy":false}"#);
        let s = resolve_attribution();
        assert_eq!(s.mode, AttributionModeKind::Off);
        assert_eq!(s.include_co_authored_by, Some(false));
    }

    #[test]
    fn empty_attribution_object_classifies_default_not_off() {
        // `attribution:{}` → CC fills both fields from its defaults, so it
        // reads as Default, NOT Off (both fields absent, not present-empty).
        let (_t, _l) = isolated();
        write_settings(r#"{"attribution":{}}"#);
        assert_eq!(resolve_attribution().mode, AttributionModeKind::Default);
    }

    #[test]
    fn present_empty_commit_with_absent_pr_classifies_custom() {
        // commit present-and-empty (suppressed) but pr ABSENT (CC default)
        // → a mix, not fully off → Custom.
        let (_t, _l) = isolated();
        write_settings(r#"{"attribution":{"commit":""}}"#);
        assert_eq!(resolve_attribution().mode, AttributionModeKind::Custom);
    }

    #[test]
    fn default_write_on_missing_file_is_noop() {
        let (_t, _l) = isolated();
        set_attribution(AttributionMode::Default).unwrap();
        // No empty settings file created.
        assert!(!settings_path().exists());
    }
}
