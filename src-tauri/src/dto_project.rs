//! Project DTOs — read-only surface for the Projects section.

use crate::dto::system_time_to_ms;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ProjectInfoDto {
    pub sanitized_name: String,
    pub original_path: String,
    pub session_count: usize,
    pub memory_file_count: usize,
    pub total_size_bytes: u64,
    /// ms since epoch; null if the dir has never been modified (new).
    pub last_modified_ms: Option<i64>,
    pub is_orphan: bool,
    /// Could the source path be definitively stat'd? False for projects
    /// on unmounted removable volumes / offline shares / permission-
    /// denied ancestors. The GUI should surface this instead of showing
    /// a misleading "source missing" error.
    pub is_reachable: bool,
    /// CC project dir is effectively empty (no sessions, no memory).
    /// Useful for callers that want to show a distinct "abandoned"
    /// label from the standard "source deleted" orphan.
    pub is_empty: bool,
}

impl From<&claudepot_core::project_types::ProjectInfo> for ProjectInfoDto {
    fn from(p: &claudepot_core::project_types::ProjectInfo) -> Self {
        Self {
            sanitized_name: p.sanitized_name.clone(),
            original_path: p.original_path.clone(),
            session_count: p.session_count,
            memory_file_count: p.memory_file_count,
            total_size_bytes: p.total_size_bytes,
            last_modified_ms: system_time_to_ms(p.last_modified),
            is_orphan: p.is_orphan,
            is_reachable: p.is_reachable,
            is_empty: p.is_empty,
        }
    }
}

#[derive(Serialize)]
pub struct SessionInfoDto {
    pub session_id: String,
    pub file_size: u64,
    pub last_modified_ms: Option<i64>,
}

impl From<&claudepot_core::project_types::SessionInfo> for SessionInfoDto {
    fn from(s: &claudepot_core::project_types::SessionInfo) -> Self {
        Self {
            session_id: s.session_id.clone(),
            file_size: s.file_size,
            last_modified_ms: system_time_to_ms(s.last_modified),
        }
    }
}

#[derive(Serialize)]
pub struct ProjectDetailDto {
    pub info: ProjectInfoDto,
    pub sessions: Vec<SessionInfoDto>,
    pub memory_files: Vec<String>,
}

impl From<&claudepot_core::project_types::ProjectDetail> for ProjectDetailDto {
    fn from(p: &claudepot_core::project_types::ProjectDetail) -> Self {
        Self {
            info: ProjectInfoDto::from(&p.info),
            sessions: p.sessions.iter().map(SessionInfoDto::from).collect(),
            memory_files: p.memory_files.clone(),
        }
    }
}

/// What `project_clean_preview` returns — the list the user needs to
/// see before confirming a destructive clean. Mirrors
/// `CleanResult { orphans_found, unreachable_skipped, ... }` shape
/// but also ships the per-project list so the UI can render badges.
#[derive(Serialize)]
pub struct CleanPreviewDto {
    pub orphans: Vec<ProjectInfoDto>,
    pub orphans_found: usize,
    pub unreachable_skipped: usize,
    /// Sum of `total_size_bytes` across the candidate orphans. The UI
    /// displays this in the confirmation copy so users can judge the
    /// impact before pressing Confirm.
    pub total_bytes: u64,
    /// How many of the listed candidates have an authoritative source
    /// path that's in the user's protected-paths set. Their CC artifact
    /// dir will still be removed, but `~/.claude.json` and
    /// `history.jsonl` entries for those paths will be preserved. The
    /// confirmation modal uses this to disclose the carve-out before
    /// Confirm.
    pub protected_count: usize,
}

// Note: the post-clean result DTO lives in `ops::CleanResultSummary`
// because the clean op is event-driven (tokio task + op-progress
// events), so its result must attach to a `RunningOpInfo` rather than
// being returned inline from a Tauri command. Keep the clean preview
// DTO here (sync read-only call).

#[derive(Serialize)]
pub struct DryRunPlanDto {
    pub would_move_dir: bool,
    pub old_cc_dir: String,
    pub new_cc_dir: String,
    pub session_count: usize,
    pub cc_dir_size: u64,
    pub estimated_history_lines: usize,
    pub conflict: Option<String>,
    pub estimated_jsonl_files: usize,
    pub would_rewrite_claude_json: bool,
    pub would_move_memory_dir: bool,
    pub would_rewrite_project_settings: bool,
}

impl From<&claudepot_core::project_types::DryRunPlan> for DryRunPlanDto {
    fn from(p: &claudepot_core::project_types::DryRunPlan) -> Self {
        Self {
            would_move_dir: p.would_move_dir,
            old_cc_dir: p.old_cc_dir.clone(),
            new_cc_dir: p.new_cc_dir.clone(),
            session_count: p.session_count,
            cc_dir_size: p.cc_dir_size,
            estimated_history_lines: p.estimated_history_lines,
            conflict: p.conflict.clone(),
            estimated_jsonl_files: p.estimated_jsonl_files,
            would_rewrite_claude_json: p.would_rewrite_claude_json,
            would_move_memory_dir: p.would_move_memory_dir,
            would_rewrite_project_settings: p.would_rewrite_project_settings,
        }
    }
}

/// Per-status counts for the pending-journals banner. Pending +
/// stale are the two actionable classes; abandoned is filtered out.
/// `running` exists so the banner can suppress itself when the
/// op is already visible in the RunningOpStrip.
#[derive(Serialize)]
pub struct PendingJournalsSummaryDto {
    pub pending: usize,
    pub stale: usize,
    pub running: usize,
}

/// Inbound args from the webview for a dry-run move. Mirrors the
/// subset of `claudepot_core::project_types::MoveArgs` the UI controls;
/// config/snapshot paths are filled server-side from `claude_config_dir`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveArgsDto {
    pub old_path: String,
    pub new_path: String,
    #[serde(default)]
    pub no_move: bool,
    #[serde(default)]
    pub merge: bool,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub ignore_pending_journals: bool,
    /// Monotonically increasing token for dry-run cancellation. When
    /// a newer token arrives during rapid typing, older in-flight
    /// calls bail out instead of returning stale plans. Present only
    /// on `project_move_dry_run`; other callers leave it None.
    #[serde(default)]
    pub cancel_token: Option<u64>,
}
