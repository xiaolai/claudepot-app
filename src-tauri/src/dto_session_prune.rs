//! Prune / slim / trash DTOs for the Sessions maintenance surface.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct PruneFilterDto {
    pub older_than_secs: Option<u64>,
    pub larger_than_bytes: Option<u64>,
    pub project: Vec<String>,
    pub has_error: Option<bool>,
    pub is_sidechain: Option<bool>,
}

#[derive(Serialize)]
pub struct PruneEntryDto {
    pub session_id: String,
    pub file_path: String,
    pub project_path: String,
    pub size_bytes: u64,
    pub last_ts_ms: Option<i64>,
    pub has_error: bool,
    pub is_sidechain: bool,
}

#[derive(Serialize)]
pub struct PrunePlanDto {
    pub entries: Vec<PruneEntryDto>,
    pub total_bytes: u64,
}

// PruneReportDto was removed — `session_prune_start` discards the
// structured report and emits a string error summary via
// emit_terminal instead. Reintroduce when the GUI needs per-path
// success/failure structures.

impl From<&claudepot_core::session_prune::PruneEntry> for PruneEntryDto {
    fn from(e: &claudepot_core::session_prune::PruneEntry) -> Self {
        Self {
            session_id: e.session_id.clone(),
            file_path: e.file_path.to_string_lossy().to_string(),
            project_path: e.project_path.clone(),
            size_bytes: e.size_bytes,
            last_ts_ms: e.last_ts_ms,
            has_error: e.has_error,
            is_sidechain: e.is_sidechain,
        }
    }
}

impl From<&claudepot_core::session_prune::PrunePlan> for PrunePlanDto {
    fn from(p: &claudepot_core::session_prune::PrunePlan) -> Self {
        Self {
            entries: p.entries.iter().map(Into::into).collect(),
            total_bytes: p.total_bytes,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SlimOptsDto {
    pub drop_tool_results_over_bytes: u64,
    #[serde(default)]
    pub exclude_tools: Vec<String>,
    #[serde(default)]
    pub strip_images: bool,
    #[serde(default)]
    pub strip_documents: bool,
}

#[derive(Serialize)]
pub struct SlimPlanDto {
    pub original_bytes: u64,
    pub projected_bytes: u64,
    pub redact_count: u32,
    pub image_redact_count: u32,
    pub document_redact_count: u32,
    pub tools_affected: Vec<String>,
    pub bytes_saved: u64,
}

// SlimReportDto was removed — `session_slim_start` discards the
// structured report and emits a string error summary via
// emit_terminal. Reintroduce when the GUI surfaces per-op byte
// deltas after completion.

impl From<&claudepot_core::session_slim::SlimPlan> for SlimPlanDto {
    fn from(p: &claudepot_core::session_slim::SlimPlan) -> Self {
        Self {
            original_bytes: p.original_bytes,
            projected_bytes: p.projected_bytes,
            redact_count: p.redact_count,
            image_redact_count: p.image_redact_count,
            document_redact_count: p.document_redact_count,
            tools_affected: p.tools_affected.clone(),
            bytes_saved: p.bytes_saved(),
        }
    }
}

#[derive(Serialize)]
pub struct BulkSlimEntryDto {
    pub session_id: String,
    pub file_path: String,
    pub project_path: String,
    pub plan: SlimPlanDto,
}

#[derive(Serialize)]
pub struct BulkSlimPlanDto {
    pub entries: Vec<BulkSlimEntryDto>,
    /// Matched rows whose `plan_slim()` call errored — surfaced so
    /// the user sees unreadable sessions in the preview.
    pub failed_to_plan: Vec<(String, String)>,
    pub total_bytes_saved: u64,
    pub total_image_redacts: u32,
    pub total_document_redacts: u32,
    pub total_tool_result_redacts: u32,
}

impl From<&claudepot_core::session_slim::BulkSlimEntry> for BulkSlimEntryDto {
    fn from(e: &claudepot_core::session_slim::BulkSlimEntry) -> Self {
        Self {
            session_id: e.session_id.clone(),
            file_path: e.file_path.to_string_lossy().to_string(),
            project_path: e.project_path.clone(),
            plan: (&e.plan).into(),
        }
    }
}

impl From<&claudepot_core::session_slim::BulkSlimPlan> for BulkSlimPlanDto {
    fn from(p: &claudepot_core::session_slim::BulkSlimPlan) -> Self {
        Self {
            entries: p.entries.iter().map(Into::into).collect(),
            failed_to_plan: p
                .failed_to_plan
                .iter()
                .map(|(p, e)| (p.to_string_lossy().to_string(), e.clone()))
                .collect(),
            total_bytes_saved: p.total_bytes_saved,
            total_image_redacts: p.total_image_redacts,
            total_document_redacts: p.total_document_redacts,
            total_tool_result_redacts: p.total_tool_result_redacts,
        }
    }
}
// BulkSlimReportDto was removed — the bulk-start worker emits a
// string error summary via emit_terminal, so the structured report
// has no consumer yet. Reintroduce when the GUI needs per-file
// success/failure structures.

#[derive(Serialize)]
pub struct TrashEntryDto {
    pub id: String,
    pub kind: String,
    pub orig_path: String,
    pub size: u64,
    pub ts_ms: i64,
    pub cwd: Option<String>,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct TrashListingDto {
    pub entries: Vec<TrashEntryDto>,
    pub total_bytes: u64,
}

impl From<&claudepot_core::trash::TrashEntry> for TrashEntryDto {
    fn from(e: &claudepot_core::trash::TrashEntry) -> Self {
        Self {
            id: e.id.clone(),
            kind: match e.kind {
                claudepot_core::trash::TrashKind::Prune => "prune",
                claudepot_core::trash::TrashKind::Slim => "slim",
            }
            .to_string(),
            orig_path: e.orig_path.to_string_lossy().to_string(),
            size: e.size,
            ts_ms: e.ts_ms,
            cwd: e.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
            reason: e.reason.clone(),
        }
    }
}

impl From<&claudepot_core::trash::TrashListing> for TrashListingDto {
    fn from(l: &claudepot_core::trash::TrashListing) -> Self {
        Self {
            entries: l.entries.iter().map(Into::into).collect(),
            total_bytes: l.total_bytes,
        }
    }
}
