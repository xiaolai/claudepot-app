//! IPC commands for the Projects → Memory pane and Settings →
//! Auto-memory toggle.
//!
//! Five commands:
//! - `memory_list_for_project(project_root)` — file enumeration + per-file
//!   change-log aggregates.
//! - `memory_read_file(project_root, abs_path)` — read content with a
//!   strict containment check.
//! - `memory_change_log(project_root, file_path?, limit?)` — query the
//!   persisted change log, scoped to project or single file.
//! - `auto_memory_state(project_root)` — full priority-chain breakdown.
//! - `auto_memory_set(project_root, scope, value)` — write the toggle.
//!   `scope = "user" | "local_project"`. Refuses any other scope.

use crate::dto_memory::{AutoMemoryStateDto, MemoryChangeDto, MemoryEnumerateDto};
use claudepot_core::memory_log::{ChangeQuery, MemoryFileStats, MemoryLog};
use claudepot_core::memory_view::{
    enumerate_project_memory, read_memory_content, ReadMemoryError,
};
use claudepot_core::project_helpers::resolve_path;
use claudepot_core::settings_writer::{
    clear_auto_memory_enabled, resolve_auto_memory_enabled, resolve_auto_memory_enabled_global,
    write_auto_memory_enabled, SettingsLayer,
};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

/// Shared state holding the `MemoryLog` handle. Cloned-`Arc` rather
/// than `Mutex<Connection>` so multiple commands can read concurrently;
/// the `MemoryLog` itself wraps its own `Mutex<Connection>` for
/// transaction safety.
#[derive(Clone)]
pub struct MemoryLogState {
    pub log: Arc<MemoryLog>,
}

impl MemoryLogState {
    pub fn new(log: Arc<MemoryLog>) -> Self {
        Self { log }
    }
}

fn resolve_project_root(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("project_root is empty".to_string());
    }
    resolve_path(raw)
        .map(PathBuf::from)
        .map_err(|e| format!("resolve project path: {e}"))
}

/// `memory_list_for_project` — enumerate memory files for a project
/// and join with the change-log per-file aggregates so the UI can
/// render the file list with badges in one round-trip.
#[tauri::command]
pub async fn memory_list_for_project(
    project_root: String,
    state: State<'_, MemoryLogState>,
) -> Result<MemoryEnumerateDto, String> {
    let root = resolve_project_root(&project_root)?;
    let result = enumerate_project_memory(&root, true)
        .map_err(|e| format!("enumerate memory: {e}"))?;
    let stats: std::collections::HashMap<PathBuf, MemoryFileStats> = state
        .log
        .project_file_stats(&result.anchor.slug)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.abs_path.clone(), s))
        .collect();
    Ok(MemoryEnumerateDto::from_result(result, &stats))
}

/// `memory_read_file` — read a memory file by absolute path. The
/// `project_root` argument scopes the containment check; passing a
/// path outside that scope returns an error rather than reading.
#[tauri::command]
pub async fn memory_read_file(
    project_root: String,
    abs_path: String,
) -> Result<String, String> {
    let root = resolve_project_root(&project_root)?;
    let target = PathBuf::from(&abs_path);
    read_memory_content(&target, &[root]).map_err(|e| match e {
        ReadMemoryError::PathOutsideScope(_) => {
            "path is outside the allowed memory scopes".to_string()
        }
        ReadMemoryError::Io(io) => format!("read failed: {io}"),
    })
}

/// `memory_change_log` — query the persisted change log. With
/// `file_path` set, returns rows for that one file; without, returns
/// the project's full log.
#[tauri::command]
pub async fn memory_change_log(
    project_root: String,
    file_path: Option<String>,
    limit: Option<usize>,
    state: State<'_, MemoryLogState>,
) -> Result<Vec<MemoryChangeDto>, String> {
    let root = resolve_project_root(&project_root)?;
    let q = ChangeQuery {
        limit,
        ..Default::default()
    };
    let rows = match file_path {
        Some(p) => state.log.query_for_path(&PathBuf::from(p), &q),
        None => {
            let anchor = claudepot_core::memory_view::ProjectMemoryAnchor::for_project(&root);
            state.log.query_for_project(&anchor.slug, &q)
        }
    }
    .map_err(|e| format!("query change log: {e}"))?;
    Ok(rows.into_iter().map(MemoryChangeDto::from).collect())
}

/// `auto_memory_state` — read CC's full `autoMemoryEnabled` priority
/// chain for a given project.
#[tauri::command]
pub async fn auto_memory_state(project_root: String) -> Result<AutoMemoryStateDto, String> {
    let root = resolve_project_root(&project_root)?;
    let state = resolve_auto_memory_enabled(&root);
    Ok(AutoMemoryStateDto::from_state(state, &root))
}

/// `auto_memory_state_global` — read only env vars + `~/.claude/settings.json`,
/// without folding in any project-scoped settings. Used by the
/// Settings → General global toggle so it doesn't accidentally treat
/// home-directory `.claude/settings.json` as a project override (audit
/// 2026-05 #3).
#[tauri::command]
pub async fn auto_memory_state_global() -> Result<AutoMemoryStateDto, String> {
    let state = resolve_auto_memory_enabled_global();
    // No project anchor — pass an empty PathBuf so the DTO carries an
    // empty string; the global toggle never displays this field.
    let empty = PathBuf::new();
    Ok(AutoMemoryStateDto::from_state(state, &empty))
}

/// `auto_memory_set` — write the toggle. `scope`:
/// - `"user"`: writes `~/.claude/settings.json`
/// - `"local_project"`: writes `<project>/.claude/settings.local.json`
///
/// `value = null` clears the key from that layer.
#[tauri::command]
pub async fn auto_memory_set(
    project_root: String,
    scope: String,
    value: Option<bool>,
) -> Result<AutoMemoryStateDto, String> {
    let root = resolve_project_root(&project_root)?;
    let layer = match scope.as_str() {
        "user" => SettingsLayer::User,
        "local_project" => SettingsLayer::LocalProject,
        other => return Err(format!("unknown scope {other}; want user|local_project")),
    };
    match value {
        Some(v) => write_auto_memory_enabled(layer, &root, v)
            .map_err(|e| format!("write setting: {e}"))?,
        None => clear_auto_memory_enabled(layer, &root)
            .map_err(|e| format!("clear setting: {e}"))?,
    }
    let state = resolve_auto_memory_enabled(&root);
    Ok(AutoMemoryStateDto::from_state(state, &root))
}
