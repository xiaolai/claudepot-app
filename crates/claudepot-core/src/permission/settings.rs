//! Read / resolve / write CC's `permissions.defaultMode` setting.
//!
//! `permissions.defaultMode` is a *nested* string key (inside the
//! `permissions` object), unlike `settings_writer`'s top-level
//! boolean `autoMemoryEnabled`. The layering chain is the same, so we
//! reuse [`SettingsLayer`]; the read/write helpers here are
//! nested-key-aware and preserve every sibling key.
//!
//! Verified against `~/github/claude_code_src/src/utils/settings/types.ts`
//! (`PermissionsSchema`) and `permissionSetup.ts:743`
//! (`settings.permissions?.defaultMode`).

use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;

use crate::fs_utils::atomic_write;
use crate::permission::mode::PermissionMode;
use crate::settings_writer::SettingsLayer;

/// The `permissions` object key in CC's settings JSON.
pub const PERMISSIONS_KEY: &str = "permissions";
/// The nested key under `permissions` carrying the default mode.
pub const DEFAULT_MODE_KEY: &str = "defaultMode";

#[derive(Debug, thiserror::Error)]
pub enum PermissionSettingsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("settings file is not a JSON object at {0}")]
    NotAJsonObject(PathBuf),
    #[error("`permissions` is present but not a JSON object at {0}")]
    PermissionsNotAnObject(PathBuf),
    #[error("write to {layer:?} is not supported (commit-bound or admin-managed)")]
    UnsupportedLayer { layer: SettingsLayer },
}

/// Where the effective `permissions.defaultMode` came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionDecisionSource {
    /// `<repo>/.claude/settings.local.json`.
    LocalProjectSettings,
    /// `<repo>/.claude/settings.json` (committed).
    ProjectSettings,
    /// `~/.claude/settings.json`.
    UserSettings,
    /// No layer set the key — CC's built-in default (`default`).
    Default,
}

/// Aggregate per-project permission state surfaced to the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionState {
    /// What CC will actually use for this project.
    pub effective: PermissionMode,
    /// Which layer decided `effective`.
    pub decided_by: PermissionDecisionSource,
    /// Per-layer raw values (`None` when the layer doesn't set it).
    pub user_value: Option<PermissionMode>,
    pub project_value: Option<PermissionMode>,
    pub local_project_value: Option<PermissionMode>,
}

/// Read `permissions.defaultMode` from one settings file. Missing
/// file / missing key / wrong type → `None`. A malformed file or a
/// `permissions` value that is not an object → `Err` (we never
/// silently treat an unreadable file as "not set").
pub fn read_default_mode(path: &Path) -> Result<Option<PermissionMode>, PermissionSettingsError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    let v: JsonValue = serde_json::from_slice(&bytes)?;
    let obj = v
        .as_object()
        .ok_or_else(|| PermissionSettingsError::NotAJsonObject(path.to_path_buf()))?;
    let permissions = match obj.get(PERMISSIONS_KEY) {
        None => return Ok(None),
        Some(JsonValue::Object(p)) => p,
        Some(JsonValue::Null) => return Ok(None),
        Some(_) => {
            return Err(PermissionSettingsError::PermissionsNotAnObject(
                path.to_path_buf(),
            ))
        }
    };
    Ok(permissions
        .get(DEFAULT_MODE_KEY)
        .and_then(JsonValue::as_str)
        .map(PermissionMode::from_wire_str))
}

/// Resolve the effective `permissions.defaultMode` for `project_root`
/// across the layering chain. Pure over the filesystem — no env vars
/// participate in this setting (unlike `autoMemoryEnabled`).
pub fn resolve_default_mode(project_root: &Path) -> PermissionState {
    let user_value =
        read_default_mode(&SettingsLayer::User.settings_file(project_root)).unwrap_or(None);
    let project_value =
        read_default_mode(&SettingsLayer::Project.settings_file(project_root)).unwrap_or(None);
    let local_project_value =
        read_default_mode(&SettingsLayer::LocalProject.settings_file(project_root)).unwrap_or(None);

    let (effective, decided_by) = if let Some(v) = local_project_value.clone() {
        (v, PermissionDecisionSource::LocalProjectSettings)
    } else if let Some(v) = project_value.clone() {
        (v, PermissionDecisionSource::ProjectSettings)
    } else if let Some(v) = user_value.clone() {
        (v, PermissionDecisionSource::UserSettings)
    } else {
        (PermissionMode::Default, PermissionDecisionSource::Default)
    };

    PermissionState {
        effective,
        decided_by,
        user_value,
        project_value,
        local_project_value,
    }
}

/// Read-modify-write `permissions.defaultMode` at `path`. Creates the
/// file (and the `permissions` object) if missing; preserves every
/// other top-level key and every sibling key inside `permissions`.
/// A malformed file errors rather than being clobbered.
fn rmw_set_default_mode(path: &Path, mode: &PermissionMode) -> Result<(), PermissionSettingsError> {
    let mut object = read_root_object(path)?;
    let permissions = upsert_permissions_object(&mut object, path)?;
    permissions.insert(
        DEFAULT_MODE_KEY.to_string(),
        JsonValue::String(mode.as_wire_str().to_string()),
    );
    write_root_object(path, object)
}

/// Read-modify-write removing `permissions.defaultMode` at `path`.
/// Missing file / missing key → no-op. An emptied `permissions`
/// object is left in place (an empty `{}` is harmless and avoids
/// guessing whether CC put it there).
fn rmw_remove_default_mode(path: &Path) -> Result<(), PermissionSettingsError> {
    let mut object = match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => return Ok(()),
        Ok(bytes) => match serde_json::from_slice::<JsonValue>(&bytes)? {
            JsonValue::Object(map) => map,
            _ => return Err(PermissionSettingsError::NotAJsonObject(path.to_path_buf())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let permissions = match object.get_mut(PERMISSIONS_KEY) {
        Some(JsonValue::Object(p)) => p,
        _ => return Ok(()),
    };
    if permissions.remove(DEFAULT_MODE_KEY).is_none() {
        return Ok(());
    }
    write_root_object(path, object)
}

fn read_root_object(
    path: &Path,
) -> Result<serde_json::Map<String, JsonValue>, PermissionSettingsError> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => Ok(serde_json::Map::new()),
        Ok(bytes) => match serde_json::from_slice::<JsonValue>(&bytes)? {
            JsonValue::Object(map) => Ok(map),
            _ => Err(PermissionSettingsError::NotAJsonObject(path.to_path_buf())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::Map::new()),
        Err(e) => Err(e.into()),
    }
}

fn upsert_permissions_object<'a>(
    object: &'a mut serde_json::Map<String, JsonValue>,
    path: &Path,
) -> Result<&'a mut serde_json::Map<String, JsonValue>, PermissionSettingsError> {
    let entry = object
        .entry(PERMISSIONS_KEY.to_string())
        .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
    match entry {
        JsonValue::Object(p) => Ok(p),
        // A `permissions: null` slot is safe to replace with an object.
        JsonValue::Null => {
            *entry = JsonValue::Object(serde_json::Map::new());
            match entry {
                JsonValue::Object(p) => Ok(p),
                _ => unreachable!("just assigned an object"),
            }
        }
        _ => Err(PermissionSettingsError::PermissionsNotAnObject(
            path.to_path_buf(),
        )),
    }
}

fn write_root_object(
    path: &Path,
    object: serde_json::Map<String, JsonValue>,
) -> Result<(), PermissionSettingsError> {
    let body = serde_json::to_string_pretty(&JsonValue::Object(object))?;
    let mut bytes = body.into_bytes();
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(path, &bytes)?;
    Ok(())
}

/// Set `permissions.defaultMode` at `layer` for `project_root`.
/// Refuses the committed `Project` layer — Claudepot grants always
/// land in `LocalProject` (per-machine, gitignored by convention).
pub fn write_default_mode(
    layer: SettingsLayer,
    project_root: &Path,
    mode: &PermissionMode,
) -> Result<(), PermissionSettingsError> {
    match layer {
        SettingsLayer::User | SettingsLayer::LocalProject => {
            rmw_set_default_mode(&layer.settings_file(project_root), mode)
        }
        SettingsLayer::Project => Err(PermissionSettingsError::UnsupportedLayer { layer }),
    }
}

/// Remove `permissions.defaultMode` at `layer` for `project_root`,
/// letting the next-higher layer (or CC's default) take over.
pub fn clear_default_mode(
    layer: SettingsLayer,
    project_root: &Path,
) -> Result<(), PermissionSettingsError> {
    match layer {
        SettingsLayer::User | SettingsLayer::LocalProject => {
            rmw_remove_default_mode(&layer.settings_file(project_root))
        }
        SettingsLayer::Project => Err(PermissionSettingsError::UnsupportedLayer { layer }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, PathBuf, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path().join("config-dir"));
        fs::create_dir_all(tmp.path().join("config-dir")).unwrap();
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();
        (tmp, project, lock)
    }

    #[test]
    fn default_when_nothing_set() {
        let (_t, project, _l) = isolated();
        let s = resolve_default_mode(&project);
        assert_eq!(s.effective, PermissionMode::Default);
        assert_eq!(s.decided_by, PermissionDecisionSource::Default);
        assert_eq!(s.local_project_value, None);
    }

    #[test]
    fn local_project_overrides_user_and_project() {
        let (_t, project, _l) = isolated();
        write_default_mode(SettingsLayer::User, &project, &PermissionMode::Plan).unwrap();
        // Project layer is hand-written (Claudepot won't write it).
        let proj_path = SettingsLayer::Project.settings_file(&project);
        fs::create_dir_all(proj_path.parent().unwrap()).unwrap();
        fs::write(
            &proj_path,
            br#"{"permissions":{"defaultMode":"acceptEdits"}}"#,
        )
        .unwrap();
        write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();

        let s = resolve_default_mode(&project);
        assert_eq!(s.effective, PermissionMode::BypassPermissions);
        assert_eq!(s.decided_by, PermissionDecisionSource::LocalProjectSettings);
        assert_eq!(s.user_value, Some(PermissionMode::Plan));
        assert_eq!(s.project_value, Some(PermissionMode::AcceptEdits));
        assert_eq!(
            s.local_project_value,
            Some(PermissionMode::BypassPermissions)
        );
    }

    #[test]
    fn write_preserves_other_top_level_and_permissions_siblings() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"model":"opus","permissions":{"allow":["Bash(ls)"],"defaultMode":"plan"}}"#,
        )
        .unwrap();

        write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();

        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], JsonValue::from("opus"));
        assert_eq!(after["permissions"]["allow"][0], JsonValue::from("Bash(ls)"));
        assert_eq!(
            after["permissions"]["defaultMode"],
            JsonValue::from("bypassPermissions")
        );
    }

    #[test]
    fn write_creates_file_and_permissions_object_when_missing() {
        let (_t, project, _l) = isolated();
        write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        assert!(path.exists());
        let v: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            v["permissions"]["defaultMode"],
            JsonValue::from("bypassPermissions")
        );
    }

    #[test]
    fn write_to_project_layer_is_unsupported() {
        let (_t, project, _l) = isolated();
        let err = write_default_mode(SettingsLayer::Project, &project, &PermissionMode::Plan)
            .unwrap_err();
        assert!(matches!(
            err,
            PermissionSettingsError::UnsupportedLayer {
                layer: SettingsLayer::Project
            }
        ));
    }

    #[test]
    fn clear_removes_only_default_mode_keeps_siblings() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"permissions":{"allow":["Bash(ls)"],"defaultMode":"bypassPermissions"}}"#,
        )
        .unwrap();

        clear_default_mode(SettingsLayer::LocalProject, &project).unwrap();

        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(after["permissions"]
            .as_object()
            .unwrap()
            .get("defaultMode")
            .is_none());
        assert_eq!(after["permissions"]["allow"][0], JsonValue::from("Bash(ls)"));
    }

    #[test]
    fn clear_on_missing_file_or_key_is_noop() {
        let (_t, project, _l) = isolated();
        clear_default_mode(SettingsLayer::LocalProject, &project).unwrap();
        assert!(!SettingsLayer::LocalProject.settings_file(&project).exists());

        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"model":"opus"}"#).unwrap();
        clear_default_mode(SettingsLayer::LocalProject, &project).unwrap();
        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], JsonValue::from("opus"));
    }

    #[test]
    fn malformed_file_errors_rather_than_clobbering() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ not valid json").unwrap();

        let err = write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::BypassPermissions,
        )
        .unwrap_err();
        assert!(matches!(err, PermissionSettingsError::JsonParse(_)));
        assert_eq!(fs::read(&path).unwrap(), b"{ not valid json");
    }

    #[test]
    fn permissions_not_an_object_errors() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"permissions":"oops"}"#).unwrap();

        let read_err = read_default_mode(&path).unwrap_err();
        assert!(matches!(
            read_err,
            PermissionSettingsError::PermissionsNotAnObject(_)
        ));
        let write_err = write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::Plan,
        )
        .unwrap_err();
        assert!(matches!(
            write_err,
            PermissionSettingsError::PermissionsNotAnObject(_)
        ));
    }

    #[test]
    fn unknown_mode_round_trips_through_settings() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"permissions":{"defaultMode":"auto"}}"#).unwrap();
        let v = read_default_mode(&path).unwrap();
        assert_eq!(v, Some(PermissionMode::Unknown("auto".into())));
    }

    #[test]
    fn permissions_null_slot_is_treated_as_unset_and_replaced_on_write() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"permissions":null}"#).unwrap();
        assert_eq!(read_default_mode(&path).unwrap(), None);

        write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();
        assert_eq!(
            read_default_mode(&path).unwrap(),
            Some(PermissionMode::BypassPermissions)
        );
    }
}
