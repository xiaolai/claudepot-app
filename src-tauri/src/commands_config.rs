//! Config section Tauri commands — P0.
//!
//! P0 ships:
//! - `config_scan` — empty tree stub (discovery lands in P1).
//! - `config_list_editors` — detected editors, cached 5 min.
//! - `config_open_in_editor` — fire-and-forget launch.
//! - `config_open_in_editor_path` — raw-path variant; used when the
//!   section is showing a stub and has no node_ids yet, and by the
//!   "Other…" file picker which receives a user-supplied binary.
//! - `config_get_editor_defaults` / `config_set_editor_default` —
//!   per-kind preferences, persisted in `preferences.json`.
//!
//! Per `.claude/rules/architecture.md`: all logic lives in
//! `claudepot_core::config_view`; these commands are DTO adapters only.

use crate::preferences::PreferencesState;
use claudepot_core::config_view::{
    launcher::{self, LaunchError},
    model::{
        DetectSource, EditorCandidate, EditorDefaults, Kind, LaunchKind,
    },
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

// ---------- DTOs ------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct ConfigTreeDto {
    pub scopes: Vec<serde_json::Value>,
    pub cwd: String,
    pub project_root: String,
    pub memory_slug: String,
    pub memory_slug_lossy: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct EditorCandidateDto {
    pub id: String,
    pub label: String,
    pub binary_path: Option<String>,
    pub bundle_id: Option<String>,
    pub launch_kind: String,
    pub detected_via: String,
    pub supports_kinds: Option<Vec<Kind>>,
}

impl From<&EditorCandidate> for EditorCandidateDto {
    fn from(c: &EditorCandidate) -> Self {
        let launch_kind = match &c.launch {
            LaunchKind::Direct { .. } => "direct",
            LaunchKind::MacosOpenA { .. } => "macos-open-a",
            LaunchKind::EnvEditor => "env-editor",
            LaunchKind::SystemHandler => "system-handler",
        }
        .to_string();
        let detected_via = match &c.detected_via {
            DetectSource::PathBinary { .. } => "path-binary",
            DetectSource::MacosAppBundle { .. } => "macos-app",
            DetectSource::WindowsRegistry { .. } => "windows-registry",
            DetectSource::LinuxDesktopFile { .. } => "linux-desktop-file",
            DetectSource::EnvVar { .. } => "env-var",
            DetectSource::SystemDefault => "system-default",
            DetectSource::UserPicked { .. } => "user-picked",
        }
        .to_string();
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            binary_path: c
                .binary_path
                .as_ref()
                .map(|p| p.display().to_string()),
            bundle_id: c.bundle_id.clone(),
            launch_kind,
            detected_via,
            supports_kinds: c.supports_kinds.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditorDefaultsDto {
    pub by_kind: std::collections::BTreeMap<String, String>,
    pub fallback: String,
}

impl From<&EditorDefaults> for EditorDefaultsDto {
    fn from(d: &EditorDefaults) -> Self {
        Self {
            by_kind: d
                .by_kind
                .iter()
                .map(|(k, v)| (kind_to_str(k).to_string(), v.clone()))
                .collect(),
            fallback: d.fallback.clone(),
        }
    }
}

fn kind_to_str(k: &Kind) -> &'static str {
    match k {
        Kind::ClaudeMd => "claude_md",
        Kind::Settings => "settings",
        Kind::SettingsLocal => "settings_local",
        Kind::ManagedSettings => "managed_settings",
        Kind::RedactedUserConfig => "redacted_user_config",
        Kind::McpJson => "mcp_json",
        Kind::ManagedMcpJson => "managed_mcp_json",
        Kind::Agent => "agent",
        Kind::Skill => "skill",
        Kind::Command => "command",
        Kind::Rule => "rule",
        Kind::Hook => "hook",
        Kind::Memory => "memory",
        Kind::MemoryIndex => "memory_index",
        Kind::Plugin => "plugin",
        Kind::Keybindings => "keybindings",
        Kind::Statusline => "statusline",
        Kind::EffectiveSettings => "effective_settings",
        Kind::EffectiveMcp => "effective_mcp",
        Kind::Other => "other",
    }
}

fn kind_from_str(s: &str) -> Option<Kind> {
    Some(match s {
        "claude_md" => Kind::ClaudeMd,
        "settings" => Kind::Settings,
        "settings_local" => Kind::SettingsLocal,
        "managed_settings" => Kind::ManagedSettings,
        "redacted_user_config" => Kind::RedactedUserConfig,
        "mcp_json" => Kind::McpJson,
        "managed_mcp_json" => Kind::ManagedMcpJson,
        "agent" => Kind::Agent,
        "skill" => Kind::Skill,
        "command" => Kind::Command,
        "rule" => Kind::Rule,
        "hook" => Kind::Hook,
        "memory" => Kind::Memory,
        "memory_index" => Kind::MemoryIndex,
        "plugin" => Kind::Plugin,
        "keybindings" => Kind::Keybindings,
        "statusline" => Kind::Statusline,
        "effective_settings" => Kind::EffectiveSettings,
        "effective_mcp" => Kind::EffectiveMcp,
        "other" => Kind::Other,
        _ => return None,
    })
}

// ---------- Commands --------------------------------------------------

/// P0 stub: return an empty tree anchored at `cwd`. Real discovery in P1.
#[tauri::command]
pub async fn config_scan(cwd: Option<String>) -> Result<ConfigTreeDto, String> {
    let cwd_path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let tree = claudepot_core::config_view::empty_tree(&cwd_path);
    Ok(ConfigTreeDto {
        scopes: Vec::new(),
        cwd: tree.cwd.display().to_string(),
        project_root: tree.project_root.display().to_string(),
        memory_slug: tree.memory_slug,
        memory_slug_lossy: tree.memory_slug_lossy,
    })
}

/// Detected editors, cached for 5 minutes. Pass `force = true` to
/// bypass the cache (e.g. after a user installs a new editor).
#[tauri::command]
pub async fn config_list_editors(
    force: Option<bool>,
) -> Result<Vec<EditorCandidateDto>, String> {
    let force = force.unwrap_or(false);
    let cands = launcher::detect_cached(force);
    Ok(cands.iter().map(EditorCandidateDto::from).collect())
}

/// Read persisted editor defaults.
#[tauri::command]
pub fn config_get_editor_defaults(
    prefs: State<'_, PreferencesState>,
) -> Result<EditorDefaultsDto, String> {
    let g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
    Ok(EditorDefaultsDto::from(&g.editor_defaults))
}

/// Set the default editor for a given `kind`. When `kind` is `None`,
/// update the global `fallback` instead.
#[tauri::command]
pub fn config_set_editor_default(
    kind: Option<String>,
    editor_id: String,
    prefs: State<'_, PreferencesState>,
) -> Result<(), String> {
    if editor_id.trim().is_empty() {
        return Err("editor_id is empty".to_string());
    }
    let mut g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
    match kind {
        None => {
            g.editor_defaults.fallback = editor_id;
        }
        Some(k) => {
            let parsed = kind_from_str(&k).ok_or_else(|| format!("unknown kind: {k}"))?;
            g.editor_defaults.by_kind.insert(parsed, editor_id);
        }
    }
    g.save()?;
    Ok(())
}

/// Launch an arbitrary path in the user's chosen editor. If
/// `editor_id` is omitted, resolves the per-kind default from
/// `preferences.editor_defaults`.
///
/// `kind_hint` steers per-kind default resolution — callers that
/// don't know the kind pass `None` and we use the global fallback.
#[tauri::command]
pub fn config_open_in_editor_path(
    path: String,
    editor_id: Option<String>,
    kind_hint: Option<String>,
    prefs: State<'_, PreferencesState>,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("path is empty".to_string());
    }
    let target = PathBuf::from(&path);
    let candidates = launcher::detect_cached(false);
    let defaults = {
        let g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
        g.editor_defaults.clone()
    };
    let chosen: &EditorCandidate = if let Some(id) = editor_id.as_ref() {
        candidates
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| format!("editor not detected: {id}"))?
    } else {
        let kind = kind_hint.as_deref().and_then(kind_from_str);
        let resolved = match kind {
            Some(k) => launcher::resolve_editor_for(&k, &defaults, &candidates),
            None => candidates
                .iter()
                .find(|c| c.id == defaults.fallback)
                .or_else(|| candidates.iter().find(|c| c.id == "system")),
        };
        resolved.ok_or_else(|| "no editor available".to_string())?
    };
    launch_into(chosen, &target)
}

fn launch_into(chosen: &EditorCandidate, target: &Path) -> Result<(), String> {
    launcher::invoke(chosen, target).map_err(|e| match e {
        LaunchError::Spawn(s) => format!("launch failed: {s}"),
        LaunchError::NoEnvEditor => "$EDITOR is not set".to_string(),
        LaunchError::NoBinary => "editor binary missing".to_string(),
        LaunchError::EmptyPath => "path is empty".to_string(),
        LaunchError::UnknownEditor(id) => format!("unknown editor: {id}"),
    })
}
