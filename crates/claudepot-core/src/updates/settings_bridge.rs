//! Read and write the four CC settings keys that gate auto-update
//! behavior. The file is `~/.claude/settings.json` (overridable via
//! `$CLAUDE_CONFIG_DIR`). Schema we touch:
//!
//! ```json
//! {
//!   "autoUpdatesChannel": "latest" | "stable",
//!   "minimumVersion": "2.1.100",
//!   "env": {
//!     "DISABLE_AUTOUPDATER": "1",
//!     "DISABLE_UPDATES": "1"
//!   }
//! }
//! ```
//!
//! We never overwrite or remove keys we don't manage. Read-modify-
//! write parses, mutates, then re-serializes pretty-printed. We do
//! NOT preserve comments — JSON has none — but we do preserve the
//! *order* of any existing keys via `serde_json::Map`'s
//! `preserve_order` feature is NOT enabled in this crate; insertion
//! order is preserved for new keys but existing keys are read in
//! sorted order. Acceptable: this file is rarely human-edited.

use crate::paths;
use crate::updates::errors::{Result, UpdateError};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// Snapshot of the four keys we care about. None for unset keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CcUpdateSettings {
    pub auto_updates_channel: Option<String>,
    pub minimum_version: Option<String>,
    pub disable_autoupdater: bool,
    pub disable_updates: bool,
}

/// The two keys this module owns, named once so the reader, the writers and
/// the transition planner cannot disagree about them.
const CHANNEL_KEY: &str = "autoUpdatesChannel";
const MINIMUM_VERSION_KEY: &str = "minimumVersion";

fn settings_path() -> PathBuf {
    paths::claude_config_dir().join("settings.json")
}

fn read_root() -> Result<Map<String, Value>> {
    let p = settings_path();
    // Match on the read rather than `exists()` then read: between those two
    // the file can disappear, and the second call would surface as an I/O
    // error where "missing" is the answer we already have a branch for.
    let body = match std::fs::read_to_string(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e.into()),
    };
    if body.trim().is_empty() {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_str(&body)?;
    match v {
        Value::Object(m) => Ok(m),
        other => Err(UpdateError::Parse(format!(
            "{} root is not an object: {}",
            p.display(),
            type_name(&other)
        ))),
    }
}

/// Apply `edit` to `~/.claude/settings.json` through the shared mutation
/// boundary.
///
/// This module used to read the whole root, mutate it, and persist a temp
/// file over the top. That is crash-safe and not concurrency-safe: an
/// overlapping read-modify-write from `settings_writer` or the env pane read
/// the same old bytes, and whichever renamed last silently discarded the
/// other's edit. `settings.json` is the file all three write, so a lock only
/// one of them held would not be a lock at all.
///
/// Two incidental changes come with the move, both toward what every other
/// writer of this file already did: the output is newline-terminated, and it
/// lands via `fs_utils::atomic_write`, which chmods 0600 on Unix.
///
/// The closure may run more than once (see [`crate::settings_mutex`]).
fn edit_root(mut edit: impl FnMut(&mut Map<String, Value>)) -> Result<()> {
    crate::settings_mutex::mutate_settings_file(&settings_path(), |root, _| {
        edit(root);
        Ok::<_, UpdateError>(crate::settings_mutex::Change::Write(()))
    })
    .map(|_| ())
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn env_truthy(v: &Value) -> bool {
    match v {
        Value::String(s) => matches!(s.as_str(), "1" | "true" | "TRUE" | "True" | "yes" | "YES"),
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_u64().map(|x| x != 0).unwrap_or(false),
        _ => false,
    }
}

pub fn read() -> Result<CcUpdateSettings> {
    let root = read_root()?;
    let auto_updates_channel = root
        .get(CHANNEL_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let minimum_version = root
        .get(MINIMUM_VERSION_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let env = root.get("env").and_then(|v| v.as_object());
    let disable_autoupdater = env
        .and_then(|e| e.get("DISABLE_AUTOUPDATER"))
        .map(env_truthy)
        .unwrap_or(false);
    let disable_updates = env
        .and_then(|e| e.get("DISABLE_UPDATES"))
        .map(env_truthy)
        .unwrap_or(false);
    Ok(CcUpdateSettings {
        auto_updates_channel,
        minimum_version,
        disable_autoupdater,
        disable_updates,
    })
}

/// Write `autoUpdatesChannel`. Pass `None` to remove the key
/// (revert to CC's default).
///
/// **Failure mode**: errors out instead of overwriting if the file
/// exists but is malformed. `read_root()` already returns `Ok(empty)`
/// for the missing/empty cases, so the only error path here is
/// "user has unparseable settings.json" — silently overwriting that
/// would destroy their other settings.
pub fn write_channel(channel: Option<&str>) -> Result<()> {
    edit_root(|root| set_optional_string(root, CHANNEL_KEY, channel))
}

/// Set a top-level optional string key, or remove it when `value` is `None`.
///
/// `write_channel` and `write_minimum_version` were byte-for-byte copies of
/// this differing only in the key name; `change_channel` needs the same
/// operation a third time, inside its own closure.
fn set_optional_string(root: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    match value {
        Some(v) => {
            root.insert(key.to_string(), Value::String(v.to_string()));
        }
        None => {
            root.remove(key);
        }
    }
}

/// Write `minimumVersion`. Pass `None` to clear the floor.
///
/// Same failure-mode contract as [`write_channel`]: malformed file →
/// error, never destructive overwrite.
pub fn write_minimum_version(version: Option<&str>) -> Result<()> {
    edit_root(|root| set_optional_string(root, MINIMUM_VERSION_KEY, version))
}

/// Switch CC's release channel with the same `minimumVersion`
/// semantics CC's own `/config` UI applies.
///
/// **`allow_downgrade`** — atomic user choice for `latest → stable`,
/// matching CC's "downgrade now" vs "stay pinned" prompt:
/// - `false` (the **safer default**) = pin `minimumVersion` to the
///   currently-installed version so the user doesn't get
///   involuntarily downgraded from a `latest` build that's newer
///   than the current `stable`.
/// - `true` = explicitly opt into downgrading; clears any existing
///   `minimumVersion` floor along with the channel switch.
///
/// Other transitions ignore `allow_downgrade`:
/// - **stable → latest** always clears `minimumVersion` so the floor
///   doesn't block forward motion on the rolling channel.
/// - **same → same** is a no-op, no writes.
///
/// `installed_version` should be the active CC binary's version
/// (via `detect_cli_installs`). Pass `None` if unknown — the
/// `latest → stable` pin path will skip the write (matching CC's
/// behavior when the version probe fails).
///
/// Returns the previous channel (parsed; defaults to `Latest` when
/// unset) so callers can show "switched from X to Y" feedback.
pub fn change_channel(
    new_channel: &str,
    installed_version: Option<&str>,
    allow_downgrade: bool,
) -> Result<String> {
    if new_channel != "latest" && new_channel != "stable" {
        return Err(UpdateError::Parse(format!(
            "unknown channel: {new_channel:?} (expected 'latest' or 'stable')"
        )));
    }
    // The whole transition happens inside ONE mutation: the previous channel
    // is read from the root the boundary hands us, both keys move together,
    // and the result lands in a single atomic write.
    //
    // It used to read through `read()` and then call two independent
    // read-modify-writes. That is two bugs. The decision came from a snapshot
    // nothing held a lock on, so a concurrent write could make the branch
    // wrong; and a failure between the two writes could leave
    // `minimumVersion` cleared with the channel unchanged — a half-applied
    // state, from a function whose own docs call this an "atomic user
    // choice".
    let outcome = crate::settings_mutex::mutate_settings_file(&settings_path(), |root, _| {
        let prev_channel = root
            .get(CHANNEL_KEY)
            .and_then(Value::as_str)
            .unwrap_or("latest")
            .to_string();

        if prev_channel == new_channel {
            return Ok::<_, UpdateError>(crate::settings_mutex::Change::Skip(prev_channel));
        }

        match (prev_channel.as_str(), new_channel) {
            ("latest", "stable") => {
                if allow_downgrade {
                    // User explicitly accepted downgrade — clear any
                    // pre-existing floor so stable can land at whatever
                    // it is right now.
                    set_optional_string(root, MINIMUM_VERSION_KEY, None);
                } else if let Some(v) = installed_version {
                    // Default: pin to current to avoid an involuntary
                    // downgrade.
                    set_optional_string(root, MINIMUM_VERSION_KEY, Some(v));
                }
                set_optional_string(root, CHANNEL_KEY, Some("stable"));
            }
            ("stable", "latest") => {
                set_optional_string(root, MINIMUM_VERSION_KEY, None);
                set_optional_string(root, CHANNEL_KEY, Some("latest"));
            }
            // Anything we didn't enumerate (e.g., a future channel name
            // already in settings.json) gets the simple write — no
            // minimumVersion gymnastics, since we don't know the rules.
            _ => {
                set_optional_string(root, CHANNEL_KEY, Some(new_channel));
            }
        }
        Ok(crate::settings_mutex::Change::Write(prev_channel))
    })?;
    Ok(outcome.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;
    use tempfile::tempdir;

    fn with_temp_config<F: FnOnce()>(f: F) {
        let _lock = lock_data_dir();
        let tmp = tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
        f();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn read_returns_defaults_when_file_missing() {
        with_temp_config(|| {
            let s = read().unwrap();
            assert_eq!(s, CcUpdateSettings::default());
        });
    }

    #[test]
    fn read_picks_up_channel_and_env() {
        with_temp_config(|| {
            let path = paths::claude_config_dir().join("settings.json");
            std::fs::write(
                &path,
                r#"{
                    "autoUpdatesChannel": "stable",
                    "minimumVersion": "2.1.100",
                    "env": {
                        "DISABLE_AUTOUPDATER": "1",
                        "DISABLE_UPDATES": "0"
                    }
                }"#,
            )
            .unwrap();
            let s = read().unwrap();
            assert_eq!(s.auto_updates_channel.as_deref(), Some("stable"));
            assert_eq!(s.minimum_version.as_deref(), Some("2.1.100"));
            assert!(s.disable_autoupdater);
            assert!(!s.disable_updates);
        });
    }

    #[test]
    fn write_channel_preserves_other_keys() {
        with_temp_config(|| {
            let path = paths::claude_config_dir().join("settings.json");
            std::fs::write(&path, r#"{"theme":"dark","permissions":{"allow":["x"]}}"#).unwrap();
            write_channel(Some("stable")).unwrap();
            let body: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(body["autoUpdatesChannel"], "stable");
            assert_eq!(body["theme"], "dark");
            assert_eq!(body["permissions"]["allow"][0], "x");
        });
    }

    #[test]
    fn write_channel_creates_file_when_missing() {
        with_temp_config(|| {
            write_channel(Some("latest")).unwrap();
            let s = read().unwrap();
            assert_eq!(s.auto_updates_channel.as_deref(), Some("latest"));
        });
    }

    #[test]
    fn write_channel_none_removes_key() {
        with_temp_config(|| {
            write_channel(Some("stable")).unwrap();
            write_channel(None).unwrap();
            let s = read().unwrap();
            assert!(s.auto_updates_channel.is_none());
        });
    }

    #[test]
    fn write_minimum_version_roundtrips() {
        with_temp_config(|| {
            write_minimum_version(Some("2.1.100")).unwrap();
            assert_eq!(read().unwrap().minimum_version.as_deref(), Some("2.1.100"));
            write_minimum_version(None).unwrap();
            assert!(read().unwrap().minimum_version.is_none());
        });
    }

    #[test]
    fn read_handles_empty_file() {
        with_temp_config(|| {
            let path = paths::claude_config_dir().join("settings.json");
            std::fs::write(&path, "").unwrap();
            let s = read().unwrap();
            assert_eq!(s, CcUpdateSettings::default());
        });
    }

    #[test]
    fn read_rejects_non_object_root() {
        with_temp_config(|| {
            let path = paths::claude_config_dir().join("settings.json");
            std::fs::write(&path, "[1,2,3]").unwrap();
            let r = read();
            assert!(matches!(r, Err(UpdateError::Parse(_))));
        });
    }
    // ─── change_channel transitions ─────────────────────────────────
    //
    // These did not exist. `change_channel` is the one function here that
    // decides policy AND writes, and every branch of it was untested — which
    // is how it kept a stale-snapshot read and two separate writes while its
    // own doc comment promised an atomic choice.

    fn settings_json() -> Value {
        serde_json::from_str(&std::fs::read_to_string(settings_path()).unwrap()).unwrap()
    }

    fn seed(body: &str) {
        let p = settings_path();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
    }

    #[test]
    fn latest_to_stable_pins_the_floor_by_default() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"latest","keep":1}"#);
            let prev = change_channel("stable", Some("2.1.220"), false).unwrap();
            assert_eq!(prev, "latest");
            let v = settings_json();
            assert_eq!(v[CHANNEL_KEY], "stable");
            assert_eq!(v[MINIMUM_VERSION_KEY], "2.1.220");
            assert_eq!(v["keep"], 1, "unrelated keys must survive");
        });
    }

    #[test]
    fn latest_to_stable_with_allow_downgrade_clears_the_floor() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"latest","minimumVersion":"2.1.100"}"#);
            change_channel("stable", Some("2.1.220"), true).unwrap();
            let v = settings_json();
            assert_eq!(v[CHANNEL_KEY], "stable");
            assert!(v.get(MINIMUM_VERSION_KEY).is_none());
        });
    }

    /// CC skips the pin when its version probe fails, so we do too — pinning
    /// to an unknown version would be inventing a floor.
    #[test]
    fn latest_to_stable_without_a_known_version_skips_the_pin() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"latest"}"#);
            change_channel("stable", None, false).unwrap();
            let v = settings_json();
            assert_eq!(v[CHANNEL_KEY], "stable");
            assert!(v.get(MINIMUM_VERSION_KEY).is_none());
        });
    }

    #[test]
    fn stable_to_latest_clears_the_floor_so_it_cannot_block_forward_motion() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"stable","minimumVersion":"2.1.100"}"#);
            let prev = change_channel("latest", Some("2.1.220"), false).unwrap();
            assert_eq!(prev, "stable");
            let v = settings_json();
            assert_eq!(v[CHANNEL_KEY], "latest");
            assert!(v.get(MINIMUM_VERSION_KEY).is_none());
        });
    }

    /// An unset channel reads as `latest`, so "switch to latest" is a no-op —
    /// and a no-op must not rewrite the file.
    #[test]
    fn switching_to_the_channel_already_in_effect_writes_nothing() {
        with_temp_config(|| {
            seed(r#"{"minimumVersion":"2.1.100"}"#);
            let before = std::fs::read(settings_path()).unwrap();
            let prev = change_channel("latest", Some("2.1.220"), false).unwrap();
            assert_eq!(prev, "latest");
            assert_eq!(
                std::fs::read(settings_path()).unwrap(),
                before,
                "a no-op transition must leave the file byte-identical"
            );
        });
    }

    #[test]
    fn an_unknown_channel_is_refused_before_anything_is_written() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"latest"}"#);
            let before = std::fs::read(settings_path()).unwrap();
            assert!(change_channel("nightly", Some("2.1.220"), false).is_err());
            assert_eq!(std::fs::read(settings_path()).unwrap(), before);
        });
    }

    /// A channel name we do not have rules for gets the plain write and no
    /// minimumVersion gymnastics — we should not guess at a policy.
    #[test]
    fn a_channel_we_do_not_recognize_in_settings_gets_the_simple_write() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"nightly","minimumVersion":"2.1.100"}"#);
            let prev = change_channel("stable", Some("2.1.220"), false).unwrap();
            assert_eq!(prev, "nightly");
            let v = settings_json();
            assert_eq!(v[CHANNEL_KEY], "stable");
            assert_eq!(
                v[MINIMUM_VERSION_KEY], "2.1.100",
                "an unrecognized source channel must not move the floor"
            );
        });
    }

    /// Both keys move in ONE write. Asserted by watching the file: a
    /// two-write implementation leaves an observable intermediate state.
    #[test]
    fn a_transition_moves_both_keys_in_a_single_write() {
        with_temp_config(|| {
            seed(r#"{"autoUpdatesChannel":"stable","minimumVersion":"2.1.100"}"#);
            change_channel("latest", Some("2.1.220"), false).unwrap();
            let v = settings_json();
            // If the channel landed while the floor was still set, or the
            // floor cleared while the channel was still stable, this is the
            // half-applied state the single mutation exists to prevent.
            assert_eq!(v[CHANNEL_KEY], "latest");
            assert!(v.get(MINIMUM_VERSION_KEY).is_none());
        });
    }

    #[test]
    fn a_malformed_settings_file_errors_rather_than_being_clobbered() {
        with_temp_config(|| {
            seed("{ not json");
            assert!(change_channel("stable", Some("2.1.220"), false).is_err());
            assert_eq!(std::fs::read(settings_path()).unwrap(), b"{ not json");
        });
    }
}
