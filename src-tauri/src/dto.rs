//! Frontend DTOs — what crosses the Tauri command boundary.
//!
//! We deliberately do NOT expose credential blobs, access tokens, or refresh
//! tokens to the webview. Only non-sensitive metadata leaves Rust.
//!
//! The DTO surface is sharded across topic-named sibling modules
//! (`dto_account`, `dto_usage`, `dto_desktop`, `dto_session`, …) so
//! each file stays under the loc-guardian limit. This file is the
//! facade — every callable still imports `crate::dto::Foo`.

use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Shared helpers used by the split-out DTO modules.
// ---------------------------------------------------------------------------

/// Millisecond epoch helper. `SystemTime` isn't directly serde-friendly
/// for the JS heap; the webview wants a number it can pass to
/// `new Date()`. Visible to sibling `dto_*` modules that need to
/// translate `Option<SystemTime>` fields at the boundary.
pub(crate) fn system_time_to_ms(t: Option<SystemTime>) -> Option<i64> {
    t.and_then(|st| st.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

// ---------------------------------------------------------------------------
// Topic-split DTO re-exports.
//
// The webview-facing DTOs are grouped by section into sibling files
// (`dto_session.rs`, `dto_project.rs`, etc.) to keep each file under
// the LOC ceiling. Consumers `use crate::dto::Foo` as before — the
// re-exports below preserve that surface so `commands.rs` doesn't
// need to track the split. A handful of them are referenced only via
// their sibling module (`crate::dto_session::Foo`) today; they're
// retained here as the canonical facade so new callers have one
// obvious place to look.
// ---------------------------------------------------------------------------

#[allow(unused_imports)]
pub use crate::{
    dto_account::{
        AccountSummary, AccountSummaryBasic, AppStatus, CcIdentity, ReconcileReportDto,
        RegisterOutcome, RemoveOutcome,
    },
    dto_activity::{
        ActivityTrendsDto, LiveDeltaDto, LiveDeltaKindDto, LiveSessionSummaryDto,
    },
    dto_activity_cards::{
        ActivityCardDto, CardNavigateDto, CardsCountDto, CardsRecentQueryDto,
        CardsReindexFailureDto, CardsReindexResultDto, HelpRefDto, SourceRefDto,
    },
    dto_desktop::{
        DesktopAdoptOutcome, DesktopClearOutcome, DesktopIdentity, DesktopProbeMethod,
        DesktopSyncOutcome,
    },
    dto_keys::{ApiKeySummaryDto, OauthTokenSummaryDto},
    dto_project::{
        CleanPreviewDto, DryRunPlanDto, MoveArgsDto, PendingJournalsSummaryDto,
        ProjectDetailDto, ProjectInfoDto, SessionInfoDto,
    },
    dto_project_repair::{JournalEntryDto, JournalFlagsDto},
    dto_session::{
        ProtectedPathDto, SessionDetailDto, SessionEventDto, SessionRowDto, TokenUsageDto,
    },
    dto_session_debug::{
        ChunkHeaderDto, ChunkMetricsDto, ContextCategoryDto, ContextInjectionDto,
        ContextPhaseDto, ContextStatsDto, LinkedToolDto, RepositoryGroupDto, SearchHitDto,
        SessionChunkDto, TokensByCategoryDto,
    },
    dto_session_move::{
        AdoptFailureDto, AdoptReportDto, DiscardReportDto, MoveSessionReportDto,
        OrphanedProjectDto,
    },
    dto_session_prune::{
        BulkSlimEntryDto, BulkSlimPlanDto, PruneEntryDto, PruneFilterDto, PrunePlanDto,
        SlimOptsDto, SlimPlanDto, TrashEntryDto, TrashListingDto,
    },
    dto_usage::{AccountUsageDto, ExtraUsageDto, UsageEntryDto, UsageWindowDto},
};
