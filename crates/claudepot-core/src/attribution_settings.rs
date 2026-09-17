//! Read + write CC's commit/PR **attribution** — whether Claude's
//! "Co-Authored-By" / "Generated with Claude Code" text lands on
//! commits and PRs, and what it says.
//!
//! # CC's resolution model (read from the 2.1.274 binary)
//!
//! Commit text, and the PR text in the attribution note CC gives the
//! model:
//! ```text
//! if settings.attribution is an object:
//!     commit = attribution.commit ?? defaultCommit   // "" stays ""
//!     pr     = attribution.pr     ?? defaultPr
//! else if settings.includeCoAuthoredBy === false:
//!     commit = pr = ""
//! else:
//!     defaults
//! ```
//!
//! The PR body CC writes itself:
//! ```text
//! if settings.attribution?.pr !== undefined:   // "" now matches
//!     return attribution.pr
//! if settings.includeCoAuthoredBy === false:
//!     return ""
//! ... otherwise builds the default enhanced attribution ...
//! ```
//!
//! That second path used to test `attribution?.pr` for *truthiness*
//! (2.1.88), so an empty `pr` fell through and CC generated PR
//! attribution anyway unless `includeCoAuthoredBy === false` was also
//! set. 2.1.274 compares against `undefined` and needs no guard; the
//! changelog never announced the change, so the guard is still written
//! for the builds in between. It is harmless where it is not needed —
//! with an object present the commit path never reads it.
//!
//! The object is `.passthrough()` and carries more than the texts:
//! `sessionUrl: false` (CC 2.1.183) is what keeps the `Claude-Session:`
//! trailer and PR link off work from web and Remote Control sessions.
//! Every write here therefore edits `commit` and `pr` **inside** the
//! existing object rather than replacing it.
//!
//! # The three modes we write (one atomic `mutate_settings`)
//!
//! - **Default** → remove `includeCoAuthoredBy` and the two texts; the
//!   `attribution` object goes only if nothing else is left in it (CC
//!   uses its default trailer).
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
/// Classified from what CC would actually produce, per the model at the
/// top of this file: the commit text from the commit path, the PR text
/// from the path that writes PR bodies. Both suppressed → **Off**, both
/// CC's defaults → **Default**, anything else → **Custom**. So
/// `attribution:{}` is Default rather than Off (absent is not empty),
/// and the deprecated key counts only where CC still reads it.
pub fn resolve_attribution() -> AttributionState {
    let obj = read_user_settings_object();
    let include = obj
        .get(INCLUDE_CO_AUTHORED_BY_KEY)
        .and_then(JsonValue::as_bool);

    let attribution = obj.get(ATTRIBUTION_KEY).and_then(JsonValue::as_object);
    let commit = attribution.and_then(|a| string_field(a, "commit"));
    let pr = attribution.and_then(|a| string_field(a, "pr"));

    // What CC falls back to where a text is not set in the object.
    let legacy = if include == Some(false) {
        FieldState::Empty
    } else {
        FieldState::Absent
    };
    let commit_state = match attribution {
        Some(a) => field_state(a, "commit"),
        None => legacy,
    };
    let pr_state = match attribution.map(|a| field_state(a, "pr")) {
        Some(FieldState::Absent) | None => legacy,
        Some(state) => state,
    };
    let mode = match (commit_state, pr_state) {
        (FieldState::Empty, FieldState::Empty) => AttributionModeKind::Off,
        (FieldState::Absent, FieldState::Absent) => AttributionModeKind::Default,
        _ => AttributionModeKind::Custom,
    };

    AttributionState {
        mode,
        commit,
        pr,
        include_co_authored_by: include,
    }
}

/// Apply the attribution mode in a single atomic write to
/// `~/.claude/settings.json`. Preserves every unrelated key, including
/// the ones inside `attribution` other than the two texts.
pub fn set_attribution(mode: AttributionMode) -> Result<(), SettingsWriteError> {
    let anchor = Path::new("");
    // Borrow rather than move: `mutate_settings` re-runs its closure when an
    // external writer moves the file mid-edit, so it cannot consume `mode`.
    mutate_settings(SettingsLayer::User, anchor, move |map| match &mode {
        AttributionMode::Default => {
            clear_texts(map);
            map.remove(INCLUDE_CO_AUTHORED_BY_KEY);
        }
        AttributionMode::Off => {
            set_texts(map, "", "");
            map.insert(
                INCLUDE_CO_AUTHORED_BY_KEY.to_string(),
                JsonValue::Bool(false),
            );
        }
        AttributionMode::Custom { commit, pr } => {
            set_texts(map, commit, pr);
            if pr.is_empty() {
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

/// Set both texts inside the existing `attribution` object, creating it
/// if absent. A non-object value is CC-invalid and is replaced.
fn set_texts(map: &mut Map<String, JsonValue>, commit: &str, pr: &str) {
    let entry = map
        .entry(ATTRIBUTION_KEY.to_string())
        .or_insert_with(|| JsonValue::Object(Map::new()));
    if !entry.is_object() {
        *entry = JsonValue::Object(Map::new());
    }
    if let JsonValue::Object(o) = entry {
        o.insert("commit".to_string(), JsonValue::String(commit.to_string()));
        o.insert("pr".to_string(), JsonValue::String(pr.to_string()));
    }
}

/// Remove both texts; drop the object only when nothing else is in it.
fn clear_texts(map: &mut Map<String, JsonValue>) {
    let keep = match map.get_mut(ATTRIBUTION_KEY) {
        Some(JsonValue::Object(o)) => {
            o.remove("commit");
            o.remove("pr");
            !o.is_empty()
        }
        _ => false,
    };
    if !keep {
        map.remove(ATTRIBUTION_KEY);
    }
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

    /// `attribution` is a passthrough object and carries more than the
    /// two texts: `sessionUrl: false` is what keeps the `Claude-Session:`
    /// trailer off commits from web and Remote Control sessions. Writing
    /// the texts must not take it with them.
    #[test]
    fn custom_and_off_keep_the_rest_of_the_attribution_object() {
        let (_t, _l) = isolated();
        write_settings(r#"{"attribution":{"sessionUrl":false,"commit":"old","future":1}}"#);
        set_attribution(AttributionMode::Custom {
            commit: "c".to_string(),
            pr: "p".to_string(),
        })
        .unwrap();
        let v = read_settings();
        assert_eq!(v["attribution"]["sessionUrl"], JsonValue::Bool(false));
        assert_eq!(v["attribution"]["future"], JsonValue::from(1));
        assert_eq!(v["attribution"]["commit"], JsonValue::from("c"));
        assert_eq!(v["attribution"]["pr"], JsonValue::from("p"));

        set_attribution(AttributionMode::Off).unwrap();
        let v = read_settings();
        assert_eq!(v["attribution"]["sessionUrl"], JsonValue::Bool(false));
        assert_eq!(v["attribution"]["commit"], JsonValue::from(""));
    }

    #[test]
    fn default_drops_only_the_texts_when_the_object_holds_more() {
        let (_t, _l) = isolated();
        write_settings(
            r#"{"attribution":{"sessionUrl":false,"commit":"","pr":""},"includeCoAuthoredBy":false}"#,
        );
        set_attribution(AttributionMode::Default).unwrap();
        let v = read_settings();
        assert_eq!(
            v["attribution"],
            serde_json::json!({"sessionUrl": false}),
            "the session-link choice survives a reset of the texts"
        );
        assert!(v.get("includeCoAuthoredBy").is_none());
        assert_eq!(resolve_attribution().mode, AttributionModeKind::Default);
    }

    /// CC reads the deprecated key for a text the object leaves unset,
    /// but only on the PR path: with an object present, the commit text
    /// is `attribution.commit ?? default` regardless. So this file hides
    /// PR text and keeps the default commit trailer — neither Default
    /// nor Off.
    #[test]
    fn a_legacy_guard_beside_an_object_without_texts_is_custom() {
        let (_t, _l) = isolated();
        write_settings(r#"{"attribution":{"sessionUrl":false},"includeCoAuthoredBy":false}"#);
        assert_eq!(resolve_attribution().mode, AttributionModeKind::Custom);
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
