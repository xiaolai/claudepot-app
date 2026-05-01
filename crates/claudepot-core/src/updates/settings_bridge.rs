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

fn settings_path() -> PathBuf {
    paths::claude_config_dir().join("settings.json")
}

fn read_root() -> Result<Map<String, Value>> {
    let p = settings_path();
    if !p.exists() {
        return Ok(Map::new());
    }
    let body = std::fs::read_to_string(&p)?;
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

fn write_root(root: &Map<String, Value>) -> Result<()> {
    let p = settings_path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(root)?;
    let parent = p.parent().unwrap_or(std::path::Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    std::fs::write(tmp.path(), body)?;
    tmp.persist(&p).map_err(|e| {
        UpdateError::Io(std::io::Error::other(format!(
            "settings_bridge: persist {}: {}",
            p.display(),
            e
        )))
    })?;
    Ok(())
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
        .get("autoUpdatesChannel")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let minimum_version = root
        .get("minimumVersion")
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
    let mut root = read_root()?;
    match channel {
        Some(c) => {
            root.insert(
                "autoUpdatesChannel".to_string(),
                Value::String(c.to_string()),
            );
        }
        None => {
            root.remove("autoUpdatesChannel");
        }
    }
    write_root(&root)
}

/// Write `minimumVersion`. Pass `None` to clear the floor.
///
/// Same failure-mode contract as [`write_channel`]: malformed file →
/// error, never destructive overwrite.
pub fn write_minimum_version(version: Option<&str>) -> Result<()> {
    let mut root = read_root()?;
    match version {
        Some(v) => {
            root.insert("minimumVersion".to_string(), Value::String(v.to_string()));
        }
        None => {
            root.remove("minimumVersion");
        }
    }
    write_root(&root)
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
    let current = read()?;
    let prev_channel = current
        .auto_updates_channel
        .clone()
        .unwrap_or_else(|| "latest".to_string());

    if prev_channel == new_channel {
        return Ok(prev_channel);
    }

    match (prev_channel.as_str(), new_channel) {
        ("latest", "stable") => {
            if allow_downgrade {
                // User explicitly accepted downgrade — clear any
                // pre-existing floor so stable can land at whatever
                // it is right now.
                write_minimum_version(None)?;
            } else if let Some(v) = installed_version {
                // Default: pin to current to avoid an involuntary
                // downgrade.
                write_minimum_version(Some(v))?;
            }
            write_channel(Some("stable"))?;
        }
        ("stable", "latest") => {
            write_minimum_version(None)?;
            write_channel(Some("latest"))?;
        }
        // Anything we didn't enumerate (e.g., a future channel name
        // already in settings.json) gets the simple write — no
        // minimumVersion gymnastics, since we don't know the rules.
        _ => {
            write_channel(Some(new_channel))?;
        }
    }
    Ok(prev_channel)
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
}
