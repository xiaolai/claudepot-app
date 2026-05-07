//! DTOs for the Projects → Memory pane and Settings → Auto-memory toggle.
//!
//! Mirrors the core types but flattens for JSON-friendly shapes. We
//! avoid leaking `PathBuf` into JSON (always stringify) and tag
//! enum-shaped fields with the same kebab/snake-case strings the
//! frontend already uses elsewhere.

use claudepot_core::memory_log::{ChangeType, DiffOmitReason, MemoryChange, MemoryFileStats};
use claudepot_core::memory_view::{
    EnumerateResult, MemoryFileRole, MemoryFileSummary, ProjectMemoryAnchor,
};
use claudepot_core::settings_writer::{AutoMemoryDecisionSource, AutoMemoryState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectMemoryAnchorDto {
    pub project_root: String,
    pub auto_memory_anchor: String,
    pub slug: String,
    pub auto_memory_dir: String,
}

impl From<ProjectMemoryAnchor> for ProjectMemoryAnchorDto {
    fn from(a: ProjectMemoryAnchor) -> Self {
        Self {
            project_root: a.project_root.to_string_lossy().into_owned(),
            auto_memory_anchor: a.auto_memory_anchor.to_string_lossy().into_owned(),
            slug: a.slug,
            auto_memory_dir: a.auto_memory_dir.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryFileSummaryDto {
    pub abs_path: String,
    pub role: MemoryFileRole,
    pub scope_label: String,
    pub size_bytes: u64,
    pub mtime_unix_ns: i64,
    pub line_count: usize,
    pub lines_past_cutoff: Option<usize>,
    /// Most recent `detected_at_ns` from `memory_changes` for this
    /// path, or `None` if we have no change-log history.
    pub last_change_unix_ns: Option<i64>,
    /// Number of change-log rows in the last 30 days.
    pub change_count_30d: u32,
}

impl MemoryFileSummaryDto {
    pub fn from_summary(s: MemoryFileSummary, stat: Option<&MemoryFileStats>) -> Self {
        let scope_label = s.role.scope_label().to_string();
        Self {
            abs_path: s.abs_path.to_string_lossy().into_owned(),
            role: s.role,
            scope_label,
            size_bytes: s.size_bytes,
            mtime_unix_ns: s.mtime_unix_ns,
            line_count: s.line_count,
            lines_past_cutoff: s.lines_past_cutoff,
            last_change_unix_ns: stat.and_then(|x| x.last_change_unix_ns),
            change_count_30d: stat.map(|x| x.change_count_30d).unwrap_or(0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryEnumerateDto {
    pub anchor: ProjectMemoryAnchorDto,
    pub files: Vec<MemoryFileSummaryDto>,
}

impl MemoryEnumerateDto {
    pub fn from_result(
        r: EnumerateResult,
        stats: &std::collections::HashMap<std::path::PathBuf, MemoryFileStats>,
    ) -> Self {
        let files = r
            .files
            .into_iter()
            .map(|s| {
                let key = s.abs_path.clone();
                MemoryFileSummaryDto::from_summary(s, stats.get(&key))
            })
            .collect();
        Self {
            anchor: r.anchor.into(),
            files,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryChangeDto {
    pub id: i64,
    pub project_slug: Option<String>,
    pub abs_path: String,
    pub role: MemoryFileRole,
    pub change_type: ChangeType,
    pub detected_at_ns: i64,
    pub mtime_ns: i64,
    pub size_before: Option<i64>,
    pub size_after: Option<i64>,
    pub hash_before: Option<String>,
    pub hash_after: Option<String>,
    pub diff_text: Option<String>,
    pub diff_omitted: bool,
    pub diff_omit_reason: Option<DiffOmitReason>,
}

impl From<MemoryChange> for MemoryChangeDto {
    fn from(c: MemoryChange) -> Self {
        Self {
            id: c.id,
            project_slug: c.project_slug,
            abs_path: c.abs_path.to_string_lossy().into_owned(),
            role: c.role,
            change_type: c.change_type,
            detected_at_ns: c.detected_at_ns,
            mtime_ns: c.mtime_ns,
            size_before: c.size_before,
            size_after: c.size_after,
            hash_before: c.hash_before,
            hash_after: c.hash_after,
            diff_text: c.diff_text,
            diff_omitted: c.diff_omitted,
            diff_omit_reason: c.diff_omit_reason,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutoMemoryStateDto {
    pub project_root: String,
    pub effective: bool,
    pub decided_by: AutoMemoryDecisionSource,
    pub decided_label: String,
    pub user_writable: bool,
    pub user_settings_value: Option<bool>,
    pub project_settings_value: Option<bool>,
    pub local_project_settings_value: Option<bool>,
    pub env_disable_set: bool,
    pub env_simple_set: bool,
    /// `Some(true)` when the project's `.gitignore` covers
    /// `settings.local.json`. `None` when the file isn't readable
    /// (no project root, perm denied) — UI hides the warning.
    pub local_settings_gitignored: Option<bool>,
}

impl AutoMemoryStateDto {
    pub fn from_state(state: AutoMemoryState, project_root: &std::path::Path) -> Self {
        let label = match state.decided_by {
            AutoMemoryDecisionSource::EnvDisable => {
                "env: CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_string()
            }
            AutoMemoryDecisionSource::EnvSimple => "env: CLAUDE_CODE_SIMPLE".to_string(),
            AutoMemoryDecisionSource::LocalProjectSettings => {
                ".claude/settings.local.json".to_string()
            }
            AutoMemoryDecisionSource::ProjectSettings => ".claude/settings.json".to_string(),
            AutoMemoryDecisionSource::UserSettings => "~/.claude/settings.json".to_string(),
            AutoMemoryDecisionSource::Default => "default".to_string(),
        };
        let gitignored =
            claudepot_core::settings_writer::local_settings_is_gitignored(project_root).ok();
        Self {
            project_root: project_root.to_string_lossy().into_owned(),
            effective: state.effective,
            decided_by: state.decided_by,
            decided_label: label,
            user_writable: state.user_writable,
            user_settings_value: state.user_settings_value,
            project_settings_value: state.project_settings_value,
            local_project_settings_value: state.local_project_settings_value,
            env_disable_set: state.env_disable_set,
            env_simple_set: state.env_simple_set,
            local_settings_gitignored: gitignored,
        }
    }
}
