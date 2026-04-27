//! Trash listing + per-entry classification.
//!
//! Reads the on-disk trash directory and reports each entry's
//! health state. The classification is intentionally
//! generous-on-failure: an entry whose manifest fails to parse is
//! `MissingManifest`, not an error, so the user can still see the
//! row and decide what to do (recover / forget).

use crate::artifact_lifecycle::error::{LifecycleError, Result};
use crate::artifact_lifecycle::trash::{TrashEntry, TrashManifest, TrashState};
use std::path::{Path, PathBuf};

/// Walk `trash_root`, return one row per entry with its state.
pub fn list_at(trash_root: &Path) -> Result<Vec<TrashEntry>> {
    if !trash_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(trash_root).map_err(LifecycleError::io("read trash root"))? {
        let entry = entry.map_err(LifecycleError::io("read trash entry"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            // Hidden siblings (e.g., .DS_Store) — skip without
            // surfacing as an error.
            continue;
        }
        out.push(classify_entry(path, &name));
    }
    Ok(out)
}

/// Read a single entry by id (with or without the `.staging`
/// suffix). Returns `Err(TrashEntryNotFound)` when neither form
/// exists on disk.
pub(super) fn read_one(trash_root: &Path, trash_id: &str) -> Result<TrashEntry> {
    let normal = trash_root.join(trash_id);
    let staging = trash_root.join(format!("{trash_id}.staging"));
    let (path, name) = if normal.is_dir() {
        (normal, trash_id.to_string())
    } else if staging.is_dir() {
        (staging, format!("{trash_id}.staging"))
    } else {
        return Err(LifecycleError::TrashEntryNotFound(trash_id.to_string()));
    };
    Ok(classify_entry(path, &name))
}

/// Inspect a single trash entry directory and return its state +
/// parsed manifest (when present). Pure I/O — no mutation.
pub(super) fn classify_entry(path: PathBuf, name: &str) -> TrashEntry {
    let directory_mtime_ms = path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);

    if name.ends_with(".staging") {
        let id = name.strip_suffix(".staging").unwrap_or(name).to_string();
        return TrashEntry {
            id,
            entry_dir: path,
            state: TrashState::AbandonedStaging,
            manifest: None,
            directory_mtime_ms,
        };
    }

    let id = name.to_string();
    let manifest_path = path.join("manifest.json");
    let payload_dir = path.join("payload");

    let manifest_exists = manifest_path.exists();
    let payload_exists = payload_dir.exists();
    let payload_children: Vec<PathBuf> = std::fs::read_dir(&payload_dir)
        .ok()
        .map(|it| it.filter_map(|e| e.ok().map(|d| d.path())).collect())
        .unwrap_or_default();

    let manifest = if manifest_exists {
        std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str::<TrashManifest>(&s).ok())
    } else {
        None
    };

    let state = if !payload_exists {
        TrashState::MissingPayload
    } else if !manifest_exists || manifest.is_none() {
        TrashState::MissingManifest
    } else if payload_children.len() != 1 {
        TrashState::OrphanPayload
    } else {
        TrashState::Healthy
    };

    TrashEntry {
        id,
        entry_dir: path,
        state,
        manifest,
        directory_mtime_ms,
    }
}
