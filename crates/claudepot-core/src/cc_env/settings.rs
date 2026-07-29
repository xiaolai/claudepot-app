//! Key-preserving read-modify-write of `settings.json`'s `env` map.
//!
//! "Format-preserving" would be the wrong word, and every precedent in this
//! crate says so: JSON has no comments, and each of them parses and
//! pretty-prints. The guarantee is **key-preserving** — every key we did not
//! touch survives, inside `env` and outside it.
//!
//! v1 writes the **user scope only** (`~/.claude/settings.json`). That is not
//! only scope control. Claude Code applies just the `SAFE_ENV_VARS` allowlist
//! from project-scoped settings before the trust dialog, precisely because
//! *"they live inside the project directory and could be committed by a
//! malicious actor to redirect traffic"* (`utils/managedEnv.ts:103`). An
//! editor for that layer is a different security design, not a layer selector.

use crate::cc_env::errors::CcEnvError;
use crate::cc_env::spec::{self, EnvControl, EnvVarSpec};
use crate::paths;
use crate::settings_mutex::{mutate_settings_file, Change};
use serde_json::{Map, Value as JsonValue};
use std::path::{Path, PathBuf};

/// The settings key this module owns.
pub const ENV_KEY: &str = "env";

/// `~/.claude/settings.json` — the one file this pane edits.
pub fn user_settings_path() -> PathBuf {
    paths::claude_config_dir().join("settings.json")
}

/// What a write left behind.
///
/// Deliberately carries **no values**. An earlier shape returned the whole
/// post-write `env` map so a caller could reconcile against it — which would
/// have handed every caller a `serde_json::Value` holding
/// `ANTHROPIC_API_KEY`'s plaintext, one `serde_json::to_string` away from a
/// log line. The authoritative post-write state the renderer needs is the
/// redacted [`crate::cc_env::state::EnvOverview`], which the command layer
/// re-reads; that projection is the only thing entitled to leave core with a
/// value attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvWriteOutcome {
    pub wrote: bool,
    /// Another writer moved the file mid-edit and the mutation was re-run
    /// against the newer bytes.
    pub rebased: bool,
    /// How many keys the `env` map holds now. A count is safe to report and
    /// enough to tell "the map is empty, so the object was removed".
    pub env_len: usize,
}

fn type_name(v: &JsonValue) -> &'static str {
    match v {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "bool",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

/// Pull the `env` object out of an already-parsed settings root.
///
/// Missing → empty. Present but not an object → `Err`. Never coerced: a user
/// who wrote `"env": null` meant something, and silently replacing it with
/// `{}` would destroy the evidence of whatever that was.
fn env_of(
    root: &Map<String, JsonValue>,
    path: &Path,
) -> Result<Map<String, JsonValue>, CcEnvError> {
    match root.get(ENV_KEY) {
        None => Ok(Map::new()),
        Some(JsonValue::Object(m)) => Ok(m.clone()),
        Some(other) => Err(CcEnvError::EnvNotAnObject {
            path: path.to_path_buf(),
            found: type_name(other),
        }),
    }
}

/// Read the `env` map. Missing file, empty file, or missing `env` key all
/// read as an empty map; a malformed file or a non-object `env` is an error.
pub fn read_env_map(path: &Path) -> Result<Map<String, JsonValue>, CcEnvError> {
    let root = crate::settings_mutex::read_settings_file(path)?;
    env_of(&root, path)
}

/// Read the global user config's own `env` block — the source Claude Code
/// applies *before* `settings.json` (`utils/managedEnv.ts:136,188`).
///
/// Settings wins where both set a key, but a variable absent from
/// settings.json may still be **set** here, which is why a row with no
/// settings entry may not claim "CC default".
///
/// Resolved through [`paths::resolved_global_claude_json`], not hardcoded to
/// `~/.claude.json`: with `CLAUDE_CONFIG_DIR` set, CC reads a different file,
/// and a reader pointed at the home-directory sibling would report "no lower
/// source" about a file CC is actively applying — the precise claim §4.4
/// exists to keep honest.
///
/// A missing or malformed file reads as empty rather than erroring: it is not
/// ours, it is enormous, and refusing to render the pane because someone
/// else's state file is unparseable would be the wrong trade.
pub fn read_legacy_global_env() -> Map<String, JsonValue> {
    let Some(path) = paths::resolved_global_claude_json() else {
        return Map::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Map::new();
    };
    let Ok(JsonValue::Object(root)) = serde_json::from_slice::<JsonValue>(&bytes) else {
        return Map::new();
    };
    match root.get(ENV_KEY) {
        Some(JsonValue::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

/// Whether a string is an integer *in form*.
///
/// Deliberately not `parse::<i64>()`. That would impose Rust's 64-bit range
/// on a value Claude Code reads with `parseInt`, so a syntactically fine
/// `99999999999999999999` would be rejected here and accepted there — the
/// same "inventing bounds" mistake as a made-up maximum, just wearing a type.
pub(crate) fn is_integer_syntax(value: &str) -> bool {
    let digits = value.strip_prefix(['-', '+']).unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Reject a value the spec says the variable cannot take.
///
/// Numbers validate **syntax only**: the spec carries no min or max, and
/// inventing bounds here would reject values Claude Code accepts.
pub fn validate(spec: &EnvVarSpec, value: &str) -> Result<(), CcEnvError> {
    match spec.control {
        EnvControl::Number => {
            if is_integer_syntax(value) {
                Ok(())
            } else {
                Err(CcEnvError::InvalidNumber {
                    name: spec.name.clone(),
                    value: crate::cc_env::errors::redacted(spec, value),
                })
            }
        }
        EnvControl::Enum | EnvControl::Toggle => {
            let allowed = spec.values.clone().unwrap_or_default();
            if allowed.iter().any(|v| v == value) {
                Ok(())
            } else {
                Err(CcEnvError::InvalidEnumValue {
                    name: spec.name.clone(),
                    value: crate::cc_env::errors::redacted(spec, value),
                    allowed,
                })
            }
        }
        // Free text, including the empty string: an explicit `""` is a real
        // process state, distinct from unset, and refusing it would make one
        // of the three states unreachable.
        EnvControl::Text => Ok(()),
    }
}

/// Set one documented, editable variable in the **user** settings file.
///
/// The path is not a parameter. v1 edits `~/.claude/settings.json` and only
/// that file; taking a path here would leave "user scope only" as caller
/// discipline, when it is a security boundary — CC applies just the
/// `SAFE_ENV_VARS` allowlist from project-scoped settings pre-trust, so
/// editing that layer is a different design, not a different argument.
pub fn set_user_env_var(name: &str, value: &str) -> Result<EnvWriteOutcome, CcEnvError> {
    set_env_var(&user_settings_path(), name, value)
}

/// Remove one key from the **user** settings file's `env` map. Sibling of
/// [`set_user_env_var`]; see it for why the path is not a parameter.
pub fn clear_user_env_var(name: &str) -> Result<EnvWriteOutcome, CcEnvError> {
    clear_env_var(&user_settings_path(), name)
}

/// Path-taking form of [`set_user_env_var`].
///
/// `pub(crate)` on purpose: tests need to point at a temp file, and nothing
/// outside this crate has a legitimate reason to choose the settings layer.
///
/// Values are always written as JSON strings — CC's settings schema types
/// `env` as `Record<string, string>`, and a number or bool there is a value
/// CC will not read back the way it was written.
pub(crate) fn set_env_var(
    path: &Path,
    name: &str,
    value: &str,
) -> Result<EnvWriteOutcome, CcEnvError> {
    let spec = spec::lookup(name).ok_or_else(|| CcEnvError::UnknownVariable(name.to_string()))?;
    if let Some(reason) = spec.safety.blocked_reason {
        return Err(CcEnvError::NotEditable {
            name: name.to_string(),
            reason,
        });
    }
    validate(spec, value)?;

    mutate_settings_file(path, |root, _| {
        let mut env = env_of(root, path)?;
        env.insert(name.to_string(), JsonValue::String(value.to_string()));
        let len = env.len();
        root.insert(ENV_KEY.to_string(), JsonValue::Object(env));
        Ok::<_, CcEnvError>(Change::Write(len))
    })
    .map(outcome_of)
}

/// Shared projection from the mutation boundary's result. Reports the key
/// count and never the keys.
fn outcome_of(m: crate::settings_mutex::Mutation<usize>) -> EnvWriteOutcome {
    EnvWriteOutcome {
        wrote: m.wrote,
        rebased: m.rebased,
        env_len: m.value,
    }
}

/// Remove one key from the `env` map.
///
/// Accepts **any name actually present**, documented or not — that is what
/// makes a hand-set key removable from the pane that claims to show env
/// config. Blocked names are still refused: their rows are read-only, and a
/// pane that cannot set a value should not be able to unset one either.
///
/// Clearing the last key removes the now-empty `env` object rather than
/// leaving `"env": {}` behind.
///
/// This never takes effect in a running session. CC re-applies `settings.env`
/// with `Object.assign` and nothing else — *"additive-only: new vars are
/// added, existing may be overwritten, nothing is deleted"*
/// (`state/onChangeAppState.ts:163`) — so the old value survives in the
/// process environment until Claude Code is relaunched. Every confirmation
/// around this call has to say so.
pub(crate) fn clear_env_var(path: &Path, name: &str) -> Result<EnvWriteOutcome, CcEnvError> {
    if let Some(reason) = spec::lookup(name).and_then(|s| s.safety.blocked_reason) {
        return Err(CcEnvError::NotEditable {
            name: name.to_string(),
            reason,
        });
    }

    mutate_settings_file(path, |root, was| {
        if !was.is_present() {
            return Err(CcEnvError::NotSet(name.to_string()));
        }
        let mut env = env_of(root, path)?;
        if env.remove(name).is_none() {
            return Err(CcEnvError::NotSet(name.to_string()));
        }
        let len = env.len();
        if env.is_empty() {
            root.remove(ENV_KEY);
        } else {
            root.insert(ENV_KEY.to_string(), JsonValue::Object(env));
        }
        Ok(Change::Write(len))
    })
    .map(outcome_of)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn tmp_settings() -> (TempDir, PathBuf) {
        let t = TempDir::new().unwrap();
        let p = t.path().join("settings.json");
        (t, p)
    }

    /// All three states of a tri-state toggle survive a full write → read
    /// → project cycle, and stay distinguishable from each other.
    ///
    /// `unset` is the one that matters: it is not a value, so it round-trips
    /// as the key being *gone*. A cycle that turned it into `"0"` — the
    /// obvious shortcut — would silently pin the variable off for a user who
    /// asked to hand the decision back to Claude Code.
    #[test]
    fn tri_state_round_trips_through_all_three_states() {
        use crate::cc_env::state::{project, EnvValue};
        let (_t, p) = tmp_settings();
        let spec = spec::lookup("USE_BUILTIN_RIPGREP").unwrap();
        assert!(spec.is_tristate());

        for value in ["0", "1"] {
            set_env_var(&p, "USE_BUILTIN_RIPGREP", value).unwrap();
            let env = read_env_map(&p).unwrap();
            assert_eq!(
                project(spec, env.get("USE_BUILTIN_RIPGREP")),
                EnvValue::Known {
                    value: value.to_string()
                }
            );
        }

        clear_env_var(&p, "USE_BUILTIN_RIPGREP").unwrap();
        let env = read_env_map(&p).unwrap();
        assert!(!env.contains_key("USE_BUILTIN_RIPGREP"));
        assert_eq!(
            project(spec, env.get("USE_BUILTIN_RIPGREP")),
            EnvValue::Absent
        );
    }

    #[test]
    fn set_preserves_siblings_inside_and_outside_env() {
        let (_t, p) = tmp_settings();
        std::fs::write(
            &p,
            br#"{"model":"opus","env":{"KEEP_ME":"yes"},"permissions":{"defaultMode":"plan"}}"#,
        )
        .unwrap();

        set_env_var(&p, "MAX_THINKING_TOKENS", "31999").unwrap();

        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["model"], json!("opus"));
        assert_eq!(after["permissions"]["defaultMode"], json!("plan"));
        assert_eq!(after["env"]["KEEP_ME"], json!("yes"));
        assert_eq!(after["env"]["MAX_THINKING_TOKENS"], json!("31999"));
    }

    #[test]
    fn values_are_written_as_strings() {
        let (_t, p) = tmp_settings();
        set_env_var(&p, "MAX_THINKING_TOKENS", "1024").unwrap();
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(after["env"]["MAX_THINKING_TOKENS"].is_string());
    }

    #[test]
    fn clear_removes_the_key_and_the_emptied_env_object() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, br#"{"model":"opus","env":{"USE_BUILTIN_RIPGREP":"0"}}"#).unwrap();

        let out = clear_env_var(&p, "USE_BUILTIN_RIPGREP").unwrap();
        assert!(out.wrote);
        assert_eq!(out.env_len, 0);

        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["model"], json!("opus"));
        assert!(
            after.as_object().unwrap().get("env").is_none(),
            "an emptied env object should be removed, not left as {{}}"
        );
    }

    #[test]
    fn clear_keeps_env_when_other_keys_remain() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, br#"{"env":{"A":"1","USE_BUILTIN_RIPGREP":"0"}}"#).unwrap();
        clear_env_var(&p, "USE_BUILTIN_RIPGREP").unwrap();
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["env"]["A"], json!("1"));
    }

    #[test]
    fn clear_accepts_an_undocumented_key_that_is_actually_set() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, br#"{"env":{"CLAUDE_CODE_SOMETHING_INTERNAL":"1"}}"#).unwrap();
        clear_env_var(&p, "CLAUDE_CODE_SOMETHING_INTERNAL").unwrap();
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(after.as_object().unwrap().get("env").is_none());
    }

    #[test]
    fn clear_of_an_unset_key_is_an_error_not_a_silent_noop() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, br#"{"env":{"A":"1"}}"#).unwrap();
        assert!(matches!(
            clear_env_var(&p, "USE_BUILTIN_RIPGREP").unwrap_err(),
            CcEnvError::NotSet(_)
        ));
        assert!(matches!(
            clear_env_var(&p, "ANYTHING").unwrap_err(),
            CcEnvError::NotSet(_)
        ));
    }

    #[test]
    fn a_missing_file_is_created_only_when_actually_writing() {
        let (_t, p) = tmp_settings();
        assert!(clear_env_var(&p, "USE_BUILTIN_RIPGREP").is_err());
        assert!(!p.exists(), "a failed clear must not create the file");

        set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap();
        assert!(p.exists());
    }

    #[test]
    fn malformed_file_errors_and_is_left_byte_identical() {
        let (_t, p) = tmp_settings();
        let original = b"{ this is not json".to_vec();
        std::fs::write(&p, &original).unwrap();
        assert!(matches!(
            set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap_err(),
            CcEnvError::JsonParse(_)
        ));
        assert_eq!(std::fs::read(&p).unwrap(), original);
    }

    #[test]
    fn non_object_root_errors() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, b"[1,2]").unwrap();
        assert!(matches!(
            set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap_err(),
            CcEnvError::NotAJsonObject(_)
        ));
    }

    #[test]
    fn env_as_null_array_or_string_errors_and_is_never_coerced() {
        for body in [
            &br#"{"env":null}"#[..],
            &br#"{"env":[1,2]}"#[..],
            &br#"{"env":"nope"}"#[..],
        ] {
            let (_t, p) = tmp_settings();
            std::fs::write(&p, body).unwrap();
            let err = set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap_err();
            assert!(
                matches!(err, CcEnvError::EnvNotAnObject { .. }),
                "got {err:?}"
            );
            assert_eq!(std::fs::read(&p).unwrap(), body, "file was rewritten");
            assert!(matches!(
                read_env_map(&p).unwrap_err(),
                CcEnvError::EnvNotAnObject { .. }
            ));
        }
    }

    #[test]
    fn a_non_string_child_survives_a_neighbouring_write_untouched() {
        let (_t, p) = tmp_settings();
        std::fs::write(
            &p,
            br#"{"env":{"WEIRD":{"nested":true},"ALSO_WEIRD":42,"NULLED":null}}"#,
        )
        .unwrap();

        set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap();

        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["env"]["WEIRD"], json!({"nested": true}));
        assert_eq!(after["env"]["ALSO_WEIRD"], json!(42));
        assert_eq!(after["env"]["NULLED"], JsonValue::Null);
        assert_eq!(after["env"]["USE_BUILTIN_RIPGREP"], json!("0"));
    }

    #[test]
    fn unrecognized_keys_survive_a_neighbouring_write() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, br#"{"env":{"TOTALLY_MADE_UP":"keep"}}"#).unwrap();
        set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap();
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["env"]["TOTALLY_MADE_UP"], json!("keep"));
    }

    #[test]
    fn set_refuses_unknown_and_blocked_names() {
        let (_t, p) = tmp_settings();
        assert!(matches!(
            set_env_var(&p, "NOT_A_REAL_VARIABLE", "1").unwrap_err(),
            CcEnvError::UnknownVariable(_)
        ));
        assert!(matches!(
            set_env_var(&p, "CLAUDE_CONFIG_DIR", "/tmp/x").unwrap_err(),
            CcEnvError::NotEditable { .. }
        ));
        assert!(!p.exists());
    }

    #[test]
    fn clear_refuses_a_blocked_name_even_when_it_is_set() {
        let (_t, p) = tmp_settings();
        std::fs::write(&p, br#"{"env":{"CLAUDE_CONFIG_DIR":"/tmp/x"}}"#).unwrap();
        assert!(matches!(
            clear_env_var(&p, "CLAUDE_CONFIG_DIR").unwrap_err(),
            CcEnvError::NotEditable { .. }
        ));
    }

    #[test]
    fn validation_rejects_out_of_enum_and_non_integer() {
        let (_t, p) = tmp_settings();
        let err = set_env_var(&p, "MAX_THINKING_TOKENS", "12x").unwrap_err();
        assert!(matches!(err, CcEnvError::InvalidNumber { .. }));
        assert!(err.to_string().contains("12x"));

        assert!(matches!(
            set_env_var(&p, "CLAUDE_CODE_EFFORT_LEVEL", "ludicrous").unwrap_err(),
            CcEnvError::InvalidEnumValue { .. }
        ));
        assert!(matches!(
            set_env_var(&p, "USE_BUILTIN_RIPGREP", "true").unwrap_err(),
            CcEnvError::InvalidEnumValue { .. }
        ));
        // Every documented value of each control is accepted.
        set_env_var(&p, "CLAUDE_CODE_EFFORT_LEVEL", "max").unwrap();
        set_env_var(&p, "USE_BUILTIN_RIPGREP", "0").unwrap();
        set_env_var(&p, "MAX_THINKING_TOKENS", "-1").unwrap();
    }

    #[test]
    fn numbers_validate_syntax_only() {
        let (_t, p) = tmp_settings();
        // Absurd but syntactically valid: the spec carries no bounds and
        // inventing them would reject values Claude Code accepts.
        set_env_var(&p, "MAX_THINKING_TOKENS", "999999999").unwrap();
        set_env_var(&p, "MAX_THINKING_TOKENS", "0").unwrap();
    }

    #[test]
    fn a_secret_value_never_reaches_an_error_message() {
        let (_t, p) = tmp_settings();
        // Force a validation failure on a secret variable by reaching past
        // the control type — the guard has to hold whatever the control is.
        let spec = spec::lookup("ANTHROPIC_API_KEY").unwrap();
        let msg = CcEnvError::InvalidEnumValue {
            name: spec.name.clone(),
            value: crate::cc_env::errors::redacted(spec, "sk-ant-oat01-secret"),
            allowed: vec!["a".into()],
        }
        .to_string();
        assert!(!msg.contains("sk-ant"), "{msg}");
        let _ = p;
    }

    #[test]
    fn empty_string_is_writable_because_it_is_a_real_process_state() {
        let (_t, p) = tmp_settings();
        set_env_var(&p, "ANTHROPIC_BASE_URL", "").unwrap();
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["env"]["ANTHROPIC_BASE_URL"], json!(""));
    }

    #[test]
    fn read_env_map_treats_missing_file_and_missing_key_as_empty() {
        let (_t, p) = tmp_settings();
        assert!(read_env_map(&p).unwrap().is_empty());
        std::fs::write(&p, br#"{"model":"opus"}"#).unwrap();
        assert!(read_env_map(&p).unwrap().is_empty());
    }

    /// The §4.1 race, with `cc_env` as one of the two participants.
    /// `read_legacy_global_env` must follow CC's own resolution, not assume
    /// `~/.claude.json`. With `CLAUDE_CONFIG_DIR` set they are different
    /// files, and reading the wrong one makes "no lower-source override" a
    /// false statement about a file CC is applying.
    #[test]
    fn the_legacy_global_reader_follows_cc_config_dir_and_the_legacy_name() {
        let _lock = crate::testing::lock_data_dir();
        let t = TempDir::new().unwrap();
        let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", t.path());

        // Nothing there yet.
        assert!(read_legacy_global_env().is_empty());

        // `$CLAUDE_CONFIG_DIR/.claude.json` — not the home sibling.
        std::fs::write(
            t.path().join(".claude.json"),
            br#"{"env":{"MAX_THINKING_TOKENS":"4096"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_legacy_global_env().get("MAX_THINKING_TOKENS"),
            Some(&JsonValue::String("4096".into()))
        );

        // The legacy `.config.json` wins when present, exactly as CC's
        // `getGlobalClaudeFile` does.
        std::fs::write(
            t.path().join(".config.json"),
            br#"{"env":{"MAX_THINKING_TOKENS":"1"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_legacy_global_env().get("MAX_THINKING_TOKENS"),
            Some(&JsonValue::String("1".into()))
        );

        // A malformed or non-object `env` reads as empty, never a panic:
        // that file is not ours to refuse to start over.
        std::fs::write(t.path().join(".config.json"), b"{ not json").unwrap();
        assert!(read_legacy_global_env().is_empty());
        std::fs::write(t.path().join(".config.json"), br#"{"env":7}"#).unwrap();
        assert!(read_legacy_global_env().is_empty());

        match prev {
            Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }
    }

    #[test]
    fn a_cc_env_write_and_a_settings_writer_write_both_land() {
        let t = TempDir::new().unwrap();
        let _lock = crate::testing::lock_data_dir();
        let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", t.path());
        let p = user_settings_path();
        std::fs::write(&p, b"{}").unwrap();
        let project = t.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        std::thread::scope(|s| {
            s.spawn(|| {
                for i in 0..30 {
                    set_env_var(&p, "MAX_THINKING_TOKENS", &i.to_string()).unwrap();
                }
            });
            s.spawn(|| {
                for i in 0..30 {
                    crate::settings_writer::write_bool_setting(
                        crate::settings_writer::SettingsLayer::User,
                        &project,
                        &format!("otherKey{i}"),
                        true,
                    )
                    .unwrap();
                }
            });
        });

        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["env"]["MAX_THINKING_TOKENS"], json!("29"));
        for i in 0..30 {
            assert!(
                after.get(format!("otherKey{i}")).is_some(),
                "settings_writer key {i} was clobbered by cc_env"
            );
        }

        // Restore: CLAUDE_CONFIG_DIR is process-global, and leaving it
        // pointed at a tempdir that is about to be deleted breaks whichever
        // test runs next.
        match prev {
            Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }
    }
}
