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

#[derive(Debug, Clone)]
pub struct MoveArgs {
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub config_dir: PathBuf,
    /// Path to `~/.claude.json` (the config file, sibling to `~/.claude/`).
    /// `None` skips Phase 7 entirely — useful for tests and for future
    /// callers that want to opt out. CLI callers pass
    /// `Some(home.join(".claude.json"))`.
    pub claude_json_path: Option<PathBuf>,
    /// Directory for destructive-phase snapshots. Used by Phase 7 (and
    /// future P4-overwrite / P8). If `None`, snapshots go to
    /// `<config_dir>/claudepot/snapshots/`.
    pub snapshots_dir: Option<PathBuf>,
    pub no_move: bool,
    pub merge: bool,
    pub overwrite: bool,
    pub force: bool,
    pub dry_run: bool,
    /// Proceed despite pending rename journals. Surfaced into the
    /// new journal's `flags.ignore_pending_journals` for audit.
    pub ignore_pending_journals: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct MoveResult {
    /// When `dry_run=true` this is populated with the structured plan;
    /// otherwise `None`. GUI callers prefer this over parsing the
    /// formatted text in `warnings[0]`.
    pub dry_run_plan: Option<DryRunPlan>,
    pub actual_dir_moved: bool,
    pub cc_dir_renamed: bool,
    pub old_sanitized: Option<String>,
    pub new_sanitized: Option<String>,
    pub history_lines_updated: usize,
    /// P6 (session + subagent jsonl rewrite) stats.
    pub jsonl_files_scanned: usize,
    pub jsonl_files_modified: usize,
    pub jsonl_lines_rewritten: usize,
    /// Per-file errors from P6 (atomic-replace happened on the successful
    /// files; these are the ones that didn't persist).
    pub jsonl_errors: Vec<(PathBuf, String)>,
    /// P7 (~/.claude.json projects map key-rename) stats.
    pub config_key_renamed: bool,
    pub config_had_collision: bool,
    pub config_merged_keys: Vec<String>,
    pub config_snapshot_path: Option<PathBuf>,
    pub config_nested_rewrites: usize,
    /// P9 (project-local .claude/settings.json autoMemoryDirectory) rewrote.
    pub project_settings_rewritten: bool,
    /// P8 (auto-memory dir move on git-root change) stats.
    pub memory_git_root_changed: bool,
    pub memory_dir_moved: bool,
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
    /// P6 preview: count of jsonl files that would be rewritten.
    pub estimated_jsonl_files: usize,
    /// P7 preview: would the projects map key rename run?
    pub would_rewrite_claude_json: bool,
    /// P8 preview: would the auto-memory dir move run?
    pub would_move_memory_dir: bool,
    /// P9 preview: would project-local settings.json be touched?
    pub would_rewrite_project_settings: bool,
}
