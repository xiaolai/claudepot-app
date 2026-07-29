//! Read + write CC's `availableModels` allowlist and its companion
//! `enforceAvailableModels` flag.
//!
//! # What the allowlist does
//!
//! `availableModels` restricts which models a user can select — in the
//! `/model` picker, via `--model`, and in fallback chains. Entries
//! match at three granularities:
//!
//! - **family alias** — `sonnet`, `opus`, `haiku`
//! - **version prefix** — `claude-sonnet-4-5`
//! - **full model id** — `claude-sonnet-4-5-20250929`, or a
//!   provider-form id like `us.anthropic.claude-opus-4-8`
//!
//! A `[1m]` suffix is stripped from both sides before matching.
//!
//! # The Default-model hole
//!
//! `availableModels` on its own does **not** constrain the picker's
//! "Default" option, which resolves to whatever the account's runtime
//! default is — so a user can bypass the whole list by choosing
//! Default. `enforceAvailableModels: true` closes that hole by
//! remapping Default to the first allowed entry. It requires CC
//! v2.1.175 or later, and it does nothing when the list is empty:
//! with `availableModels: []`, Default stays usable no matter what,
//! which is deliberate on CC's side so a bad config can't lock a user
//! out of every model.
//!
//! [`AvailableModelsState::enforce_is_effective`] reports whether the
//! flag is actually doing anything, so the UI can say "set but inert"
//! rather than implying a restriction that isn't in force.
//!
//! # Scope
//!
//! User layer only, like the other CC behavior surfaces here. CC also
//! reads this key from managed/policy settings, where an admin-deployed
//! list *replaces* rather than merges with the user's — Claudepot
//! neither writes nor overrides those, so a managed list can make this
//! editor's contents inert. That is CC's precedence, not something to
//! paper over.

use crate::paths::claude_config_dir;
use crate::settings_writer::{mutate_settings, SettingsLayer, SettingsWriteError};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::path::PathBuf;

/// Setting key holding the allowlist.
pub const AVAILABLE_MODELS_KEY: &str = "availableModels";
/// Setting key extending the allowlist to the Default option.
pub const ENFORCE_AVAILABLE_MODELS_KEY: &str = "enforceAvailableModels";

/// Minimum CC version that honors `enforceAvailableModels`.
pub const ENFORCE_MIN_CC_VERSION: &str = "2.1.175";

/// Aggregate state surfaced by the allowlist editor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AvailableModelsState {
    /// The allowlist, in file order. Empty means "no restriction".
    pub entries: Vec<String>,
    /// `enforceAvailableModels` as written. `None` when absent.
    pub enforce: Option<bool>,
    /// True when the key exists at all — distinguishes "no allowlist"
    /// from "an explicitly empty allowlist", which read the same to CC
    /// but mean different things to a user looking at the editor.
    pub key_present: bool,
}

impl AvailableModelsState {
    /// Whether any restriction is actually in force.
    pub fn restricts_models(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Whether `enforceAvailableModels` is doing anything. CC ignores
    /// it when the list is empty, so a `true` with no entries is inert
    /// and the UI must not imply otherwise.
    pub fn enforce_is_effective(&self) -> bool {
        self.enforce == Some(true) && self.restricts_models()
    }
}

/// Rejections from [`normalize_entries`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AllowlistError {
    #[error("entry {0} is empty")]
    EmptyEntry(usize),
    #[error("entry {0} contains whitespace: {1:?}")]
    ContainsWhitespace(usize, String),
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Clean a user-supplied allowlist: trim each entry, reject empties and
/// interior whitespace, and drop later duplicates while keeping first
/// occurrence order.
///
/// Order matters beyond tidiness: with `enforceAvailableModels`, the
/// Default option resolves to the **first** allowed entry, so
/// re-ordering the list changes which model Default becomes. Sorting
/// the list would silently change behavior, so we don't.
pub fn normalize_entries(raw: &[String]) -> Result<Vec<String>, AllowlistError> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for (i, entry) in raw.iter().enumerate() {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            return Err(AllowlistError::EmptyEntry(i));
        }
        if trimmed.chars().any(char::is_whitespace) {
            return Err(AllowlistError::ContainsWhitespace(i, trimmed.to_string()));
        }
        if !out.iter().any(|e| e == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

/// Read the allowlist from `~/.claude/settings.json`. A malformed file
/// or a non-array value reads as "no allowlist" rather than erroring —
/// the editor then shows an empty list, and the first save rewrites the
/// key cleanly. Non-string array members are skipped.
pub fn resolve_available_models() -> AvailableModelsState {
    let Ok(bytes) = std::fs::read(user_settings_path()) else {
        return AvailableModelsState::default();
    };
    let Ok(JsonValue::Object(map)) = serde_json::from_slice::<JsonValue>(&bytes) else {
        return AvailableModelsState::default();
    };
    let raw = map.get(AVAILABLE_MODELS_KEY);
    let entries = match raw {
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    AvailableModelsState {
        entries,
        enforce: map
            .get(ENFORCE_AVAILABLE_MODELS_KEY)
            .and_then(JsonValue::as_bool),
        key_present: matches!(raw, Some(JsonValue::Array(_))),
    }
}

/// Write the allowlist and the enforce flag in one atomic settings
/// write, so a crash can't leave `enforceAvailableModels: true` beside
/// a list that no longer justifies it.
///
/// - An empty `entries` removes `availableModels` entirely rather than
///   writing `[]`. The two behave the same in CC, but absence is what
///   "no restriction" looks like in a hand-edited file, and it avoids
///   leaving a key whose only effect is to confuse the next reader.
/// - `enforce = false` likewise clears the key rather than writing an
///   explicit `false` — CC's default.
/// - `enforce = true` with an *empty* list also clears the key. CC
///   ignores enforcement without entries, so writing it would leave a
///   file that claims a restriction which isn't in force.
pub fn set_available_models(
    entries: &[String],
    enforce: bool,
) -> Result<Vec<String>, SetAllowlistError> {
    let normalized = normalize_entries(entries)?;
    let list_is_empty = normalized.is_empty();
    let to_write = normalized.clone();
    mutate_settings(SettingsLayer::User, std::path::Path::new(""), move |map| {
        if to_write.is_empty() {
            map.remove(AVAILABLE_MODELS_KEY);
        } else {
            // Cloned per call, not consumed: `mutate_settings` re-runs its
            // closure when an external writer moves the file mid-edit.
            map.insert(
                AVAILABLE_MODELS_KEY.to_string(),
                JsonValue::Array(
                    to_write
                        .iter()
                        .cloned()
                        .map(JsonValue::String)
                        .collect::<Vec<_>>(),
                ),
            );
        }
        // Only write enforce alongside a list that gives it something
        // to enforce. CC ignores the flag when the allowlist is empty,
        // so persisting `true` there leaves a key that reads as an
        // active restriction and isn't one.
        if enforce && !list_is_empty {
            map.insert(
                ENFORCE_AVAILABLE_MODELS_KEY.to_string(),
                JsonValue::Bool(true),
            );
        } else {
            map.remove(ENFORCE_AVAILABLE_MODELS_KEY);
        }
    })?;
    Ok(normalized)
}

#[derive(Debug, thiserror::Error)]
pub enum SetAllowlistError {
    #[error(transparent)]
    Allowlist(#[from] AllowlistError),
    #[error(transparent)]
    Write(#[from] SettingsWriteError),
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

    fn write_user_settings(body: &str) {
        let p = user_settings_path();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
    }

    fn read_user_settings() -> JsonValue {
        serde_json::from_slice(&fs::read(user_settings_path()).unwrap()).unwrap()
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ── normalize ──────────────────────────────────────────────────

    #[test]
    fn normalize_trims_and_dedupes_preserving_order() {
        let got = normalize_entries(&v(&["  opus ", "sonnet", "opus", "haiku"])).unwrap();
        assert_eq!(got, v(&["opus", "sonnet", "haiku"]));
    }

    #[test]
    fn normalize_keeps_first_occurrence_because_order_is_load_bearing() {
        // With enforceAvailableModels, Default resolves to the FIRST
        // allowed entry — dropping the earlier duplicate would change
        // which model Default becomes.
        let got = normalize_entries(&v(&["sonnet", "opus", "sonnet"])).unwrap();
        assert_eq!(got[0], "sonnet");
    }

    #[test]
    fn normalize_rejects_empty_and_whitespace_entries() {
        assert_eq!(
            normalize_entries(&v(&["opus", "   "])),
            Err(AllowlistError::EmptyEntry(1))
        );
        assert_eq!(
            normalize_entries(&v(&["claude opus"])),
            Err(AllowlistError::ContainsWhitespace(0, "claude opus".into()))
        );
    }

    #[test]
    fn normalize_accepts_every_documented_entry_granularity() {
        let got = normalize_entries(&v(&[
            "sonnet",
            "claude-sonnet-4-5",
            "claude-sonnet-4-5-20250929",
            "us.anthropic.claude-opus-4-8",
        ]))
        .unwrap();
        assert_eq!(got.len(), 4);
    }

    // ── read ───────────────────────────────────────────────────────

    #[test]
    fn a_missing_file_reads_as_no_allowlist() {
        let (_t, _l) = isolated();
        let s = resolve_available_models();
        assert_eq!(s, AvailableModelsState::default());
        assert!(!s.restricts_models());
        assert!(!s.key_present);
    }

    #[test]
    fn entries_and_enforce_round_trip_from_disk() {
        let (_t, _l) = isolated();
        write_user_settings(
            r#"{"availableModels":["sonnet","haiku"],"enforceAvailableModels":true}"#,
        );
        let s = resolve_available_models();
        assert_eq!(s.entries, v(&["sonnet", "haiku"]));
        assert_eq!(s.enforce, Some(true));
        assert!(s.key_present);
        assert!(s.restricts_models());
        assert!(s.enforce_is_effective());
    }

    #[test]
    fn enforce_is_inert_with_an_empty_list() {
        // CC ignores enforceAvailableModels when the list is empty, so
        // the UI must not claim Default is restricted.
        let (_t, _l) = isolated();
        write_user_settings(r#"{"availableModels":[],"enforceAvailableModels":true}"#);
        let s = resolve_available_models();
        assert!(s.key_present, "an explicit [] is still a present key");
        assert!(!s.restricts_models());
        assert_eq!(s.enforce, Some(true));
        assert!(
            !s.enforce_is_effective(),
            "enforce with no entries restricts nothing"
        );
    }

    #[test]
    fn non_string_members_are_skipped_rather_than_failing_the_read() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"availableModels":["sonnet",42,null,"opus"]}"#);
        assert_eq!(resolve_available_models().entries, v(&["sonnet", "opus"]));
    }

    #[test]
    fn a_malformed_file_reads_as_no_allowlist() {
        let (_t, _l) = isolated();
        write_user_settings("{not json");
        assert_eq!(resolve_available_models(), AvailableModelsState::default());
    }

    #[test]
    fn a_non_array_value_reads_as_no_allowlist() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"availableModels":"sonnet"}"#);
        let s = resolve_available_models();
        assert!(s.entries.is_empty());
        assert!(!s.key_present);
    }

    // ── write ──────────────────────────────────────────────────────

    #[test]
    fn saving_writes_both_keys_together() {
        let (_t, _l) = isolated();
        set_available_models(&v(&["sonnet", "haiku"]), true).unwrap();
        let file = read_user_settings();
        assert_eq!(
            file["availableModels"],
            serde_json::json!(["sonnet", "haiku"])
        );
        assert_eq!(file["enforceAvailableModels"], JsonValue::Bool(true));
    }

    #[test]
    fn an_empty_list_removes_the_key_rather_than_writing_an_empty_array() {
        let (_t, _l) = isolated();
        write_user_settings(
            r#"{"availableModels":["sonnet"],"enforceAvailableModels":true,"keep":1}"#,
        );
        set_available_models(&[], false).unwrap();
        let file = read_user_settings();
        assert!(file.get("availableModels").is_none());
        assert!(file.get("enforceAvailableModels").is_none());
        assert_eq!(file["keep"], JsonValue::from(1), "siblings preserved");
    }

    #[test]
    fn enforce_is_not_written_without_a_list_to_enforce() {
        // CC ignores enforcement with an empty allowlist, so persisting
        // `true` there leaves a key that reads as a live restriction
        // and isn't one.
        //
        // Both keys end up absent, and since nothing else was in the
        // file, `mutate_settings` declines to create an empty `{}` —
        // so assert through the resolver, which handles a missing file.
        let (_t, _l) = isolated();
        set_available_models(&[], true).unwrap();
        let s = resolve_available_models();
        assert_eq!(s.enforce, None);
        assert!(s.entries.is_empty());
        assert!(!s.enforce_is_effective());
    }

    #[test]
    fn clearing_the_list_also_drops_a_previously_written_enforce() {
        let (_t, _l) = isolated();
        set_available_models(&v(&["opus"]), true).unwrap();
        assert_eq!(
            read_user_settings()["enforceAvailableModels"],
            JsonValue::Bool(true)
        );
        set_available_models(&[], true).unwrap();
        assert!(read_user_settings().get("enforceAvailableModels").is_none());
    }

    #[test]
    fn clearing_enforce_removes_the_key_rather_than_writing_false() {
        let (_t, _l) = isolated();
        set_available_models(&v(&["sonnet"]), true).unwrap();
        set_available_models(&v(&["sonnet"]), false).unwrap();
        let file = read_user_settings();
        assert!(file.get("enforceAvailableModels").is_none());
        assert_eq!(file["availableModels"], serde_json::json!(["sonnet"]));
    }

    #[test]
    fn saving_normalizes_and_reports_what_it_wrote() {
        let (_t, _l) = isolated();
        let written = set_available_models(&v(&[" opus ", "opus", "sonnet"]), false).unwrap();
        assert_eq!(written, v(&["opus", "sonnet"]));
        assert_eq!(
            read_user_settings()["availableModels"],
            serde_json::json!(["opus", "sonnet"])
        );
    }

    #[test]
    fn an_invalid_entry_is_refused_before_anything_is_written() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"keep":1}"#);
        let err = set_available_models(&v(&["opus", ""]), false).unwrap_err();
        assert!(matches!(
            err,
            SetAllowlistError::Allowlist(AllowlistError::EmptyEntry(1))
        ));
        let file = read_user_settings();
        assert!(file.get("availableModels").is_none());
        assert_eq!(file["keep"], JsonValue::from(1));
    }

    #[test]
    fn saving_preserves_unrelated_settings() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"theme":"dark","fastMode":true}"#);
        set_available_models(&v(&["opus"]), true).unwrap();
        let file = read_user_settings();
        assert_eq!(file["theme"], JsonValue::from("dark"));
        assert_eq!(file["fastMode"], JsonValue::Bool(true));
    }

    #[test]
    fn a_saved_allowlist_reads_back_identically() {
        let (_t, _l) = isolated();
        set_available_models(&v(&["sonnet", "claude-opus-4-8"]), true).unwrap();
        let s = resolve_available_models();
        assert_eq!(s.entries, v(&["sonnet", "claude-opus-4-8"]));
        assert!(s.enforce_is_effective());
    }
}
