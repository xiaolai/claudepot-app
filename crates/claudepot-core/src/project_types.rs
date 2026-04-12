//! Data types for the project module.

use serde::Serialize;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub sanitized_name: String,
    pub original_path: String,
    pub session_count: usize,
    pub memory_file_count: usize,
    pub total_size_bytes: u64,
    pub last_modified: Option<SystemTime>,
    pub is_orphan: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectDetail {
    pub info: ProjectInfo,
    pub sessions: Vec<SessionInfo>,
    pub memory_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub file_size: u64,
    pub last_modified: Option<SystemTime>,
}

pub struct MoveArgs {
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub config_dir: PathBuf,
    pub no_move: bool,
    pub merge: bool,
    pub overwrite: bool,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct MoveResult {
    pub actual_dir_moved: bool,
    pub cc_dir_renamed: bool,
    pub old_sanitized: Option<String>,
    pub new_sanitized: Option<String>,
    pub history_lines_updated: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct CleanResult {
    pub orphans_found: usize,
    pub orphans_removed: usize,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DryRunPlan {
    pub would_move_dir: bool,
    pub old_cc_dir: String,
    pub new_cc_dir: String,
    pub session_count: usize,
    pub cc_dir_size: u64,
    pub estimated_history_lines: usize,
    pub conflict: Option<String>,
}
