//! Tauri commands for the artifact-lifecycle layer.
//!
//! Read-only `artifact_classify_path` lets the renderer pre-flight
//! actions; the mutating commands take the canonical
//! `(scope_root, kind, relative_path)` triple so paths are
//! reconstructed inside the core, never built by the UI. All run via
//! `spawn_blocking` (filesystem operations).

use crate::dto_artifact_lifecycle::{
    parse_kind, ClassifyPathDto, DisabledRecordDto, RestoredArtifactDto, TrackableDto,
    TrashEntryDto,
};
use claudepot_core::artifact_lifecycle::{
    self,
    disable::OnConflict,
    paths::{classify_path, ActiveRoots, ArtifactKind, Trackable},
    LifecycleError,
};
use claudepot_core::paths;
use std::path::PathBuf;

fn join_blocking_err(e: tokio::task::JoinError) -> String {
    format!("blocking task failed: {e}")
}

fn err_to_string(e: LifecycleError) -> String {
    e.to_string()
}

fn parse_on_conflict(s: &str) -> Result<OnConflict, String> {
    match s {
        "refuse" => Ok(OnConflict::Refuse),
        "suffix" => Ok(OnConflict::Suffix),
        other => Err(format!("unknown on_conflict value: {other}")),
    }
}

/// Build the active-roots snapshot used by every command. The
/// project root is optional — global-only callers omit it.
fn build_roots(project_root: Option<String>) -> ActiveRoots {
    let mut roots = ActiveRoots::user(paths::claude_config_dir());
    if let Some(p) = project_root.filter(|s| !s.is_empty()) {
        roots = roots.with_project(PathBuf::from(p));
    }
    // Managed-policy roots are added per-platform; left empty for
    // now since none of our shipped flows currently set them.
    roots
}

/// Reconstruct an absolute path from the canonical triple, then
/// classify it. Used internally by mutating commands so the core
/// always re-derives the Trackable from the triple (defense
/// against stale UI state).
fn rebuild_trackable(
    scope_root: &str,
    kind: &str,
    relative_path: &str,
    roots: &ActiveRoots,
) -> Result<Trackable, String> {
    let kind = parse_kind(kind)?;
    let scope_root_path = PathBuf::from(scope_root);
    let abs = scope_root_path
        .join(kind.subdir())
        .join(relative_path);
    classify_path(&abs, roots)
        .or_else(|_| {
            // Maybe it's already disabled — try the .disabled location.
            let disabled = scope_root_path
                .join(claudepot_core::artifact_lifecycle::DISABLED_DIR)
                .join(kind.subdir())
                .join(relative_path);
            classify_path(&disabled, roots)
        })
        .map_err(|reason| reason.to_string())
}

/// Read-only helper: take an absolute path and report whether it's
/// trackable (and therefore eligible for Disable / Trash) or refused
/// with a typed reason. The UI uses this to render per-row
/// affordances without calling a mutating command.
#[tauri::command]
pub async fn artifact_classify_path(
    abs_path: String,
    project_root: Option<String>,
) -> Result<ClassifyPathDto, String> {
    tokio::task::spawn_blocking(move || {
        let roots = build_roots(project_root);
        match classify_path(std::path::Path::new(&abs_path), &roots) {
            Ok(t) => Ok::<_, String>(ClassifyPathDto {
                already_disabled: t.already_disabled,
                trackable: Some(TrackableDto::from(&t)),
                refused: None,
            }),
            Err(reason) => Ok(ClassifyPathDto {
                trackable: None,
                refused: Some(reason.to_string()),
                already_disabled: false,
            }),
        }
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_disable(
    scope_root: String,
    kind: String,
    relative_path: String,
    on_conflict: String,
    project_root: Option<String>,
) -> Result<DisabledRecordDto, String> {
    tokio::task::spawn_blocking(move || {
        let roots = build_roots(project_root);
        let trackable = rebuild_trackable(&scope_root, &kind, &relative_path, &roots)?;
        let policy = parse_on_conflict(&on_conflict)?;
        artifact_lifecycle::disable_at(&trackable, policy, &roots)
            .map(DisabledRecordDto::from)
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_enable(
    scope_root: String,
    kind: String,
    relative_path: String,
    on_conflict: String,
    project_root: Option<String>,
) -> Result<DisabledRecordDto, String> {
    tokio::task::spawn_blocking(move || {
        let roots = build_roots(project_root);
        let trackable = rebuild_trackable(&scope_root, &kind, &relative_path, &roots)?;
        let policy = parse_on_conflict(&on_conflict)?;
        artifact_lifecycle::enable_at(&trackable, policy, &roots)
            .map(DisabledRecordDto::from)
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_list_disabled(
    project_root: Option<String>,
) -> Result<Vec<DisabledRecordDto>, String> {
    tokio::task::spawn_blocking(move || {
        let roots = build_roots(project_root);
        artifact_lifecycle::list_disabled(&roots)
            .map(|rows| rows.into_iter().map(DisabledRecordDto::from).collect())
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_trash(
    scope_root: String,
    kind: String,
    relative_path: String,
    project_root: Option<String>,
) -> Result<TrashEntryDto, String> {
    tokio::task::spawn_blocking(move || {
        let roots = build_roots(project_root);
        let trackable = rebuild_trackable(&scope_root, &kind, &relative_path, &roots)?;
        let trash_root = artifact_lifecycle::default_trash_root();
        artifact_lifecycle::trash_at(&trackable, &trash_root, &roots)
            .map(TrashEntryDto::from)
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_list_trash() -> Result<Vec<TrashEntryDto>, String> {
    tokio::task::spawn_blocking(|| {
        let trash_root = artifact_lifecycle::default_trash_root();
        artifact_lifecycle::list_trash_at(&trash_root)
            .map(|rows| rows.into_iter().map(TrashEntryDto::from).collect())
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_restore_from_trash(
    trash_id: String,
    on_conflict: String,
) -> Result<RestoredArtifactDto, String> {
    tokio::task::spawn_blocking(move || {
        let trash_root = artifact_lifecycle::default_trash_root();
        let policy = parse_on_conflict(&on_conflict)?;
        artifact_lifecycle::restore_at(&trash_root, &trash_id, policy)
            .map(RestoredArtifactDto::from)
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_recover_trash(
    trash_id: String,
    confirmed_target_path: String,
    confirmed_kind: String,
    on_conflict: String,
) -> Result<RestoredArtifactDto, String> {
    tokio::task::spawn_blocking(move || {
        let trash_root = artifact_lifecycle::default_trash_root();
        let policy = parse_on_conflict(&on_conflict)?;
        let kind = parse_kind(&confirmed_kind)?;
        let target = PathBuf::from(confirmed_target_path);
        artifact_lifecycle::recover_at(&trash_root, &trash_id, &target, kind, policy)
            .map(RestoredArtifactDto::from)
            .map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_forget_trash(trash_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let trash_root = artifact_lifecycle::default_trash_root();
        artifact_lifecycle::forget_at(&trash_root, &trash_id).map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

#[tauri::command]
pub async fn artifact_purge_trash(older_than_days: u32) -> Result<u32, String> {
    tokio::task::spawn_blocking(move || {
        let trash_root = artifact_lifecycle::default_trash_root();
        artifact_lifecycle::purge_older_than(&trash_root, older_than_days).map_err(err_to_string)
    })
    .await
    .map_err(join_blocking_err)?
}

// Suppress unused-import warning when ArtifactKind isn't used directly
// in the source (it is — via parse_kind — but the `use` makes
// future additions easy).
#[allow(dead_code)]
fn _unused_artifact_kind_typed_use(_: ArtifactKind) {}
