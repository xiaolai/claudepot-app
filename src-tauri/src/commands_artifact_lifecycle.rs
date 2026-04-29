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
use std::path::{Component, PathBuf};

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
///
/// Renderer-supplied `project_root` is accepted only when it BOTH
///   1. passes shape validation (absolute, ends with `.claude`, no
///      `..`, not under `plugins/cache/`, not the user root), AND
///   2. is in the backend-discovered set of known project anchors
///      (`paths::discover_known_project_roots`, derived from the
///      session-index sweep).
///
/// Without the second check the validation would be circular: any
/// `.claude`-shaped directory the renderer named would get accepted
/// back as a writable scope. With it, the renderer's freedom is
/// reduced to *selecting* among roots the user has actually opened
/// — never inventing new ones.
///
/// Invalid candidates are silently dropped; the affected command
/// falls through to user-only scope and surfaces OutOfScope.
fn build_roots(project_root: Option<String>) -> ActiveRoots {
    let user_root = paths::claude_config_dir();
    let mut roots = ActiveRoots::user(user_root.clone());
    if let Some(p) = project_root.filter(|s| !s.is_empty()) {
        let candidate = PathBuf::from(p);
        if is_valid_project_root(&candidate, &user_root) {
            let known =
                claudepot_core::artifact_lifecycle::paths::discover_known_project_roots(&user_root);
            if known.iter().any(|k| k == &candidate) {
                roots = roots.with_project(candidate);
            }
            // Else: shape-valid but not in the known-set — drop
            // silently. The user can disable/trash by selecting a
            // file in the Config tree (which the backend already
            // knows about because the session index recorded the
            // project).
        }
    }
    // Managed-policy roots are added per-platform; left empty for
    // now since none of our shipped flows currently set them.
    roots
}

fn is_valid_project_root(candidate: &std::path::Path, user_root: &std::path::Path) -> bool {
    if !candidate.is_absolute() {
        return false;
    }
    // Reject any traversal segments.
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return false;
    }
    // Must end with `.claude`.
    if candidate.file_name().and_then(|s| s.to_str()) != Some(".claude") {
        return false;
    }
    // Must not be the user-scope root (already covered by ActiveRoots::user).
    if candidate == user_root {
        return false;
    }
    // Must not be under plugins/cache/.
    let plugin_cache_segment = ["plugins", "cache"];
    let parts: Vec<&str> = candidate
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    for win in parts.windows(2) {
        if win == plugin_cache_segment {
            return false;
        }
    }
    true
}

/// Validate that `relative_path` is a clean rel-path with no
/// traversal segments, no absolute roots, no Windows prefixes, and
/// no empty components. Rejects:
///   - absolute paths (`/foo`, `C:\foo`)
///   - parent dir refs (`..`)
///   - root dir refs (this implies an absolute path was passed)
///   - Windows prefixes (drive letters / UNC)
///   - empty components (consecutive separators)
///   - backslashes (the wire contract is forward-slash only)
///
/// The renderer is our own code, but the IPC trust model puts the
/// validation here so a future caller (a CLI, a third-party plugin
/// that issues invokes) can't smuggle traversal segments through.
fn validate_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.is_empty() {
        return Err("relative_path is empty".into());
    }
    if relative_path.contains('\\') {
        return Err("relative_path must use forward slashes only".into());
    }
    let p = std::path::Path::new(relative_path);
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            Component::ParentDir => {
                return Err(format!(
                    "relative_path must not contain `..`: {relative_path}"
                ));
            }
            Component::CurDir => {
                return Err(format!(
                    "relative_path must not contain `.`: {relative_path}"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "relative_path must be relative (no root): {relative_path}"
                ));
            }
        }
    }
    Ok(())
}

/// Validate that the `scope_root` the renderer claims is one of the
/// roots the backend knows about. Without this check, the renderer
/// could ask the backend to operate on an arbitrary directory shaped
/// like `<scope_root>/agents/...`. Plugin / managed-policy paths
/// stay refused at `classify_path` regardless.
fn validate_scope_root(scope_root: &str, roots: &ActiveRoots) -> Result<PathBuf, String> {
    let p = PathBuf::from(scope_root);
    let ok = roots.iter_scoped().any(|(_, root)| root == p.as_path());
    if !ok {
        return Err(format!("scope_root not in active roots: {}", p.display()));
    }
    Ok(p)
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
    validate_relative_path(relative_path)?;
    let scope_root_path = validate_scope_root(scope_root, roots)?;
    let abs = scope_root_path.join(kind.subdir()).join(relative_path);
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

/// Read the contents of a disabled artifact. The Config tree's
/// existing preview path can't reach `.disabled/` entries (they're
/// excluded from active discovery by design); this command is the
/// targeted read surface that drives the Disabled-scope preview pane.
///
/// Validates via the same classify_path gate used by mutating
/// commands, then returns the body as UTF-8 (with a small head
/// truncation guard).
#[tauri::command]
pub async fn artifact_disabled_preview(
    scope_root: String,
    kind: String,
    relative_path: String,
    project_root: Option<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let kind = parse_kind(&kind)?;
        validate_relative_path(&relative_path)?;
        let roots = build_roots(project_root);
        let scope_root_path = validate_scope_root(&scope_root, &roots)?;
        // The disabled location is what we read.
        let abs = scope_root_path
            .join(claudepot_core::artifact_lifecycle::DISABLED_DIR)
            .join(kind.subdir())
            .join(&relative_path);
        let trackable = claudepot_core::artifact_lifecycle::paths::classify_path(&abs, &roots)
            .map_err(|reason| reason.to_string())?;
        if !trackable.already_disabled {
            return Err(format!("not a disabled artifact: {}", abs.display()));
        }
        // For File payloads read the file itself; for Directory
        // payloads (Skill dir) read the SKILL.md inside.
        let read_path = match trackable.payload_kind {
            claudepot_core::artifact_lifecycle::paths::PayloadKind::File => abs,
            claudepot_core::artifact_lifecycle::paths::PayloadKind::Directory => {
                abs.join("SKILL.md")
            }
        };
        // 256 KiB head cap — same order of magnitude as the existing
        // ConfigPreview body cap; large markdowns get truncated.
        // Stream the read with `take(N + 1)` so a multi-GB
        // accidentally-trashed file doesn't spike memory or block
        // the spawn_blocking worker pool.
        const PREVIEW_HEAD_BYTES: usize = 256 * 1024;
        use std::io::Read;
        let file = std::fs::File::open(&read_path).map_err(|e| format!("read open failed: {e}"))?;
        let mut bytes = Vec::with_capacity(PREVIEW_HEAD_BYTES.min(64 * 1024));
        file.take(PREVIEW_HEAD_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read failed: {e}"))?;
        let truncated = bytes.len() > PREVIEW_HEAD_BYTES;
        if truncated {
            bytes.truncate(PREVIEW_HEAD_BYTES);
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        Ok::<_, String>(if truncated {
            format!("{body}\n\n…(truncated)")
        } else {
            body
        })
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
