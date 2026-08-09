//! Session move DTOs — orphan adoption / discard / single-session move.

use serde::Serialize;

/// What a free-form move target actually is on disk, resolved once so
/// the Move-session modal can say "this folder will be created" rather
/// than guessing — and so the path it finally submits is the expanded
/// one, not the `~/…` the user typed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetProbeDto {
    /// The input with a leading `~` / `~/` expanded; byte-identical to
    /// the input for every other shape (including `~user`, which has no
    /// portable expansion — see `path_utils::expand_tilde`).
    pub resolved_path: String,
    /// `Path::is_absolute` on `resolved_path`. A relative cwd would be
    /// resolved against whatever directory CC happens to start in, so
    /// the caller must refuse one.
    pub is_absolute: bool,
    pub exists: bool,
    /// Follows symlinks, matching what CC does when it `cd`s there.
    /// `exists && !is_dir` is the one combination a move must refuse.
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanedProjectDto {
    pub slug: String,
    pub cwd_from_transcript: Option<String>,
    pub session_count: usize,
    pub total_size_bytes: u64,
    pub suggested_adoption_target: Option<String>,
}

impl From<&claudepot_core::session_move::OrphanedProject> for OrphanedProjectDto {
    fn from(o: &claudepot_core::session_move::OrphanedProject) -> Self {
        Self {
            slug: o.slug.clone(),
            cwd_from_transcript: o
                .cwd_from_transcript
                .as_ref()
                .map(|p| p.display().to_string()),
            session_count: o.session_count,
            total_size_bytes: o.total_size_bytes,
            suggested_adoption_target: o
                .suggested_adoption_target
                .as_ref()
                .map(|p| p.display().to_string()),
        }
    }
}

// The move-report wire struct lives in `crate::ops` as
// `MoveSessionReportSummary` (it rides on `RunningOpInfo` for the
// op-progress pipeline) — same single-home pattern as
// `ops::CleanResultSummary` / `dto_project`. Re-exported here under
// the DTO name so the legacy `session_move` IPC keeps its import
// path and both surfaces serialize the identical camelCase shape.
pub use crate::ops::MoveSessionReportSummary as MoveSessionReportDto;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptReportDto {
    pub sessions_attempted: usize,
    pub sessions_moved: usize,
    pub sessions_failed: Vec<AdoptFailureDto>,
    pub source_dir_removed: bool,
    pub per_session: Vec<MoveSessionReportDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptFailureDto {
    pub session_id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardReportDto {
    pub sessions_discarded: usize,
    pub total_size_bytes: u64,
    pub dir_removed: bool,
}

impl From<&claudepot_core::session_move::DiscardReport> for DiscardReportDto {
    fn from(r: &claudepot_core::session_move::DiscardReport) -> Self {
        Self {
            sessions_discarded: r.sessions_discarded,
            total_size_bytes: r.total_size_bytes,
            dir_removed: r.dir_removed,
        }
    }
}

impl From<&claudepot_core::session_move::AdoptReport> for AdoptReportDto {
    fn from(r: &claudepot_core::session_move::AdoptReport) -> Self {
        Self {
            sessions_attempted: r.sessions_attempted,
            sessions_moved: r.sessions_moved,
            sessions_failed: r
                .sessions_failed
                .iter()
                .map(|(sid, msg)| AdoptFailureDto {
                    session_id: sid.to_string(),
                    error: msg.clone(),
                })
                .collect(),
            source_dir_removed: r.source_dir_removed,
            per_session: r
                .per_session
                .iter()
                .map(MoveSessionReportDto::from)
                .collect(),
        }
    }
}
