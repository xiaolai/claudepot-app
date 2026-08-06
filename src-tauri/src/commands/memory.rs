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

use crate::dto_error::{codes, ErrorDto};
use crate::dto_memory::{AutoMemoryStateDto, MemoryChangeDto, MemoryEnumerateDto};
use claudepot_core::memory_log::{ChangeQuery, MemoryFileStats, MemoryLog};
use claudepot_core::memory_view::{enumerate_project_memory, read_memory_content};
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

fn resolve_project_root(raw: &str) -> Result<PathBuf, ErrorDto> {
    if raw.is_empty() {
        return Err(ErrorDto::new(
            codes::MEMORY_PROJECT_ROOT_EMPTY,
            "project_root is empty",
        ));
    }
    // `resolve project path: ` is gone — the UI owns that framing.
    // `resolve_path` fails with a `ProjectError`, which already carries
    // its own code and params.
    resolve_path(raw).map(PathBuf::from).map_err(ErrorDto::from)
}

/// `memory_list_for_project` — enumerate memory files for a project
/// and join with the change-log per-file aggregates so the UI can
/// render the file list with badges in one round-trip.
#[tauri::command]
pub async fn memory_list_for_project(
    project_root: String,
    state: State<'_, MemoryLogState>,
) -> Result<MemoryEnumerateDto, ErrorDto> {
    let root = resolve_project_root(&project_root)?;
    // `enumerate_project_memory` returns a bare `std::io::Result`, so
    // the identity is minted at this boundary. `enumerate memory: ` is
    // gone; the underlying io text survives as `message`.
    let result = enumerate_project_memory(&root, true)
        .map_err(|e| ErrorDto::detail(codes::MEMORY_ENUMERATE_FAILED, e))?;
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
pub async fn memory_read_file(project_root: String, abs_path: String) -> Result<String, ErrorDto> {
    let root = resolve_project_root(&project_root)?;
    let target = PathBuf::from(&abs_path);
    // The hand-written per-variant strings are gone: `ReadMemoryError`
    // carries `memory_view.path_outside_scope` / `memory_view.io` with
    // the rejected path in `params`, and its own `Display` is the
    // English. The refusal message now names the path (core's text
    // does; this call site used to drop it), which is what lets the
    // GUI say *which* file was outside scope.
    read_memory_content(&target, &[root]).map_err(ErrorDto::from)
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
) -> Result<Vec<MemoryChangeDto>, ErrorDto> {
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
    .map_err(ErrorDto::from)?;
    Ok(rows.into_iter().map(MemoryChangeDto::from).collect())
}

/// `auto_memory_state` — read CC's full `autoMemoryEnabled` priority
/// chain for a given project.
#[tauri::command]
pub async fn auto_memory_state(project_root: String) -> Result<AutoMemoryStateDto, ErrorDto> {
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
pub async fn auto_memory_state_global() -> Result<AutoMemoryStateDto, ErrorDto> {
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
) -> Result<AutoMemoryStateDto, ErrorDto> {
    let root = resolve_project_root(&project_root)?;
    let layer = match scope.as_str() {
        "user" => SettingsLayer::User,
        "local_project" => SettingsLayer::LocalProject,
        other => {
            return Err(ErrorDto::with_params(
                codes::MEMORY_UNKNOWN_SCOPE,
                serde_json::json!({ "scope": other }),
                format!("unknown scope {other}; want user|local_project"),
            ))
        }
    };
    // `SettingsWriteError` has no `ErrorCode` yet — it belongs to the
    // settings slice — so these two mint a command-site identity and
    // carry core's English through as `message`. `write setting: ` /
    // `clear setting: ` are gone; the UI owns that framing.
    match value {
        Some(v) => write_auto_memory_enabled(layer, &root, v)
            .map_err(|e| ErrorDto::detail(codes::MEMORY_AUTO_MEMORY_WRITE_FAILED, e))?,
        None => clear_auto_memory_enabled(layer, &root)
            .map_err(|e| ErrorDto::detail(codes::MEMORY_AUTO_MEMORY_CLEAR_FAILED, e))?,
    }
    let state = resolve_auto_memory_enabled(&root);
    Ok(AutoMemoryStateDto::from_state(state, &root))
}
