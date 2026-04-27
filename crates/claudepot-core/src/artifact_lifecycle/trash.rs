//! Trash + restore + recover + forget + purge — public API surface.
//! Listing/IO/hashing live in `trash_listing.rs` + `trash_io.rs`.
//! See `dev-docs/artifact-lifecycle-plan.md` for the staging protocol
//! (two-phase: stage → write manifest → remove source → commit-rename).

use crate::artifact_lifecycle::error::{LifecycleError, RefuseReason, Result};
use crate::artifact_lifecycle::paths::{
    enabled_target_for, ActiveRoots, ArtifactKind, PayloadKind, Scope, Trackable,
};
use crate::artifact_lifecycle::trash_io::{
    hash_payload, move_or_copy, remove_source, write_manifest_atomic,
};
use crate::artifact_lifecycle::trash_listing::{list_at as listing_list_at, read_one};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// JSON-stable manifest written into every trash entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashManifest {
    pub version: u32,
    pub id: String,
    pub trashed_at_ms: i64,
    pub scope: Scope,
    pub scope_root: PathBuf,
    pub kind: ArtifactKind,
    pub relative_path: String,
    pub original_path: PathBuf,
    pub source_basename: String,
    pub payload_kind: PayloadKind,
    pub byte_count: u64,
    pub sha256: Option<String>,
}

const MANIFEST_VERSION: u32 = 1;

/// Health status surfaced for each trash entry by `list_trash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TrashState {
    Healthy,
    MissingManifest,
    MissingPayload,
    OrphanPayload,
    AbandonedStaging,
    /// Manifest parses and the payload exists, but the byte count
    /// (and sha256 when recorded) doesn't match what the manifest
    /// claims. Surfaced as a distinct state so the user knows the
    /// stored content was modified after trashing — restore is
    /// refused (manual recover or forget required).
    Tampered,
}

impl TrashState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::MissingManifest => "missing_manifest",
            Self::MissingPayload => "missing_payload",
            Self::OrphanPayload => "orphan_payload",
            Self::AbandonedStaging => "abandoned_staging",
            Self::Tampered => "tampered",
        }
    }
}

/// One row returned by `list_trash`. `manifest` is `None` for
/// entries whose state precludes parsing (MissingManifest /
/// AbandonedStaging without a written manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashEntry {
    pub id: String,
    pub entry_dir: PathBuf,
    pub state: TrashState,
    pub manifest: Option<TrashManifest>,
    /// Wall-clock ms of the directory's mtime — used as a fallback
    /// "when was this trashed" when the manifest is missing/corrupt.
    pub directory_mtime_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoredArtifact {
    pub id: String,
    pub final_path: PathBuf,
}

/// Trash an artifact: source must be the active path (not a
/// `.disabled/` entry). The Trackable already names the canonical
/// triple; we read it back from there.
pub fn trash_at(
    trackable: &Trackable,
    trash_root: &Path,
    _roots: &ActiveRoots,
) -> Result<TrashEntry> {
    if trackable.already_disabled {
        // Trashing a disabled artifact is allowed — we still trash
        // the disabled-on-disk path. Use the disabled location as
        // the source.
        return trash_from_path(
            trackable,
            &crate::artifact_lifecycle::paths::disabled_target_for(trackable),
            trash_root,
        );
    }
    let source = enabled_target_for(trackable);
    trash_from_path(trackable, &source, trash_root)
}

fn trash_from_path(
    trackable: &Trackable,
    source: &Path,
    trash_root: &Path,
) -> Result<TrashEntry> {
    if !source.exists() {
        return Err(LifecycleError::SourceMissing(source.to_path_buf()));
    }
    let id = Uuid::new_v4().to_string();
    std::fs::create_dir_all(trash_root).map_err(LifecycleError::io("create trash root"))?;

    let staging = trash_root.join(format!("{id}.staging"));
    let committed = trash_root.join(&id);
    std::fs::create_dir_all(&staging).map_err(LifecycleError::io("create staging dir"))?;

    let basename = source
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| LifecycleError::Refused(RefuseReason::WrongKind {
            path: source.to_path_buf(),
        }))?;

    let payload_dir = staging.join("payload");
    std::fs::create_dir_all(&payload_dir).map_err(LifecycleError::io("create payload dir"))?;
    let payload_target = payload_dir.join(&basename);

    // Step 2: move (or copy) the artifact into staging payload.
    move_or_copy(source, &payload_target, trackable.payload_kind)?;

    // Compute byte_count + sha256 from the staged payload (so the
    // manifest reflects what we actually wrote).
    let (byte_count, sha) = hash_payload(&payload_target, trackable.payload_kind)?;

    let manifest = TrashManifest {
        version: MANIFEST_VERSION,
        id: id.clone(),
        trashed_at_ms: Utc::now().timestamp_millis(),
        scope: trackable.scope,
        scope_root: trackable.scope_root.clone(),
        kind: trackable.kind,
        relative_path: trackable.relative_path.clone(),
        original_path: source.to_path_buf(),
        source_basename: basename,
        payload_kind: trackable.payload_kind,
        byte_count,
        sha256: sha,
    };

    // Step 3: write manifest atomically (tempfile + rename).
    write_manifest_atomic(&staging, &manifest)?;

    // Step 4: remove source if we copied (move_or_copy already
    // removed for a same-volume rename). For copy paths, the source
    // still exists.
    if source.exists() {
        remove_source(source, trackable.payload_kind)?;
    }

    // Step 5: commit-rename staging → committed.
    std::fs::rename(&staging, &committed).map_err(LifecycleError::io("commit trash entry"))?;

    Ok(TrashEntry {
        id,
        entry_dir: committed,
        state: TrashState::Healthy,
        manifest: Some(manifest),
        directory_mtime_ms: None,
    })
}

/// Restore a `Healthy` trash entry to its original location.
pub fn restore_at(
    trash_root: &Path,
    trash_id: &str,
    on_conflict: super::disable::OnConflict,
) -> Result<RestoredArtifact> {
    validate_trash_id(trash_id)?;
    let entry = read_one(trash_root, trash_id)?;
    if entry.state != TrashState::Healthy {
        return Err(LifecycleError::WrongTrashState {
            state: entry.state.as_str(),
            action: "restore",
        });
    }
    let manifest = entry.manifest.as_ref().expect("Healthy implies manifest");
    if !manifest.scope_root.exists() {
        return Err(LifecycleError::ScopeRootMissing(manifest.scope_root.clone()));
    }
    // Hold the scope lock for the manifest's destination scope_root
    // so collision check + rename + a racing disable/enable on the
    // same scope don't interleave. This is the same primitive
    // disable_at uses; restore must respect it.
    let _lock = super::scope_lock::acquire(&manifest.scope_root)?;

    let payload_src = entry
        .entry_dir
        .join("payload")
        .join(&manifest.source_basename);

    // Full sha256 + byte_count + basename verification at the moment
    // it matters most — right before we write to the active scope.
    // The list_at fast-path only checked size; this catches a payload
    // whose bytes were modified in-place after trashing without
    // changing the file size.
    if !super::trash_listing::verify_against_manifest_full(&payload_src, manifest) {
        return Err(LifecycleError::WrongTrashState {
            state: "tampered",
            action: "restore",
        });
    }

    let target = super::disable::resolve_collision_pub(&manifest.original_path, on_conflict)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(LifecycleError::io("create restore parent"))?;
    }
    // No-replace rename first; on EXDEV (cross-volume) fall back to
    // copy with a final existence check inside the lock so the
    // copy can't silently overwrite a racing disable's destination.
    match super::disable::rename_no_replace_pub(&payload_src, &target) {
        Ok(()) => {}
        Err(LifecycleError::Conflict(_)) => {
            return Err(LifecycleError::Conflict(target));
        }
        Err(_) => {
            if target.exists() {
                return Err(LifecycleError::Conflict(target));
            }
            move_or_copy(&payload_src, &target, manifest.payload_kind)?;
        }
    }
    // After restore we drop the trash entry.
    std::fs::remove_dir_all(&entry.entry_dir)
        .map_err(LifecycleError::io("drop restored trash entry"))?;
    Ok(RestoredArtifact {
        id: trash_id.to_string(),
        final_path: target,
    })
}

/// Recover a `MissingManifest` or `AbandonedStaging` entry by
/// promoting it to a synthetic manifest then performing the restore.
/// The user has confirmed the target path and kind via UI dialog.
pub fn recover_at(
    trash_root: &Path,
    trash_id: &str,
    confirmed_target: &Path,
    confirmed_kind: ArtifactKind,
    on_conflict: super::disable::OnConflict,
) -> Result<RestoredArtifact> {
    validate_trash_id(trash_id)?;
    let entry = read_one(trash_root, trash_id)?;
    match entry.state {
        TrashState::MissingManifest | TrashState::AbandonedStaging => {}
        other => {
            return Err(LifecycleError::WrongTrashState {
                state: other.as_str(),
                action: "recover",
            })
        }
    }

    // Promote AbandonedStaging to a "regular" entry by renaming.
    let entry_dir = if entry.entry_dir.extension().and_then(|s| s.to_str()) == Some("staging") {
        let promoted = trash_root.join(trash_id);
        std::fs::rename(&entry.entry_dir, &promoted)
            .map_err(LifecycleError::io("promote staging"))?;
        promoted
    } else {
        entry.entry_dir.clone()
    };

    let payload_dir = entry_dir.join("payload");
    let mut children: Vec<PathBuf> = std::fs::read_dir(&payload_dir)
        .map_err(LifecycleError::io("read payload"))?
        .filter_map(|e| e.ok().map(|d| d.path()))
        .collect();
    if children.len() != 1 {
        return Err(LifecycleError::RecoveryAmbiguous(format!(
            "payload contains {} entries",
            children.len()
        )));
    }
    let payload_src = children.remove(0);
    let basename = payload_src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| LifecycleError::RecoveryAmbiguous("no basename".into()))?;
    let payload_kind = if payload_src.is_dir() {
        PayloadKind::Directory
    } else {
        PayloadKind::File
    };
    let (byte_count, sha) = hash_payload(&payload_src, payload_kind)?;

    // Derive scope_root + relative_path by walking up from the
    // confirmed target through the kind subdir. For
    // `/repo/.claude/agents/team/foo.md` with kind=agent the
    // expected scope_root is `/repo/.claude` and rel-path is
    // `team/foo.md`. Falls back to the parent dir when the
    // expected layout doesn't match — recover is a manual flow,
    // the user can correct via re-recover.
    let kind_subdir = confirmed_kind.subdir();
    let (scope_root, relative_path) = derive_scope_and_rel(confirmed_target, kind_subdir)
        .unwrap_or_else(|| {
            (
                confirmed_target
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| confirmed_target.to_path_buf()),
                basename.clone(),
            )
        });

    let manifest = TrashManifest {
        version: MANIFEST_VERSION,
        id: trash_id.to_string(),
        trashed_at_ms: entry.directory_mtime_ms.unwrap_or_else(|| Utc::now().timestamp_millis()),
        scope: Scope::User, // unknown — best-effort. The (scope_root, kind, rel) triple is what restore actually uses.
        scope_root,
        kind: confirmed_kind,
        relative_path,
        original_path: confirmed_target.to_path_buf(),
        source_basename: basename,
        payload_kind,
        byte_count,
        sha256: sha,
    };
    write_manifest_atomic(&entry_dir, &manifest)?;

    // Now restore from the synthesized manifest.
    restore_at(trash_root, trash_id, on_conflict)
}

/// Forget a trash entry (used for MissingPayload, OrphanPayload, or
/// last-resort cleanup of any entry).
pub fn forget_at(trash_root: &Path, trash_id: &str) -> Result<()> {
    validate_trash_id(trash_id)?;
    let dir_normal = trash_root.join(trash_id);
    let dir_staging = trash_root.join(format!("{trash_id}.staging"));
    let dir = if dir_normal.exists() {
        dir_normal
    } else if dir_staging.exists() {
        dir_staging
    } else {
        return Err(LifecycleError::TrashEntryNotFound(trash_id.to_string()));
    };
    std::fs::remove_dir_all(&dir).map_err(LifecycleError::io("forget trash entry"))?;
    Ok(())
}

/// Purge entries older than `older_than_days`. Only `Healthy` entries
/// participate — corrupt entries stay until manually forgotten so the
/// user can investigate.
pub fn purge_older_than(trash_root: &Path, older_than_days: u32) -> Result<u32> {
    if !trash_root.exists() {
        return Ok(0);
    }
    let cutoff = Utc::now().timestamp_millis() - (older_than_days as i64) * 86_400_000;
    let mut purged = 0u32;
    let entries = list_at(trash_root)?;
    for entry in entries {
        if entry.state != TrashState::Healthy {
            continue;
        }
        let ts = entry
            .manifest
            .as_ref()
            .map(|m| m.trashed_at_ms)
            .unwrap_or(0);
        if ts < cutoff {
            std::fs::remove_dir_all(&entry.entry_dir)
                .map_err(LifecycleError::io("purge trash entry"))?;
            purged += 1;
        }
    }
    Ok(purged)
}

/// Walk `trash_root`, return one row per entry with its state.
/// Thin re-export so callers don't need to import `trash_listing`
/// directly — the public surface stays at `artifact_lifecycle::list_trash_at`.
pub fn list_at(trash_root: &Path) -> Result<Vec<TrashEntry>> {
    listing_list_at(trash_root)
}

/// Walk up from `confirmed_target` to find the `<kind_subdir>/` ancestor
/// and split into (scope_root, relative_path-under-kind). Used by
/// `recover_at` to synthesize a correct manifest from the user's
/// confirmed target instead of mis-attributing scope_root to the
/// file's parent directory.
fn derive_scope_and_rel(
    confirmed_target: &Path,
    kind_subdir: &str,
) -> Option<(PathBuf, String)> {
    let mut comps: Vec<&std::ffi::OsStr> = confirmed_target
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    let kind_idx = comps.iter().rposition(|s| {
        s.to_str().map(|x| x == kind_subdir).unwrap_or(false)
    })?;
    if kind_idx == 0 {
        return None;
    }
    // scope_root = path up to (but not including) the kind subdir.
    let mut scope_root = PathBuf::new();
    for c in confirmed_target.components() {
        match c {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                scope_root.push(c.as_os_str());
            }
            std::path::Component::Normal(s) => {
                if s == comps[kind_idx] {
                    break;
                }
                scope_root.push(s);
            }
            _ => {}
        }
        // Stop when we've added the segment immediately before kind_subdir.
        let depth = scope_root
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .count();
        if depth == kind_idx {
            break;
        }
    }
    let rel: String = comps
        .split_off(kind_idx + 1)
        .into_iter()
        .filter_map(|s| s.to_str().map(str::to_string))
        .collect::<Vec<_>>()
        .join("/");
    if rel.is_empty() {
        return None;
    }
    Some((scope_root, rel))
}

/// Validate `trash_id` strictly as a UUID before any path join.
/// All trash mutators MUST call this first — the trash_id flows
/// from the renderer (untrusted layer per the IPC trust model) and
/// is path-joined inside core; without validation it's a path-
/// traversal vector.
///
/// Accepts the canonical 36-char UUID hex form (8-4-4-4-12). Anything
/// containing `/`, `\`, `..`, or other non-hex / non-hyphen chars is
/// rejected.
pub(super) fn validate_trash_id(trash_id: &str) -> Result<()> {
    if uuid::Uuid::parse_str(trash_id).is_err() {
        return Err(LifecycleError::InvalidTrashId(trash_id.to_string()));
    }
    Ok(())
}

// Listing (`list_at`, `read_one`, `classify_entry`), IO helpers
// (`write_manifest_atomic`, `move_or_copy`, `copy_dir_recursive`,
// `remove_source`), and payload hashing live in the sibling
// `trash_listing.rs` and `trash_io.rs` shards. This file is the
// public API surface only.
