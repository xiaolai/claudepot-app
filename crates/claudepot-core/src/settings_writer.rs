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

use crate::paths::claude_config_dir;
use crate::settings_mutex::{mutate_settings_file, Change, FileWas, SettingsMutexError};
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
    /// A settings layer could not be read — unreadable file, bad permissions,
    /// malformed JSON.
    ///
    /// The resolver used to swallow those with `.unwrap_or(None)`, which
    /// turned "we could not tell" into the positive claim "no layer sets
    /// this, so auto-memory is on". `rules/design.md` will not let a status
    /// surface present an unverified claim as fact, so the flag travels with
    /// the state and the UI qualifies what it says. The env vars are still
    /// readable and still decide, so this is a caveat rather than a failure.
    pub read_incomplete: bool,
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
    /// Another writer — Claude Code, or a text editor — kept moving the file
    /// while we were rebasing onto it. See [`crate::settings_mutex`].
    #[error("{path} is being written by something else; try again")]
    Contended { path: PathBuf },
}

/// Map the shared boundary's failures onto this module's existing shape, so
/// callers (and their tests) keep matching on the variants they already know.
impl From<SettingsMutexError> for SettingsWriteError {
    fn from(e: SettingsMutexError) -> Self {
        match e {
            SettingsMutexError::Io(e) => Self::Io(e),
            SettingsMutexError::JsonParse(e) => Self::JsonParse(e),
            SettingsMutexError::NotAJsonObject(p) => Self::NotAJsonObject(p),
            SettingsMutexError::Contended { path, .. } => Self::Contended { path },
        }
    }
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

/// What a settings file actually says about one key.
///
/// The third arm is the reason this is not an `Option`. "Key absent" and
/// "key present holding something of the wrong type" look identical through
/// an `Option`, and for `cleanupPeriodDays` they mean opposite things: absent
/// means CC applies its 30-day default and deletes transcripts, while present
/// -but-invalid means CC's own validation fails and cleanup is **suppressed**
/// (`utils/cleanup.ts:579`). Collapsing the two told the user their history
/// was on a 30-day timer when it was in fact safe — and offered them
/// "restore the default", which is the one action that re-arms the deletion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingValue<T> {
    /// No file, no key, or an empty file.
    Absent,
    /// Present and of the expected type.
    Present(T),
    /// Present, but not something CC's schema accepts for this key.
    Invalid,
}

impl<T> SettingValue<T> {
    /// Collapse to the legacy `Option` shape: both `Absent` and `Invalid`
    /// read as "not set". Correct for the boolean toggles, where CC coerces
    /// rather than erroring; wrong for anything whose invalid case changes
    /// CC's behavior.
    pub fn ok(self) -> Option<T> {
        match self {
            Self::Present(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_invalid(self) -> bool {
        matches!(self, Self::Invalid)
    }
}

/// Read one key out of a settings file through `extract`.
///
/// The single reader the typed helpers below delegate to — they used to be
/// byte-for-byte copies differing only in the `as_*` call.
fn read_setting_with<T>(
    path: &Path,
    key: &str,
    extract: impl Fn(&JsonValue) -> Option<T>,
) -> Result<SettingValue<T>, SettingsWriteError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SettingValue::Absent),
        Err(e) => return Err(e.into()),
    };
    if bytes.is_empty() {
        return Ok(SettingValue::Absent);
    }
    let v: JsonValue = serde_json::from_slice(&bytes)?;
    match v.as_object().and_then(|o| o.get(key)) {
        None => Ok(SettingValue::Absent),
        Some(raw) => Ok(match extract(raw) {
            Some(v) => SettingValue::Present(v),
            None => SettingValue::Invalid,
        }),
    }
}

/// Read one setting from a JSON file. Missing file → `None`. Missing
/// key → `None`. Wrong type for the key → `None` (treated as "not
/// set" rather than erroring; CC does the same coercion).
pub fn read_bool_setting(path: &Path, key: &str) -> Result<Option<bool>, SettingsWriteError> {
    read_setting_with(path, key, JsonValue::as_bool).map(SettingValue::ok)
}

/// Integer sibling of [`read_bool_setting`], with the present-but-invalid
/// case preserved.
///
/// Added for `cleanupPeriodDays` ([`crate::cc_retention`]), the one CC
/// setting whose value is a count rather than a flag — and the one where
/// wrong-type is not a shrug: CC skips cleanup entirely when its settings
/// fail validation while that key is present, so an invalid value silently
/// *protects* transcripts.
///
/// Note `as_i64` rejects a JSON float (`30.5`), which is correct here —
/// CC's schema declares the key `z.number().int()`, so a float is
/// already invalid upstream and must not be echoed back as if honored.
pub fn read_i64_setting(path: &Path, key: &str) -> Result<SettingValue<i64>, SettingsWriteError> {
    read_setting_with(path, key, JsonValue::as_i64)
}

/// Resolve `autoMemoryEnabled` for the global scope only. Reads env
/// vars + `~/.claude/settings.json` and ignores the project-scoped
/// layers entirely. Use this when the caller has no real project
/// anchor (Settings → General global toggle); the per-project
/// resolver feeds the home dir as project_root, which then collapses
/// `userSettings` and `projectSettings` onto the same file (audit
/// 2026-05 #3).
/// One layer's value plus whether reading it failed.
fn read_layer(path: &Path) -> (Option<bool>, bool) {
    match read_bool_setting(path, AUTO_MEMORY_KEY) {
        Ok(v) => (v, false),
        Err(_) => (None, true),
    }
}

/// The whole resolution, over values already read. Both entry points call
/// this; they used to be two copies of the same five-branch chain with the
/// same eight-field literal repeated at every return, which is how the two
/// drifted in the first place.
fn resolve_auto_memory(
    user_settings_value: Option<bool>,
    project_settings_value: Option<bool>,
    local_project_settings_value: Option<bool>,
    read_incomplete: bool,
) -> AutoMemoryState {
    let env_disable_raw = std::env::var("CLAUDE_CODE_DISABLE_AUTO_MEMORY").ok();
    let env_simple_raw = std::env::var("CLAUDE_CODE_SIMPLE").ok();
    let env_disable_set = env_is_truthy(env_disable_raw.as_deref());
    let env_simple_set = env_is_truthy(env_simple_raw.as_deref());
    let env_disable_explicit_off = env_is_falsy(env_disable_raw.as_deref());

    // Exactly CC's order, first match wins. Env vars beat every settings
    // layer, and an explicit falsy CLAUDE_CODE_DISABLE_AUTO_MEMORY
    // short-circuits SIMPLE and the rest of the chain too.
    let (effective, decided_by, user_writable) = if env_disable_set {
        (false, AutoMemoryDecisionSource::EnvDisable, false)
    } else if env_disable_explicit_off {
        (true, AutoMemoryDecisionSource::EnvDisable, false)
    } else if env_simple_set {
        (false, AutoMemoryDecisionSource::EnvSimple, false)
    } else if let Some(v) = local_project_settings_value {
        (v, AutoMemoryDecisionSource::LocalProjectSettings, true)
    } else if let Some(v) = project_settings_value {
        // Project settings are committed to the repo, so a Claudepot toggle
        // still writes — to LocalProject — it just overrides this value.
        (v, AutoMemoryDecisionSource::ProjectSettings, true)
    } else if let Some(v) = user_settings_value {
        (v, AutoMemoryDecisionSource::UserSettings, true)
    } else {
        (true, AutoMemoryDecisionSource::Default, true)
    };

    AutoMemoryState {
        effective,
        decided_by,
        user_writable,
        user_settings_value,
        project_settings_value,
        local_project_settings_value,
        env_disable_set,
        env_simple_set,
        read_incomplete,
    }
}

/// Resolve `autoMemoryEnabled` for the global scope only. Reads env
/// vars + `~/.claude/settings.json` and ignores the project-scoped
/// layers entirely. Use this when the caller has no real project
/// anchor (Settings → General global toggle); the per-project
/// resolver feeds the home dir as project_root, which then collapses
/// `userSettings` and `projectSettings` onto the same file (audit
/// 2026-05 #3).
pub fn resolve_auto_memory_enabled_global() -> AutoMemoryState {
    let (user_value, failed) = read_layer(&claude_config_dir().join("settings.json"));
    resolve_auto_memory(user_value, None, None, failed)
}

/// Read the full `autoMemoryEnabled` resolution for `project_root`.
/// Pure function over env + filesystem state — no side effects.
pub fn resolve_auto_memory_enabled(project_root: &Path) -> AutoMemoryState {
    let (user, e1) = read_layer(&SettingsLayer::User.settings_file(project_root));
    let (project, e2) = read_layer(&SettingsLayer::Project.settings_file(project_root));
    let (local, e3) = read_layer(&SettingsLayer::LocalProject.settings_file(project_root));
    resolve_auto_memory(user, project, local, e1 || e2 || e3)
}

/// Read-modify-write a settings file, setting `key` to `value`.
/// Preserves all unknown keys. If the file is missing, creates it
/// with just `{ key: value }`. If the file is malformed JSON, the
/// caller gets `SettingsWriteError::JsonParse` — we never silently
/// overwrite a file we couldn't parse.
///
/// Serialized through [`crate::settings_mutex`]: atomic rename alone would
/// let a concurrent read-modify-write of the same file discard this edit.
fn rmw_settings_bool(path: &Path, key: &str, value: bool) -> Result<(), SettingsWriteError> {
    mutate_settings_file(path, |object, _| {
        object.insert(key.to_string(), JsonValue::Bool(value));
        Ok::<_, SettingsWriteError>(Change::Write(()))
    })
    .map(|_| ())
}

/// Remove `key` from the settings file. If the file is missing or the
/// key is absent, this is a no-op. Used to clear an override.
fn rmw_settings_remove(path: &Path, key: &str) -> Result<(), SettingsWriteError> {
    mutate_settings_file(path, |object, was| {
        if !was.is_present() || object.remove(key).is_none() {
            return Ok::<_, SettingsWriteError>(Change::Skip(()));
        }
        Ok(Change::Write(()))
    })
    .map(|_| ())
}

/// Set `autoMemoryEnabled` at the given layer. Refuses to write to
/// `Project` (committed file) or any other unsupported layer.
pub fn write_auto_memory_enabled(
    layer: SettingsLayer,
    project_root: &Path,
    value: bool,
) -> Result<(), SettingsWriteError> {
    write_bool_setting(layer, project_root, AUTO_MEMORY_KEY, value)
}

/// Clear `autoMemoryEnabled` at the given layer (the key is removed,
/// not set to a default). Lets the next-higher layer take over the
/// decision.
pub fn clear_auto_memory_enabled(
    layer: SettingsLayer,
    project_root: &Path,
) -> Result<(), SettingsWriteError> {
    clear_bool_setting(layer, project_root, AUTO_MEMORY_KEY)
}

/// Generic sibling of [`write_auto_memory_enabled`]: set an arbitrary
/// top-level boolean `key` at `layer`. Preserves every other key in
/// the file (read-modify-write). Refuses `Project` (committed to the
/// repo) for the same reason auto-memory does.
///
/// Added for the `enableArtifact` toggle (`artifact_toggle`), which is
/// a top-level boolean of exactly the same shape as `autoMemoryEnabled`
/// but a different key — so the write path is shared rather than
/// copied per key.
pub fn write_bool_setting(
    layer: SettingsLayer,
    project_root: &Path,
    key: &str,
    value: bool,
) -> Result<(), SettingsWriteError> {
    match layer {
        SettingsLayer::User | SettingsLayer::LocalProject => {
            rmw_settings_bool(&layer.settings_file(project_root), key, value)
        }
        SettingsLayer::Project => Err(SettingsWriteError::UnsupportedLayer { layer }),
    }
}

/// Generic sibling of [`clear_auto_memory_enabled`]: remove an
/// arbitrary top-level `key` at `layer` (no-op if absent). Refuses
/// `Project`.
pub fn clear_bool_setting(
    layer: SettingsLayer,
    project_root: &Path,
    key: &str,
) -> Result<(), SettingsWriteError> {
    match layer {
        SettingsLayer::User | SettingsLayer::LocalProject => {
            rmw_settings_remove(&layer.settings_file(project_root), key)
        }
        SettingsLayer::Project => Err(SettingsWriteError::UnsupportedLayer { layer }),
    }
}

/// Read-modify-write a settings file through a closure that mutates the
/// top-level JSON object in place, then persists the result in ONE
/// atomic write. This is the multi-key sibling of
/// [`write_bool_setting`] / [`clear_bool_setting`]: a caller that must
/// change several keys together (e.g. `attribution` +
/// `includeCoAuthoredBy`) can do so without a crash leaving a half-
/// applied, mixed-semantics file.
///
/// Guarantees, matching the single-key helpers:
/// - preserves every key the closure doesn't touch;
/// - refuses the committed `Project` layer;
/// - errors (never clobbers) on malformed or non-object JSON;
/// - creates the file (and parent dir) if missing — UNLESS the file
///   was absent *and* the closure leaves the object empty, in which
///   case it is a no-op (we don't litter an empty `{}` settings file,
///   mirroring `rmw_settings_remove`'s no-op-on-missing behavior).
///
/// The closure is `FnMut` rather than `FnOnce` because the mutation boundary
/// re-runs it when an external writer moves the file mid-edit; it must
/// therefore be free of side effects outside the object it is handed.
pub fn mutate_settings(
    layer: SettingsLayer,
    project_root: &Path,
    mut mutate: impl FnMut(&mut serde_json::Map<String, JsonValue>),
) -> Result<(), SettingsWriteError> {
    if layer == SettingsLayer::Project {
        return Err(SettingsWriteError::UnsupportedLayer { layer });
    }
    let path = layer.settings_file(project_root);
    mutate_settings_file(&path, |object, was| {
        mutate(object);
        if was == FileWas::Absent && object.is_empty() {
            return Ok::<_, SettingsWriteError>(Change::Skip(()));
        }
        Ok(Change::Write(()))
    })
    .map(|_| ())
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

    /// An unreadable layer must not become the positive claim "no layer sets
    /// this". The env vars are still readable and still decide, so the state
    /// is usable — it just has to say it is incomplete.
    #[test]
    fn an_unreadable_layer_is_reported_rather_than_read_as_absent() {
        let (_t, project, _l) = isolated();
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");

        let clean = resolve_auto_memory_enabled(&project);
        assert!(!clean.read_incomplete);

        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ not json").unwrap();

        let s = resolve_auto_memory_enabled(&project);
        assert!(s.read_incomplete, "a malformed layer must be surfaced");
        assert_eq!(s.user_settings_value, None);
        // Still decides — the env chain is intact — but the caller now knows
        // the settings half of that decision is unverified.
        assert_eq!(s.decided_by, AutoMemoryDecisionSource::Default);

        let g = resolve_auto_memory_enabled_global();
        assert!(g.read_incomplete);
    }

    /// The two resolvers share one chain now; this pins them to the same
    /// answer for the same user-layer input, which is what drifted before.
    #[test]
    fn the_global_and_per_project_resolvers_agree_on_the_user_layer() {
        let (_t, project, _l) = isolated();
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("CLAUDE_CODE_SIMPLE");

        for value in [true, false] {
            write_auto_memory_enabled(SettingsLayer::User, &project, value).unwrap();
            let per_project = resolve_auto_memory_enabled(&project);
            let global = resolve_auto_memory_enabled_global();
            assert_eq!(global.effective, value);
            assert_eq!(per_project.effective, value);
            assert_eq!(global.decided_by, per_project.decided_by);
            assert_eq!(global.user_settings_value, per_project.user_settings_value);
        }
    }

    #[test]
    fn write_preserves_unknown_keys() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"unrelatedKey":42,"nested":{"keep":"me"}}"#).unwrap();

        write_auto_memory_enabled(SettingsLayer::User, &project, false).unwrap();

        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
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
            SettingsWriteError::UnsupportedLayer {
                layer: SettingsLayer::Project,
            } => {}
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

        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(after
            .as_object()
            .unwrap()
            .get("autoMemoryEnabled")
            .is_none());
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

    #[test]
    fn generic_write_and_clear_round_trips_arbitrary_key() {
        let (_t, project, _l) = isolated();
        let file = SettingsLayer::User.settings_file(&project);
        write_bool_setting(SettingsLayer::User, &project, "enableArtifact", false).unwrap();
        assert_eq!(
            read_bool_setting(&file, "enableArtifact").unwrap(),
            Some(false)
        );
        clear_bool_setting(SettingsLayer::User, &project, "enableArtifact").unwrap();
        assert_eq!(read_bool_setting(&file, "enableArtifact").unwrap(), None);
    }

    #[test]
    fn generic_write_bool_setting_refuses_project_layer() {
        let (_t, project, _l) = isolated();
        let err = write_bool_setting(SettingsLayer::Project, &project, "enableArtifact", true)
            .unwrap_err();
        assert!(matches!(
            err,
            SettingsWriteError::UnsupportedLayer {
                layer: SettingsLayer::Project
            }
        ));
    }

    #[test]
    fn generic_clear_bool_setting_refuses_project_layer() {
        let (_t, project, _l) = isolated();
        let err =
            clear_bool_setting(SettingsLayer::Project, &project, "enableArtifact").unwrap_err();
        assert!(matches!(
            err,
            SettingsWriteError::UnsupportedLayer {
                layer: SettingsLayer::Project
            }
        ));
    }

    #[test]
    fn mutate_settings_applies_multi_key_edit_and_preserves_rest() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"keep":1}"#).unwrap();

        mutate_settings(SettingsLayer::User, &project, |m| {
            m.insert("a".into(), JsonValue::Bool(true));
            m.insert("b".into(), JsonValue::String("x".into()));
        })
        .unwrap();

        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["keep"], JsonValue::from(1));
        assert_eq!(after["a"], JsonValue::Bool(true));
        assert_eq!(after["b"], JsonValue::from("x"));
    }

    #[test]
    fn mutate_settings_noop_when_file_missing_and_result_empty() {
        let (_t, project, _l) = isolated();
        // Closure removes keys that aren't there → empty object → the
        // helper must NOT create an empty settings file.
        mutate_settings(SettingsLayer::User, &project, |m| {
            m.remove("attribution");
            m.remove("includeCoAuthoredBy");
        })
        .unwrap();
        assert!(!SettingsLayer::User.settings_file(&project).exists());
    }

    /// Two *different modules* writing `~/.claude/settings.json` at the same
    /// time must both land. This is the case the shared boundary exists for:
    /// before it, `settings_writer` and `updates::settings_bridge` each did
    /// their own read-modify-write, so whichever renamed last silently
    /// discarded the other's key. A lock only one participant holds is not a
    /// lock, which is why the migration was not deferred.
    #[test]
    fn settings_writer_and_the_updates_bridge_do_not_clobber_each_other() {
        let (_t, project, _l) = isolated();
        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{}").unwrap();

        // Both modules resolve this file from CLAUDE_CONFIG_DIR, which the
        // isolated() guard has already pointed at the tempdir.
        std::thread::scope(|s| {
            s.spawn(|| {
                for i in 0..40 {
                    write_bool_setting(
                        SettingsLayer::User,
                        &project,
                        &format!("writerKey{i}"),
                        i % 2 == 0,
                    )
                    .unwrap();
                }
            });
            s.spawn(|| {
                for i in 0..40 {
                    crate::updates::settings_bridge::write_minimum_version(Some(&format!(
                        "2.1.{i}"
                    )))
                    .unwrap();
                }
            });
        });

        let after: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let object = after.as_object().unwrap();
        for i in 0..40 {
            assert!(
                object.contains_key(&format!("writerKey{i}")),
                "settings_writer's key {i} was clobbered by the updates bridge"
            );
        }
        assert_eq!(
            object["minimumVersion"],
            JsonValue::from("2.1.39"),
            "the updates bridge's last write was clobbered by settings_writer"
        );
    }

    #[test]
    fn mutate_settings_refuses_project_layer_and_errors_on_malformed() {
        let (_t, project, _l) = isolated();
        let err = mutate_settings(SettingsLayer::Project, &project, |_| {}).unwrap_err();
        assert!(matches!(
            err,
            SettingsWriteError::UnsupportedLayer {
                layer: SettingsLayer::Project
            }
        ));

        let path = SettingsLayer::User.settings_file(&project);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ not json").unwrap();
        let err = mutate_settings(SettingsLayer::User, &project, |_| {}).unwrap_err();
        assert!(matches!(err, SettingsWriteError::JsonParse(_)));
        // Untouched on parse failure.
        assert_eq!(fs::read(&path).unwrap(), b"{ not json");
    }
}
