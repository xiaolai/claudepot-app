//! DTO payloads for the `config-tree-patch` event.
//!
//! Split out of `config_watch.rs` so the watcher glue there stays focused
//! on the debouncer state machine rather than DTO shape. The DTOs mirror
//! the scan-time shapes from `commands_config` so the React reducer can
//! handle both with the same code path.

use crate::config_dto::FileNodeDto;
use serde::Serialize;

/// DTO payload for the `config-tree-patch` event.
#[derive(Serialize, Clone, Debug)]
pub struct ConfigTreePatchEvent {
    pub generation: u64,
    pub added: Vec<AddedFileDto>,
    pub updated: Vec<FileNodeDto>,
    pub removed: Vec<String>,
    pub reordered: Vec<ReorderedDto>,
    pub full_snapshot: Option<ConfigTreeSnapshotDto>,
    pub dirty_during_emit: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct AddedFileDto {
    pub parent_scope_id: String,
    pub file: FileNodeDto,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReorderedDto {
    pub parent_scope_id: String,
    pub child_ids: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConfigTreeSnapshotDto {
    // Reuse the same shape the top-level config_scan returns so the
    // React reducer can handle both with the same code path.
    pub scopes: Vec<ScopeSnapshotDto>,
    pub cwd: String,
    pub project_root: String,
    pub config_home_dir: String,
    pub memory_slug: String,
    pub memory_slug_lossy: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScopeSnapshotDto {
    pub id: String,
    pub label: String,
    pub scope_type: String,
    pub recursive_count: usize,
    pub files: Vec<FileNodeDto>,
}
