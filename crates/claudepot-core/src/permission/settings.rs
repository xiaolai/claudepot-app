//! Read / resolve / write CC's `permissions.defaultMode` setting.
//!
//! `permissions.defaultMode` is a *nested* string key (inside the
//! `permissions` object), unlike `settings_writer`'s top-level
//! boolean `autoMemoryEnabled`. The layering chain is the same, so we
//! reuse [`SettingsLayer`]; the read/write helpers here are
//! nested-key-aware and preserve every sibling key.
//!
//! The key and its values are verified against CC's published settings
//! reference (`code.claude.com/docs/en/settings-reference`) and the
//! 2.1.259 binary (2026-09-03), not the abandoned source mirror. The
//! one rule that is not "most specific layer wins" is
//! [`PROJECT_SCOPE_IGNORES_SINCE`].

use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;

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
    /// Another writer — Claude Code, or a text editor — kept moving the file
    /// while we were rebasing onto it. See [`crate::settings_mutex`].
    #[error("{path} is being written by something else; try again")]
    Contended { path: PathBuf },
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl crate::error_code::ErrorCode for PermissionSettingsError {
    fn code(&self) -> &'static str {
        match self {
            PermissionSettingsError::Io(_) => "permission_settings.io",
            PermissionSettingsError::JsonParse(_) => "permission_settings.json_parse",
            PermissionSettingsError::NotAJsonObject(_) => "permission_settings.not_a_json_object",
            PermissionSettingsError::PermissionsNotAnObject(_) => {
                "permission_settings.permissions_not_an_object"
            }
            PermissionSettingsError::UnsupportedLayer { .. } => {
                "permission_settings.unsupported_layer"
            }
            PermissionSettingsError::Contended { .. } => "permission_settings.contended",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            PermissionSettingsError::Io(e) => serde_json::json!({ "detail": e.to_string() }),
            PermissionSettingsError::JsonParse(e) => serde_json::json!({ "detail": e.to_string() }),
            // Settings-file paths. This module reads and rewrites one
            // nested string key; no setting *value* is ever quoted into
            // an error, which is what keeps these safe to structure.
            PermissionSettingsError::NotAJsonObject(path)
            | PermissionSettingsError::PermissionsNotAnObject(path) => {
                serde_json::json!({ "path": path.display().to_string() })
            }
            // `layer` is the `SettingsLayer` variant name, matching the
            // `{layer:?}` the English message prints — a stable token a
            // catalog can switch on.
            PermissionSettingsError::UnsupportedLayer { layer } => {
                serde_json::json!({ "layer": format!("{layer:?}") })
            }
            PermissionSettingsError::Contended { path } => {
                serde_json::json!({ "path": path.display().to_string() })
            }
        }
    }
}

/// Grants land in the same `settings.local.json` that `settings_writer`
/// edits, so this module writes through the shared mutation boundary too;
/// map its failures onto the shape callers already match on.
impl From<crate::settings_mutex::SettingsMutexError> for PermissionSettingsError {
    fn from(e: crate::settings_mutex::SettingsMutexError) -> Self {
        use crate::settings_mutex::SettingsMutexError as S;
        match e {
            S::Io(e) => Self::Io(e),
            S::JsonParse(e) => Self::JsonParse(e),
            S::NotAJsonObject(p) => Self::NotAJsonObject(p),
            S::Contended { path, .. } => Self::Contended { path },
        }
    }
}

/// First Claude Code release that ignores `bypassPermissions` when it
/// comes from `.claude/settings.json` or `.claude/settings.local.json`.
///
/// CC's changelog for 2.1.257: *"Changed `defaultMode:
/// "bypassPermissions"` in `.claude/settings.json` or
/// `.claude/settings.local.json` to be ignored, like `"auto"`; set it
/// in user or managed settings, or pass `--permission-mode`."* The
/// binary's own log line names the reason: *"only policy/user/flag
/// settings may grant bypass mode (projectSettings and localSettings
/// are repo-controllable)"*. The session starts in Manual, and CC does
/// **not** fall through to a user-settings value — the settings
/// reference says it "uses the built-in default rather than a
/// `defaultMode` from `~/.claude/settings.json`".
///
/// This is the pin for [`resolve_default_mode`]'s ignore rule and the
/// version the pane quotes. Re-check against the `permissions.defaultMode`
/// row in `crates/xtask/cc-upstream-watch.md`.
pub const PROJECT_SCOPE_IGNORES_SINCE: &str = "2.1.257";

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
    /// A project-scope file set a value CC refuses from that scope
    /// (`bypassPermissions` or `auto`, since
    /// [`PROJECT_SCOPE_IGNORES_SINCE`]). CC starts the session in its
    /// built-in default and ignores the user layer too; the offending
    /// value is in [`PermissionState::ignored`].
    ProjectScopeIgnored,
}

/// A `defaultMode` value present on disk that CC will not honour
/// because of the file it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IgnoredValue {
    /// The project-scope layer carrying the value.
    pub layer: SettingsLayer,
    pub mode: PermissionMode,
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
    /// Set when the layer that would have won holds a value CC ignores
    /// from project scope. The pane renders it as a stale key with a
    /// repair action rather than as "elevated".
    pub ignored: Option<IgnoredValue>,
}

/// The two values CC refuses from `.claude/settings.json` and
/// `.claude/settings.local.json`. `acceptEdits`, `plan`, `dontAsk` and
/// `default`/`manual` are honoured from any file.
fn refused_from_project_scope(mode: &PermissionMode) -> bool {
    matches!(
        mode,
        PermissionMode::BypassPermissions | PermissionMode::Auto
    )
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
///
/// Mirrors CC ≥ [`PROJECT_SCOPE_IGNORES_SINCE`]: the most specific
/// layer wins, except that `bypassPermissions` / `auto` in a
/// project-scope file is dropped outright — the session starts in
/// CC's built-in default, and a user-layer value beneath it is *not*
/// consulted. Reporting the local value as effective there is exactly
/// how this pane showed "Bypass active" over a session that prompted
/// for everything.
pub fn resolve_default_mode(
    project_root: &Path,
) -> Result<PermissionState, PermissionSettingsError> {
    let user_value = read_default_mode(&SettingsLayer::User.settings_file(project_root))?;
    let project_value = read_default_mode(&SettingsLayer::Project.settings_file(project_root))?;
    let local_project_value =
        read_default_mode(&SettingsLayer::LocalProject.settings_file(project_root))?;

    let winning_project_scope = local_project_value
        .clone()
        .map(|v| (SettingsLayer::LocalProject, v))
        .or_else(|| project_value.clone().map(|v| (SettingsLayer::Project, v)));

    let (effective, decided_by, ignored) = match winning_project_scope {
        Some((layer, mode)) if refused_from_project_scope(&mode) => (
            PermissionMode::Default,
            PermissionDecisionSource::ProjectScopeIgnored,
            Some(IgnoredValue { layer, mode }),
        ),
        Some((SettingsLayer::LocalProject, mode)) => {
            (mode, PermissionDecisionSource::LocalProjectSettings, None)
        }
        Some((_, mode)) => (mode, PermissionDecisionSource::ProjectSettings, None),
        None => match user_value.clone() {
            Some(v) => (v, PermissionDecisionSource::UserSettings, None),
            None => (
                PermissionMode::Default,
                PermissionDecisionSource::Default,
                None,
            ),
        },
    };

    Ok(PermissionState {
        effective,
        decided_by,
        user_value,
        project_value,
        local_project_value,
        ignored,
    })
}

/// Read-modify-write `permissions.defaultMode` at `path`. Creates the
/// file (and the `permissions` object) if missing; preserves every
/// other top-level key and every sibling key inside `permissions`.
/// A malformed file errors rather than being clobbered.
fn rmw_set_default_mode(path: &Path, mode: &PermissionMode) -> Result<(), PermissionSettingsError> {
    crate::settings_mutex::mutate_settings_file(path, |object, _| {
        let permissions = upsert_permissions_object(object, path)?;
        permissions.insert(
            DEFAULT_MODE_KEY.to_string(),
            JsonValue::String(mode.as_wire_str().to_string()),
        );
        Ok::<_, PermissionSettingsError>(crate::settings_mutex::Change::Write(()))
    })
    .map(|_| ())
}

/// Read-modify-write removing `permissions.defaultMode` at `path`.
/// Missing file / missing key → no-op. An emptied `permissions`
/// object is left in place (an empty `{}` is harmless and avoids
/// guessing whether CC put it there).
fn rmw_remove_default_mode(path: &Path) -> Result<(), PermissionSettingsError> {
    crate::settings_mutex::mutate_settings_file(path, |object, was| {
        if !was.is_present() {
            return Ok(crate::settings_mutex::Change::Skip(()));
        }
        let permissions = match object.get_mut(PERMISSIONS_KEY) {
            Some(JsonValue::Object(p)) => p,
            // Genuinely nothing to remove.
            None | Some(JsonValue::Null) => return Ok(crate::settings_mutex::Change::Skip(())),
            // `permissions` exists but is not an object. Reporting success
            // here would tell the orchestrator the grant was reverted and
            // let it drop the grant record, leaving the project elevated
            // with nothing left that knows to revert it. `read_default_mode`
            // already errors on this shape; the clear path has to agree.
            Some(_) => {
                return Err(PermissionSettingsError::PermissionsNotAnObject(
                    path.to_path_buf(),
                ))
            }
        };
        if permissions.remove(DEFAULT_MODE_KEY).is_none() {
            return Ok(crate::settings_mutex::Change::Skip(()));
        }
        Ok::<_, PermissionSettingsError>(crate::settings_mutex::Change::Write(()))
    })
    .map(|_| ())
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
        let s = resolve_default_mode(&project).unwrap();
        assert_eq!(s.effective, PermissionMode::Default);
        assert_eq!(s.decided_by, PermissionDecisionSource::Default);
        assert_eq!(s.local_project_value, None);
    }

    fn write_project_layer(project: &Path, mode: &str) {
        // Project layer is hand-written (Claudepot won't write it).
        let proj_path = SettingsLayer::Project.settings_file(project);
        fs::create_dir_all(proj_path.parent().unwrap()).unwrap();
        fs::write(
            &proj_path,
            format!(r#"{{"permissions":{{"defaultMode":"{mode}"}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn local_project_overrides_user_and_project_for_honoured_values() {
        let (_t, project, _l) = isolated();
        write_default_mode(SettingsLayer::User, &project, &PermissionMode::Plan).unwrap();
        write_project_layer(&project, "acceptEdits");
        write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::DontAsk,
        )
        .unwrap();

        let s = resolve_default_mode(&project).unwrap();
        assert_eq!(s.effective, PermissionMode::DontAsk);
        assert_eq!(s.decided_by, PermissionDecisionSource::LocalProjectSettings);
        assert_eq!(s.user_value, Some(PermissionMode::Plan));
        assert_eq!(s.project_value, Some(PermissionMode::AcceptEdits));
        assert_eq!(s.local_project_value, Some(PermissionMode::DontAsk));
        assert_eq!(s.ignored, None);
    }

    #[test]
    fn bypass_in_the_local_file_is_ignored_and_does_not_fall_through_to_user() {
        // CC ≥ 2.1.257. The session starts in Manual, and the user
        // layer's `plan` is NOT consulted — the settings reference says
        // CC "uses the built-in default rather than a defaultMode from
        // ~/.claude/settings.json". This is the state every Claudepot
        // grant written before schema 2 left behind.
        let (_t, project, _l) = isolated();
        write_default_mode(SettingsLayer::User, &project, &PermissionMode::Plan).unwrap();
        write_default_mode(
            SettingsLayer::LocalProject,
            &project,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();

        let s = resolve_default_mode(&project).unwrap();
        assert_eq!(s.effective, PermissionMode::Default);
        assert_eq!(s.decided_by, PermissionDecisionSource::ProjectScopeIgnored);
        assert_eq!(
            s.ignored,
            Some(IgnoredValue {
                layer: SettingsLayer::LocalProject,
                mode: PermissionMode::BypassPermissions,
            })
        );
        // The raw per-layer values are still reported verbatim.
        assert_eq!(s.user_value, Some(PermissionMode::Plan));
        assert_eq!(
            s.local_project_value,
            Some(PermissionMode::BypassPermissions)
        );
    }

    #[test]
    fn bypass_and_auto_in_the_committed_project_file_are_ignored_too() {
        for mode in ["bypassPermissions", "auto"] {
            let (_t, project, _l) = isolated();
            write_project_layer(&project, mode);
            let s = resolve_default_mode(&project).unwrap();
            assert_eq!(s.effective, PermissionMode::Default, "{mode}");
            assert_eq!(s.decided_by, PermissionDecisionSource::ProjectScopeIgnored);
            assert_eq!(
                s.ignored.as_ref().map(|i| i.layer),
                Some(SettingsLayer::Project)
            );
            assert_eq!(
                s.ignored.map(|i| i.mode),
                Some(PermissionMode::from_wire_str(mode))
            );
        }
    }

    #[test]
    fn the_local_file_still_shadows_a_project_file_value_that_would_be_ignored() {
        // Local `plan` wins over committed `bypassPermissions`: the
        // most specific layer decides, and its value is honoured.
        let (_t, project, _l) = isolated();
        write_project_layer(&project, "bypassPermissions");
        write_default_mode(SettingsLayer::LocalProject, &project, &PermissionMode::Plan).unwrap();
        let s = resolve_default_mode(&project).unwrap();
        assert_eq!(s.effective, PermissionMode::Plan);
        assert_eq!(s.decided_by, PermissionDecisionSource::LocalProjectSettings);
        assert_eq!(s.ignored, None, "the shadowed value is not what CC ignores");
    }

    #[test]
    fn bypass_and_auto_in_user_settings_are_honoured() {
        // "only policy/user/flag settings may grant bypass mode".
        for mode in [PermissionMode::BypassPermissions, PermissionMode::Auto] {
            let (_t, project, _l) = isolated();
            write_default_mode(SettingsLayer::User, &project, &mode).unwrap();
            let s = resolve_default_mode(&project).unwrap();
            assert_eq!(s.effective, mode);
            assert_eq!(s.decided_by, PermissionDecisionSource::UserSettings);
            assert_eq!(s.ignored, None);
        }
    }

    #[test]
    fn the_pin_names_the_release_that_changed_the_rule() {
        // The pane quotes this; the watchlist row re-verifies it.
        assert_eq!(PROJECT_SCOPE_IGNORES_SINCE, "2.1.257");
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
        assert_eq!(
            after["permissions"]["allow"][0],
            JsonValue::from("Bash(ls)")
        );
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
        assert_eq!(
            after["permissions"]["allow"][0],
            JsonValue::from("Bash(ls)")
        );
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
        let write_err =
            write_default_mode(SettingsLayer::LocalProject, &project, &PermissionMode::Plan)
                .unwrap_err();
        assert!(matches!(
            write_err,
            PermissionSettingsError::PermissionsNotAnObject(_)
        ));
        // Clear has to agree. Reporting success would tell the orchestrator
        // the grant was reverted, and it would drop the grant record —
        // leaving the project elevated with nothing left that knows to
        // revert it.
        let clear_err = clear_default_mode(SettingsLayer::LocalProject, &project).unwrap_err();
        assert!(
            matches!(
                clear_err,
                PermissionSettingsError::PermissionsNotAnObject(_)
            ),
            "got {clear_err:?}"
        );
        // The malformed file is left exactly as it was.
        assert_eq!(fs::read(&path).unwrap(), br#"{"permissions":"oops"}"#);
    }

    #[test]
    fn clear_is_still_a_noop_when_permissions_is_absent_or_null() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for body in [&br#"{"other":1}"#[..], &br#"{"permissions":null}"#[..]] {
            fs::write(&path, body).unwrap();
            clear_default_mode(SettingsLayer::LocalProject, &project).unwrap();
            assert_eq!(fs::read(&path).unwrap(), body);
        }
    }

    #[test]
    fn unknown_mode_round_trips_through_settings() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::LocalProject.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"permissions":{"defaultMode":"bubble"}}"#).unwrap();
        let v = read_default_mode(&path).unwrap();
        assert_eq!(v, Some(PermissionMode::Unknown("bubble".into())));
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
