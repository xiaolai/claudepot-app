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

impl From<&claudepot_core::project::CleanPreview> for CleanPreviewDto {
    fn from(p: &claudepot_core::project::CleanPreview) -> Self {
        Self {
            orphans: p.orphans.iter().map(ProjectInfoDto::from).collect(),
            orphans_found: p.orphans_found,
            unreachable_skipped: p.unreachable_skipped,
            total_bytes: p.total_bytes,
            protected_count: p.protected_count,
        }
    }
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

// ---------------------------------------------------------------------------
// project remove — preview + result DTOs
// ---------------------------------------------------------------------------

/// Read-only preview the GUI's RemoveProjectModal renders honestly.
/// Field-for-field copy of `claudepot_core::project_remove::RemovePreview`
/// with `last_modified_ms` instead of `SystemTime` for serde.
#[derive(Serialize)]
pub struct RemoveProjectPreviewDto {
    pub slug: String,
    pub original_path: Option<String>,
    pub bytes: u64,
    pub session_count: usize,
    pub last_modified_ms: Option<i64>,
    pub has_live_session: bool,
    pub claude_json_entry_present: bool,
    pub history_lines_count: usize,
}

impl From<&claudepot_core::project_remove::RemovePreview> for RemoveProjectPreviewDto {
    fn from(p: &claudepot_core::project_remove::RemovePreview) -> Self {
        Self {
            slug: p.slug.clone(),
            original_path: p.original_path.clone(),
            bytes: p.bytes,
            session_count: p.session_count,
            last_modified_ms: system_time_to_ms(p.last_modified),
            has_live_session: p.has_live_session,
            claude_json_entry_present: p.claude_json_entry_present,
            history_lines_count: p.history_lines_count,
        }
    }
}

/// Cheap subset — what the GUI modal renders on first paint. No live-
/// session probe, no large-file reads. Returns in <50 ms even when
/// sibling state is multi-MB.
#[derive(Serialize)]
pub struct RemoveProjectPreviewBasicDto {
    pub slug: String,
    pub original_path: Option<String>,
    pub bytes: u64,
    pub session_count: usize,
    pub last_modified_ms: Option<i64>,
}

impl From<&claudepot_core::project_remove::RemovePreviewBasic> for RemoveProjectPreviewBasicDto {
    fn from(p: &claudepot_core::project_remove::RemovePreviewBasic) -> Self {
        Self {
            slug: p.slug.clone(),
            original_path: p.original_path.clone(),
            bytes: p.bytes,
            session_count: p.session_count,
            last_modified_ms: system_time_to_ms(p.last_modified),
        }
    }
}

/// Slow subset — fields that gate the Remove button (`has_live_session`)
/// and annotate the disclosure (sibling-state counts). Comes in via a
/// follow-up call so the modal can render without waiting on it.
#[derive(Serialize)]
pub struct RemoveProjectPreviewExtrasDto {
    pub has_live_session: bool,
    pub claude_json_entry_present: bool,
    pub history_lines_count: usize,
}

impl From<&claudepot_core::project_remove::RemovePreviewExtras> for RemoveProjectPreviewExtrasDto {
    fn from(p: &claudepot_core::project_remove::RemovePreviewExtras) -> Self {
        Self {
            has_live_session: p.has_live_session,
            claude_json_entry_present: p.claude_json_entry_present,
            history_lines_count: p.history_lines_count,
        }
    }
}

/// Outcome of a successful `project_remove_execute`. Carries the trash
/// id so the GUI can offer a one-click Undo right after the operation
/// (in addition to the persistent Trash drawer).
#[derive(Serialize)]
pub struct RemoveProjectResultDto {
    pub slug: String,
    pub original_path: Option<String>,
    pub bytes: u64,
    pub session_count: usize,
    pub trash_id: String,
    pub claude_json_entry_removed: bool,
    pub history_lines_removed: usize,
}

impl From<&claudepot_core::project_remove::RemoveResult> for RemoveProjectResultDto {
    fn from(r: &claudepot_core::project_remove::RemoveResult) -> Self {
        Self {
            slug: r.slug.clone(),
            original_path: r.original_path.clone(),
            bytes: r.bytes,
            session_count: r.session_count,
            trash_id: r.trash_id.clone(),
            claude_json_entry_removed: r.claude_json_entry_removed,
            history_lines_removed: r.history_lines_removed,
        }
    }
}

/// One row in the project Trash drawer / `project trash list` output.
#[derive(Serialize)]
pub struct ProjectTrashEntryDto {
    pub id: String,
    pub slug: String,
    pub original_path: Option<String>,
    pub bytes: u64,
    pub session_count: usize,
    pub ts_ms: i64,
    pub has_claude_json_entry: bool,
    pub history_lines_count: usize,
}

impl From<&claudepot_core::project_trash::ProjectTrashEntry> for ProjectTrashEntryDto {
    fn from(e: &claudepot_core::project_trash::ProjectTrashEntry) -> Self {
        Self {
            id: e.id.clone(),
            slug: e.slug.clone(),
            original_path: e.original_path.clone(),
            bytes: e.bytes,
            session_count: e.session_count,
            ts_ms: e.ts_ms,
            has_claude_json_entry: e.claude_json_entry.is_some(),
            history_lines_count: e.history_lines.len(),
        }
    }
}

#[derive(Serialize)]
pub struct ProjectTrashListingDto {
    pub entries: Vec<ProjectTrashEntryDto>,
    pub total_bytes: u64,
}

impl From<&claudepot_core::project_trash::ProjectTrashListing> for ProjectTrashListingDto {
    fn from(l: &claudepot_core::project_trash::ProjectTrashListing) -> Self {
        Self {
            entries: l.entries.iter().map(ProjectTrashEntryDto::from).collect(),
            total_bytes: l.total_bytes,
        }
    }
}

#[derive(Serialize)]
pub struct ProjectRestoreReportDto {
    pub restored_dir: String,
    pub claude_json_restored: bool,
    pub history_lines_restored: usize,
}

impl From<&claudepot_core::project_trash::ProjectRestoreReport> for ProjectRestoreReportDto {
    fn from(r: &claudepot_core::project_trash::ProjectRestoreReport) -> Self {
        Self {
            restored_dir: r.restored_dir.to_string_lossy().to_string(),
            claude_json_restored: r.claude_json_restored,
            history_lines_restored: r.history_lines_restored,
        }
    }
}

#[cfg(test)]
mod clean_preview_dto_tests {
    use super::*;
    use claudepot_core::project::CleanPreview;

    /// `CleanPreviewDto::from(&CleanPreview)` must be a pure field copy.
    /// If a future field is added to `CleanPreview` and this conversion
    /// silently drops it, the JSON the GUI sees will lose data without
    /// any compile-time signal. Locking the field-by-field copy down
    /// here makes that drift load-bearing for tests.
    #[test]
    fn test_clean_preview_dto_is_pure_field_copy() {
        let preview = CleanPreview {
            orphans: Vec::new(),
            orphans_found: 7,
            unreachable_skipped: 3,
            total_bytes: 1234,
            protected_count: 2,
        };
        let dto = CleanPreviewDto::from(&preview);
        assert_eq!(dto.orphans_found, preview.orphans_found);
        assert_eq!(dto.unreachable_skipped, preview.unreachable_skipped);
        assert_eq!(dto.total_bytes, preview.total_bytes);
        assert_eq!(dto.protected_count, preview.protected_count);
        assert_eq!(dto.orphans.len(), preview.orphans.len());
    }
}
