//! Trash listing + per-entry classification.
//!
//! Reads the on-disk trash directory and reports each entry's
//! health state. The classification is intentionally
//! generous-on-failure: an entry whose manifest fails to parse is
//! `MissingManifest`, not an error, so the user can still see the
//! row and decide what to do (recover / forget).

use crate::artifact_lifecycle::error::{LifecycleError, Result};
use crate::artifact_lifecycle::paths::PayloadKind;
use crate::artifact_lifecycle::trash::{TrashEntry, TrashManifest, TrashState};
use crate::artifact_lifecycle::trash_io;
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
///
/// Defense-in-depth: also runs `validate_trash_id` here so any
/// path that bypasses the public API (e.g., a future internal
/// caller) still can't smuggle traversal segments through.
pub(super) fn read_one(trash_root: &Path, trash_id: &str) -> Result<TrashEntry> {
    crate::artifact_lifecycle::trash::validate_trash_id(trash_id)?;
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

    let mut state = if !payload_exists {
        TrashState::MissingPayload
    } else if !manifest_exists || manifest.is_none() {
        TrashState::MissingManifest
    } else if payload_children.len() != 1 {
        TrashState::OrphanPayload
    } else {
        TrashState::Healthy
    };

    // For Healthy candidates, also verify the payload size matches
    // the manifest (cheap stat) and — for File payloads with a
    // recorded sha256 — recompute the digest (one streaming pass).
    // Mismatch downgrades to Tampered so restore is refused and the
    // user is forced to investigate.
    if state == TrashState::Healthy {
        if let Some(m) = manifest.as_ref() {
            if !verify_against_manifest(&payload_children[0], m) {
                state = TrashState::Tampered;
            }
        }
    }

    TrashEntry {
        id,
        entry_dir: path,
        state,
        manifest,
        directory_mtime_ms,
    }
}

/// Cheap manifest cross-check — basename + byte_count only. The full
/// sha256 verification is deferred to `verify_against_manifest_full`,
/// called by `restore_at` before it touches the filesystem so we
/// pay the digest cost once per restore instead of once per listing.
///
/// `list_at` runs this on every Healthy candidate; a mismatch
/// downgrades to `Tampered`. The user sees the same UX as before —
/// Tampered entries refuse restore — but the listing scales with
/// the number of trash entries, not their byte sizes.
fn verify_against_manifest(payload_path: &Path, manifest: &TrashManifest) -> bool {
    if payload_path
        .file_name()
        .and_then(|n| n.to_str())
        != Some(manifest.source_basename.as_str())
    {
        return false;
    }
    let observed_size = match observed_size(payload_path, manifest.payload_kind) {
        Some(n) => n,
        None => return false,
    };
    observed_size == manifest.byte_count
}

/// Full verification including sha256 (for File payloads with a
/// recorded digest). Called by `restore_at` immediately before the
/// rename so a Tampered entry that snuck past the cheap list check
/// still gets caught at the moment that matters most.
pub(super) fn verify_against_manifest_full(
    payload_path: &Path,
    manifest: &TrashManifest,
) -> bool {
    if !verify_against_manifest(payload_path, manifest) {
        return false;
    }
    if manifest.payload_kind == PayloadKind::File {
        if let Some(expected) = manifest.sha256.as_deref() {
            match trash_io::stream_hash_file(payload_path) {
                Ok((_, hex)) => return hex == expected,
                Err(_) => return false,
            }
        }
    }
    true
}

fn observed_size(payload_path: &Path, kind: PayloadKind) -> Option<u64> {
    match kind {
        PayloadKind::File => {
            // Use `metadata` (follows symlinks) — matches the
            // producer side, where `stream_hash_file` opens the file
            // through `File::open` (also follows symlinks). If the
            // two used different policies a File payload that's a
            // symlink would always be Tampered: producer hashes the
            // target, verifier sizes the link.
            std::fs::metadata(payload_path).ok().map(|m| m.len())
        }
        PayloadKind::Directory => {
            let mut total = 0u64;
            // Use the same lstat-based walker the producer used so
            // the count is comparable byte-for-byte.
            trash_io::walk_dir(payload_path, &mut |k, p| match k {
                trash_io::WalkEntryKind::File | trash_io::WalkEntryKind::Symlink => {
                    if let Ok(meta) = std::fs::symlink_metadata(p) {
                        total += meta.len();
                    }
                }
                trash_io::WalkEntryKind::Directory => {}
            })
            .ok()?;
            Some(total)
        }
    }
}
