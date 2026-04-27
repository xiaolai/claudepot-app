//! DTOs for the artifact-lifecycle Tauri surface.
//!
//! Mirrors `claudepot_core::artifact_lifecycle` types as serializable
//! shapes. Path types serialize as strings (display-form) so the JS
//! side gets stable strings rather than platform-specific Path
//! variants.

use claudepot_core::artifact_lifecycle::{
    paths::{ArtifactKind, PayloadKind, Scope, Trackable},
    DisabledRecord, RestoredArtifact, TrashEntry, TrashState,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DisabledRecordDto {
    pub scope: &'static str,
    pub scope_root: String,
    pub kind: &'static str,
    pub name: String,
    pub original_path: String,
    pub current_path: String,
    pub payload_kind: &'static str,
}

impl From<DisabledRecord> for DisabledRecordDto {
    fn from(r: DisabledRecord) -> Self {
        Self {
            scope: scope_str(r.scope),
            scope_root: r.scope_root.display().to_string(),
            kind: r.kind.as_str(),
            name: r.name,
            original_path: r.original_path.display().to_string(),
            current_path: r.current_path.display().to_string(),
            payload_kind: payload_kind_str(r.payload_kind),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TrashEntryDto {
    pub id: String,
    pub entry_dir: String,
    pub state: &'static str,
    /// Wall-clock ms — preferred from the manifest, falls back to
    /// the directory mtime when the manifest is missing/corrupt.
    pub trashed_at_ms: Option<i64>,
    /// `None` for non-Healthy entries that lack a parsed manifest.
    pub manifest: Option<TrashManifestDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrashManifestDto {
    pub scope: &'static str,
    pub scope_root: String,
    pub kind: &'static str,
    pub relative_path: String,
    pub original_path: String,
    pub source_basename: String,
    pub payload_kind: &'static str,
    pub byte_count: u64,
    pub sha256: Option<String>,
}

impl From<TrashEntry> for TrashEntryDto {
    fn from(e: TrashEntry) -> Self {
        let manifest_dto = e.manifest.as_ref().map(|m| TrashManifestDto {
            scope: scope_str(m.scope),
            scope_root: m.scope_root.display().to_string(),
            kind: m.kind.as_str(),
            relative_path: m.relative_path.clone(),
            original_path: m.original_path.display().to_string(),
            source_basename: m.source_basename.clone(),
            payload_kind: payload_kind_str(m.payload_kind),
            byte_count: m.byte_count,
            sha256: m.sha256.clone(),
        });
        let trashed_at_ms = e
            .manifest
            .as_ref()
            .map(|m| m.trashed_at_ms)
            .or(e.directory_mtime_ms);
        Self {
            id: e.id,
            entry_dir: e.entry_dir.display().to_string(),
            state: trash_state_str(e.state),
            trashed_at_ms,
            manifest: manifest_dto,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoredArtifactDto {
    pub id: String,
    pub final_path: String,
}

impl From<RestoredArtifact> for RestoredArtifactDto {
    fn from(r: RestoredArtifact) -> Self {
        Self {
            id: r.id,
            final_path: r.final_path.display().to_string(),
        }
    }
}

/// Shape returned by `artifact_classify_path` so the JS side can
/// pre-flight the action without needing to interpret refusals
/// from a thrown error string.
#[derive(Debug, Clone, Serialize)]
pub struct ClassifyPathDto {
    pub trackable: Option<TrackableDto>,
    pub refused: Option<String>,
    pub already_disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackableDto {
    pub scope: &'static str,
    pub scope_root: String,
    pub kind: &'static str,
    pub relative_path: String,
    pub payload_kind: &'static str,
}

impl From<&Trackable> for TrackableDto {
    fn from(t: &Trackable) -> Self {
        Self {
            scope: scope_str(t.scope),
            scope_root: t.scope_root.display().to_string(),
            kind: t.kind.as_str(),
            relative_path: t.relative_path.clone(),
            payload_kind: payload_kind_str(t.payload_kind),
        }
    }
}

fn scope_str(s: Scope) -> &'static str {
    // `Scope` is `#[non_exhaustive]`; default to "user" for any
    // future-added variant so the JS side never sees an empty string.
    match s {
        Scope::User => "user",
        Scope::Project => "project",
        _ => "user",
    }
}

fn payload_kind_str(p: PayloadKind) -> &'static str {
    match p {
        PayloadKind::File => "file",
        PayloadKind::Directory => "directory",
    }
}

fn trash_state_str(s: TrashState) -> &'static str {
    s.as_str()
}

/// Parse the on-the-wire kind string into the core enum.
pub fn parse_kind(s: &str) -> Result<ArtifactKind, String> {
    ArtifactKind::parse(s).ok_or_else(|| format!("unknown artifact kind: {s}"))
}
