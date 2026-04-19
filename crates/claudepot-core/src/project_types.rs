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
    /// True iff the project is safe to clean: either the source path is
    /// confirmed absent AND reachable, or the CC project dir is
    /// essentially empty (no sessions, no memory, tiny on disk).
    pub is_orphan: bool,
    /// True iff we were able to definitively classify the source path's
    /// existence. False when the path lives under an unmounted removable
    /// volume, an offline network share, or an ancestor whose status
    /// can't be stat'd (permission-denied, EIO). Unreachable projects
    /// are NEVER auto-cleaned.
    pub is_reachable: bool,
    /// True iff the CC project dir contains no sessions and no memory
    /// files and its total size is below one filesystem block. Empty
    /// dirs are always safe to remove regardless of source existence.
    pub is_empty: bool,
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
    /// Orphans skipped because a live CC session was detected against
    /// their project dir (lsof / process scan / heartbeat).
    pub orphans_skipped_live: usize,
    /// Candidates not counted as orphans because we could not
    /// definitively stat the source path (unmounted volume, permission
    /// denied). Surfaces so callers can surface "please mount /Volumes/X
    /// and re-run".
    pub unreachable_skipped: usize,
    /// `~/.claude.json` `projects[<original_path>]` entries removed.
    pub claude_json_entries_removed: usize,
    /// `~/.claude/history.jsonl` lines whose `project` field referenced
    /// a cleaned orphan.
    pub history_lines_removed: usize,
    /// Claudepot-owned artifacts removed (stale snapshots, abandoned
    /// journal sidecars whose sanitized name keys matched a cleaned
    /// orphan). Never touches in-flight journals; those are gated
    /// upstream.
    pub claudepot_artifacts_removed: usize,
    /// Paths of snapshots written during cleanup (config + history)
    /// so callers can surface recovery hints.
    pub snapshot_paths: Vec<PathBuf>,
    /// Number of orphans whose authoritative `original_path` matched
    /// the user's protected-paths set. The CC artifact dirs were still
    /// removed; sibling state (`~/.claude.json` + `history.jsonl`) was
    /// left intact, with a read-only recovery snapshot written so the
    /// user can restore the affected entries manually if they ever
    /// remove the path from the protected list.
    pub protected_paths_skipped: usize,
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
