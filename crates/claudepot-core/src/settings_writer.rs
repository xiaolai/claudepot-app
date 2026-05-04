//! Read + write CC's `settings.json` family for the auto-memory toggle.
//!
//! CC layers four-plus sources for boolean preferences. For
//! `autoMemoryEnabled` the priority chain (verified against
//! `~/github/claude_code_src/src/memdir/paths.ts:30 isAutoMemoryEnabled`)
//! is, first match wins:
//!
//! 1. `CLAUDE_CODE_DISABLE_AUTO_MEMORY` env var (truthy → disabled)
//! 2. `CLAUDE_CODE_SIMPLE` env var (truthy → disabled)
//! 3. CCR remote without `CLAUDE_CODE_REMOTE_MEMORY_DIR` (skipped here —
//!    we don't run inside CCR; document and ignore)
//! 4. `autoMemoryEnabled` in settings.json, layered:
//!    - `policySettings` (MDM / managed; not writable from a UI)
//!    - `flagSettings` (CLI `--settings`; not in scope here)
//!    - `localProjectSettings` (`<repo>/.claude/settings.local.json`)
//!    - `projectSettings` (`<repo>/.claude/settings.json`)
//!    - `userSettings` (`~/.claude/settings.json`)
//! 5. Default: enabled.
//!
//! For writing, we only touch `userSettings` (global toggle) and
//! `localProjectSettings` (per-project, per-machine). `projectSettings`
//! is committed to the repo and a UI write would land in someone's
//! commit; we refuse to write there. `policySettings` belongs to the
//! org admin.
//!
//! All writes are JSON read-modify-write — `serde_json::Value`
//! preserves keys we don't know about, then `fs_utils::atomic_write`
//! lands the result.

use crate::fs_utils::atomic_write;
use crate::paths::claude_config_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

/// Setting key in CC's `settings.json` for auto-memory.
pub const AUTO_MEMORY_KEY: &str = "autoMemoryEnabled";

/// Where a particular settings value came from. Mirrors CC's
/// SettingSource enum but with only the layers we read or write here.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SettingsLayer {
    /// `~/.claude/settings.json`. Writable by Claudepot.
    User,
    /// `<repo>/.claude/settings.json`. Read-only from Claudepot's
    /// perspective — committed to the repo.
    Project,
    /// `<repo>/.claude/settings.local.json`. Writable by Claudepot;
    /// gitignored by convention.
    LocalProject,
}

impl SettingsLayer {
    pub fn settings_file(self, project_root: &Path) -> PathBuf {
        match self {
            Self::User => claude_config_dir().join("settings.json"),
            Self::Project => project_root.join(".claude").join("settings.json"),
            Self::LocalProject => project_root.join(".claude").join("settings.local.json"),
        }
    }
}

/// Why CC will (or won't) auto-memory for a given project.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AutoMemoryDecisionSource {
    /// `CLAUDE_CODE_DISABLE_AUTO_MEMORY` truthy → disabled.
    EnvDisable,
    /// `CLAUDE_CODE_SIMPLE` truthy → disabled.
    EnvSimple,
    /// `<repo>/.claude/settings.local.json :: autoMemoryEnabled`.
    LocalProjectSettings,
    /// `<repo>/.claude/settings.json :: autoMemoryEnabled`.
    ProjectSettings,
    /// `~/.claude/settings.json :: autoMemoryEnabled`.
    UserSettings,
    /// No source set the key — CC's default (enabled) wins.
    Default,
}

/// Aggregate state surfaced by the toggle UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoMemoryState {
    /// What CC will actually do for this project.
    pub effective: bool,
    /// Why `effective` is what it is.
    pub decided_by: AutoMemoryDecisionSource,
    /// `false` when an env var or SIMPLE flag is overriding all
    /// settings layers — the toggle renders disabled with a reason.
    pub user_writable: bool,
    /// Per-source values. Each is `Some(true/false)` when the layer
    /// has the key, `None` when absent or invalid.
    pub user_settings_value: Option<bool>,
    pub project_settings_value: Option<bool>,
    pub local_project_settings_value: Option<bool>,
    /// Whether the disabling env vars are detected. Surfaced for UX.
    pub env_disable_set: bool,
    pub env_simple_set: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsWriteError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("settings file is not a JSON object at {0}")]
    NotAJsonObject(PathBuf),
    #[error("write to {layer:?} is not supported (commit-bound or admin-managed)")]
    UnsupportedLayer { layer: SettingsLayer },
}

/// Truthy/falsy parser matching CC's `isEnvTruthy` / `isEnvDefinedFalsy`
/// (`utils/envUtils.ts`) — we accept the same `1/true/yes/on` and
/// `0/false/no/off` forms.
fn env_is_truthy(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn env_is_falsy(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::to_ascii_lowercase).as_deref(),
        Some("0" | "false" | "no" | "off")
    )
}

/// Read one setting from a JSON file. Missing file → `None`. Missing
/// key → `None`. Wrong type for the key → `None` (treated as "not
/// set" rather than erroring; CC does the same coercion).
pub fn read_bool_setting(path: &Path, key: &str) -> Result<Option<bool>, SettingsWriteError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    let v: JsonValue = serde_json::from_slice(&bytes)?;
    Ok(v.as_object()
        .and_then(|o| o.get(key))
        .and_then(JsonValue::as_bool))
}

/// Resolve `autoMemoryEnabled` for the global scope only. Reads env
/// vars + `~/.claude/settings.json` and ignores the project-scoped
/// layers entirely. Use this when the caller has no real project
/// anchor (Settings → General global toggle); the per-project
/// resolver feeds the home dir as project_root, which then collapses
/// `userSettings` and `projectSettings` onto the same file (audit
/// 2026-05 #3).
pub fn resolve_auto_memory_enabled_global() -> AutoMemoryState {
    let env_disable_raw = std::env::var("CLAUDE_CODE_DISABLE_AUTO_MEMORY").ok();
    let env_simple_raw = std::env::var("CLAUDE_CODE_SIMPLE").ok();
    let env_disable_set = env_is_truthy(env_disable_raw.as_deref());
    let env_simple_set = env_is_truthy(env_simple_raw.as_deref());
    let env_disable_explicit_off = env_is_falsy(env_disable_raw.as_deref());

    let user_value =
        read_bool_setting(&claude_config_dir().join("settings.json"), AUTO_MEMORY_KEY)
            .unwrap_or(None);

    if env_disable_set {
        return AutoMemoryState {
            effective: false,
            decided_by: AutoMemoryDecisionSource::EnvDisable,
            user_writable: false,
            user_settings_value: user_value,
            project_settings_value: None,
            local_project_settings_value: None,
            env_disable_set,
            env_simple_set,
        };
    }
    if env_disable_explicit_off {
        return AutoMemoryState {
            effective: true,
            decided_by: AutoMemoryDecisionSource::EnvDisable,
            user_writable: false,
            user_settings_value: user_value,
            project_settings_value: None,
            local_project_settings_value: None,
            env_disable_set,
            env_simple_set,
        };
    }
    if env_simple_set {
        return AutoMemoryState {
            effective: false,
            decided_by: AutoMemoryDecisionSource::EnvSimple,
            user_writable: false,
            user_settings_value: user_value,
            project_settings_value: None,
            local_project_settings_value: None,
            env_disable_set,
            env_simple_set,
        };
    }
    if let Some(v) = user_value {
        return AutoMemoryState {
            effective: v,
            decided_by: AutoMemoryDecisionSource::UserSettings,
            user_writable: true,
            user_settings_value: user_value,
            project_settings_value: None,
            local_project_settings_value: None,
            env_disable_set,
            env_simple_set,
        };
    }
    AutoMemoryState {
        effective: true,
        decided_by: AutoMemoryDecisionSource::Default,
        user_writable: true,
        user_settings_value: user_value,
        project_settings_value: None,
        local_project_settings_value: None,
        env_disable_set,
        env_simple_set,
    }
}

/// Read the full `autoMemoryEnabled` resolution for `project_root`.
/// Pure function over env + filesystem state — no side effects.
pub fn resolve_auto_memory_enabled(project_root: &Path) -> AutoMemoryState {
    let env_disable_raw = std::env::var("CLAUDE_CODE_DISABLE_AUTO_MEMORY").ok();
    let env_simple_raw = std::env::var("CLAUDE_CODE_SIMPLE").ok();
    let env_disable_set = env_is_truthy(env_disable_raw.as_deref());
    let env_simple_set = env_is_truthy(env_simple_raw.as_deref());
    let env_disable_explicit_off = env_is_falsy(env_disable_raw.as_deref());

    let user_value = read_bool_setting(
        &SettingsLayer::User.settings_file(project_root),
        AUTO_MEMORY_KEY,
    )
    .unwrap_or(None);
    let project_value = read_bool_setting(
        &SettingsLayer::Project.settings_file(project_root),
        AUTO_MEMORY_KEY,
    )
    .unwrap_or(None);
    let local_project_value = read_bool_setting(
        &SettingsLayer::LocalProject.settings_file(project_root),
        AUTO_MEMORY_KEY,
    )
    .unwrap_or(None);

    // Env priority — exactly as CC.
    if env_disable_set {
        return AutoMemoryState {
            effective: false,
            decided_by: AutoMemoryDecisionSource::EnvDisable,
            user_writable: false,
            user_settings_value: user_value,
            project_settings_value: project_value,
            local_project_settings_value: local_project_value,
            env_disable_set,
            env_simple_set,
        };
    }
    // CC: "If isEnvDefinedFalsy(CLAUDE_CODE_DISABLE_AUTO_MEMORY) → return true"
    // — this short-circuits SIMPLE + the rest of the chain too.
    if env_disable_explicit_off {
        return AutoMemoryState {
            effective: true,
            decided_by: AutoMemoryDecisionSource::EnvDisable,
            user_writable: false,
            user_settings_value: user_value,
            project_settings_value: project_value,
            local_project_settings_value: local_project_value,
            env_disable_set,
            env_simple_set,
        };
    }
    if env_simple_set {
        return AutoMemoryState {
            effective: false,
            decided_by: AutoMemoryDecisionSource::EnvSimple,
            user_writable: false,
            user_settings_value: user_value,
            project_settings_value: project_value,
            local_project_settings_value: local_project_value,
            env_disable_set,
            env_simple_set,
        };
    }

    // Settings layering (most-specific wins).
    if let Some(v) = local_project_value {
        return AutoMemoryState {
            effective: v,
            decided_by: AutoMemoryDecisionSource::LocalProjectSettings,
            user_writable: true,
            user_settings_value: user_value,
            project_settings_value: project_value,
            local_project_settings_value: local_project_value,
            env_disable_set,
            env_simple_set,
        };
    }
    if let Some(v) = project_value {
        return AutoMemoryState {
            effective: v,
            decided_by: AutoMemoryDecisionSource::ProjectSettings,
            // Project settings exist but are committed to the repo —
            // a Claudepot toggle still writes (to LocalProject), it
            // just overrides the project value.
            user_writable: true,
            user_settings_value: user_value,
            project_settings_value: project_value,
            local_project_settings_value: local_project_value,
            env_disable_set,
            env_simple_set,
        };
    }
    if let Some(v) = user_value {
        return AutoMemoryState {
            effective: v,
            decided_by: AutoMemoryDecisionSource::UserSettings,
            user_writable: true,
            user_settings_value: user_value,
            project_settings_value: project_value,
            local_project_settings_value: local_project_value,
            env_disable_set,
            env_simple_set,
        };
    }
    AutoMemoryState {
        effective: true,
        decided_by: AutoMemoryDecisionSource::Default,
        user_writable: true,
        user_settings_value: user_value,
        project_settings_value: project_value,
        local_project_settings_value: local_project_value,
        env_disable_set,
        env_simple_set,
    }
}

/// Read-modify-write a settings file, setting `key` to `value`.
/// Preserves all unknown keys. If the file is missing, creates it
/// with just `{ key: value }`. If the file is malformed JSON, the
/// caller gets `SettingsWriteError::JsonParse` — we never silently
/// overwrite a file we couldn't parse.
fn rmw_settings_bool(
    path: &Path,
    key: &str,
    value: bool,
) -> Result<(), SettingsWriteError> {
    let mut object = match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => serde_json::Map::new(),
        Ok(bytes) => {
            let v: JsonValue = serde_json::from_slice(&bytes)?;
            match v {
                JsonValue::Object(map) => map,
                _ => return Err(SettingsWriteError::NotAJsonObject(path.to_path_buf())),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(e) => return Err(e.into()),
    };
    object.insert(key.to_string(), JsonValue::Bool(value));
    let body = serde_json::to_string_pretty(&JsonValue::Object(object))?;
    let mut bytes = body.into_bytes();
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(path, &bytes)?;
    Ok(())
}

/// Remove `key` from the settings file. If the file is missing or the
/// key is absent, this is a no-op. Used to clear an override.
fn rmw_settings_remove(path: &Path, key: &str) -> Result<(), SettingsWriteError> {
    let mut object = match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => return Ok(()),
        Ok(bytes) => {
            let v: JsonValue = serde_json::from_slice(&bytes)?;
            match v {
                JsonValue::Object(map) => map,
                _ => return Err(SettingsWriteError::NotAJsonObject(path.to_path_buf())),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if object.remove(key).is_none() {
        return Ok(());
    }
    let body = serde_json::to_string_pretty(&JsonValue::Object(object))?;
    let mut bytes = body.into_bytes();
    bytes.push(b'\n');
    atomic_write(path, &bytes)?;
    Ok(())
}

/// Set `autoMemoryEnabled` at the given layer. Refuses to write to
/// `Project` (committed file) or any other unsupported layer.
pub fn write_auto_memory_enabled(
    layer: SettingsLayer,
    project_root: &Path,
    value: bool,
) -> Result<(), SettingsWriteError> {
    match layer {
        SettingsLayer::User | SettingsLayer::LocalProject => {
            let path = layer.settings_file(project_root);
            rmw_settings_bool(&path, AUTO_MEMORY_KEY, value)
        }
        SettingsLayer::Project => Err(SettingsWriteError::UnsupportedLayer { layer }),
    }
}

/// Clear `autoMemoryEnabled` at the given layer (the key is removed,
/// not set to a default). Lets the next-higher layer take over the
/// decision.
pub fn clear_auto_memory_enabled(
    layer: SettingsLayer,
    project_root: &Path,
) -> Result<(), SettingsWriteError> {
    match layer {
        SettingsLayer::User | SettingsLayer::LocalProject => {
            let path = layer.settings_file(project_root);
            rmw_settings_remove(&path, AUTO_MEMORY_KEY)
        }
        SettingsLayer::Project => Err(SettingsWriteError::UnsupportedLayer { layer }),
    }
}

/// Whether the project's `.gitignore` covers
/// `.claude/settings.local.json`. Returns `Ok(true)` if the gitignore
/// exists and contains a matching pattern; `Ok(false)` if the file
/// exists but lacks coverage. `Err` only on real I/O failure (perm
/// denied) — a missing gitignore is `Ok(false)`. Pattern match is
/// substring-based; we don't pretend to evaluate gitignore globs.
pub fn local_settings_is_gitignored(project_root: &Path) -> std::io::Result<bool> {
    let path = project_root.join(".gitignore");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let text = String::from_utf8_lossy(&bytes);
    // Patterns we accept as "covered enough":
    //   - `.claude/settings.local.json` (exact)
    //   - `.claude/*.local.json`
    //   - `**/*.local.json`
    //   - `*.local.json`
    //   - `settings.local.json`
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l == ".claude/settings.local.json"
            || l == "settings.local.json"
            || l == "*.local.json"
            || l == "**/*.local.json"
            || l == ".claude/*.local.json"
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Set up an isolated test environment: tempdir → CLAUDE_CONFIG_DIR
    /// + a project root. The data-dir lock prevents env races between
    /// parallel tests.
    fn isolated() -> (TempDir, PathBuf, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path().join("config-dir"));
        std::fs::create_dir_all(tmp.path().join("config-dir")).unwrap();
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();
        (tmp, project, lock)
    }

    #[test]
    fn default_when_nothing_set() {
        let (_t, project, _l) = isolated();
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
        let s = resolve_auto_memory_enabled(&project);
        assert!(s.effective);
        assert_eq!(s.decided_by, AutoMemoryDecisionSource::Default);
        assert!(s.user_writable);
    }

    #[test]
    fn env_disable_truthy_wins() {
        let (_t, project, _l) = isolated();
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
        let s = resolve_auto_memory_enabled(&project);
        assert!(!s.effective);
        assert_eq!(s.decided_by, AutoMemoryDecisionSource::EnvDisable);
        assert!(!s.user_writable);
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
    }

    #[test]
    fn env_simple_disables_unless_explicit_off_on_other_var() {
        let (_t, project, _l) = isolated();
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::set_var("CLAUDE_CODE_SIMPLE", "1");
        let s = resolve_auto_memory_enabled(&project);
        assert!(!s.effective);
        assert_eq!(s.decided_by, AutoMemoryDecisionSource::EnvSimple);
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
    }

    #[test]
    fn env_disable_falsy_overrides_simple_to_enabled() {
        let (_t, project, _l) = isolated();
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "0");
        std::env::set_var("CLAUDE_CODE_SIMPLE", "1");
        let s = resolve_auto_memory_enabled(&project);
        assert!(s.effective);
        assert_eq!(s.decided_by, AutoMemoryDecisionSource::EnvDisable);
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
    }

    #[test]
    fn local_project_overrides_user_setting() {
        let (_t, project, _l) = isolated();
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");

        write_auto_memory_enabled(SettingsLayer::User, &project, true).unwrap();
        write_auto_memory_enabled(SettingsLayer::LocalProject, &project, false).unwrap();

        let s = resolve_auto_memory_enabled(&project);
        assert!(!s.effective);
        assert_eq!(s.decided_by, AutoMemoryDecisionSource::LocalProjectSettings);
    }

    #[test]
    fn write_preserves_unknown_keys() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"unrelatedKey":42,"nested":{"keep":"me"}}"#,
        )
        .unwrap();

        write_auto_memory_enabled(SettingsLayer::User, &project, false).unwrap();

        let after: JsonValue =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["autoMemoryEnabled"], JsonValue::Bool(false));
        assert_eq!(after["unrelatedKey"], JsonValue::from(42));
        assert_eq!(after["nested"]["keep"], JsonValue::from("me"));
    }

    #[test]
    fn write_creates_parent_directory_if_missing() {
        let (_t, project, _l) = isolated();
        // .claude/ dir doesn't exist yet — write must create it.
        write_auto_memory_enabled(SettingsLayer::LocalProject, &project, true).unwrap();
        let p = SettingsLayer::LocalProject.settings_file(&project);
        assert!(p.exists());
        let v: JsonValue = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        assert_eq!(v["autoMemoryEnabled"], JsonValue::Bool(true));
    }

    #[test]
    fn write_to_project_layer_is_unsupported() {
        let (_t, project, _l) = isolated();
        let err = write_auto_memory_enabled(SettingsLayer::Project, &project, false).unwrap_err();
        match err {
            SettingsWriteError::UnsupportedLayer { layer: SettingsLayer::Project } => {}
            other => panic!("expected UnsupportedLayer(Project), got {:?}", other),
        }
    }

    #[test]
    fn malformed_settings_file_errors_rather_than_clobbering() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ this is not valid json").unwrap();

        let err = write_auto_memory_enabled(SettingsLayer::User, &project, true).unwrap_err();
        match err {
            SettingsWriteError::JsonParse(_) => {}
            other => panic!("expected JsonParse, got {:?}", other),
        }
        // Original bytes should be untouched.
        let after = fs::read(&path).unwrap();
        assert_eq!(after, b"{ this is not valid json");
    }

    #[test]
    fn clear_removes_key_keeps_rest() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"autoMemoryEnabled":false,"keep":1}"#).unwrap();

        clear_auto_memory_enabled(SettingsLayer::LocalProject, &project).unwrap();

        let after: JsonValue =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(after.as_object().unwrap().get("autoMemoryEnabled").is_none());
        assert_eq!(after["keep"], JsonValue::from(1));
    }

    #[test]
    fn clear_on_missing_file_is_noop() {
        let (_t, project, _l) = isolated();
        // No settings file exists.
        clear_auto_memory_enabled(SettingsLayer::LocalProject, &project).unwrap();
        assert!(!SettingsLayer::LocalProject.settings_file(&project).exists());
    }

    #[test]
    fn gitignore_detection_recognizes_common_patterns() {
        let (_t, project, _l) = isolated();
        fs::write(project.join(".gitignore"), "node_modules/\n*.local.json\n").unwrap();
        assert!(local_settings_is_gitignored(&project).unwrap());

        fs::write(project.join(".gitignore"), "node_modules/\n").unwrap();
        assert!(!local_settings_is_gitignored(&project).unwrap());

        fs::remove_file(project.join(".gitignore")).unwrap();
        assert!(!local_settings_is_gitignored(&project).unwrap());
    }
}
