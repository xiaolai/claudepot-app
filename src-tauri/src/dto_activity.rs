//! Live activity DTOs — snapshot + per-session deltas + trends.

use serde::Serialize;

/// Front-end view of one live Claude Code session. Mirrors
/// `claudepot_core::session_live::types::LiveSessionSummary` but with
/// chrono types pre-serialized to milliseconds-since-epoch so the
/// webview doesn't need to handle `DateTime<Utc>`.
#[derive(Serialize, Clone)]
pub struct LiveSessionSummaryDto {
    pub session_id: String,
    pub pid: u32,
    pub cwd: String,
    pub transcript_path: Option<String>,
    /// One of `busy | idle | waiting` — the canonical CC vocabulary.
    pub status: String,
    pub current_action: Option<String>,
    pub model: Option<String>,
    /// Only populated when `status == "waiting"`.
    pub waiting_for: Option<String>,
    pub errored: bool,
    pub stuck: bool,
    pub idle_ms: i64,
    pub seq: u64,
}

impl From<claudepot_core::session_live::types::LiveSessionSummary>
    for LiveSessionSummaryDto
{
    fn from(s: claudepot_core::session_live::types::LiveSessionSummary) -> Self {
        use claudepot_core::session_live::types::Status;
        let status = match s.status {
            Status::Busy => "busy",
            Status::Idle => "idle",
            Status::Waiting => "waiting",
        }
        .to_string();
        Self {
            session_id: s.session_id,
            pid: s.pid,
            cwd: s.cwd,
            transcript_path: s.transcript_path,
            status,
            current_action: s.current_action,
            model: s.model,
            waiting_for: s.waiting_for,
            errored: s.errored,
            stuck: s.stuck,
            idle_ms: s.idle_ms,
            seq: s.seq,
        }
    }
}

/// Per-session delta carried over the `live::<sessionId>` event
/// channel. The webview discriminates on `kind`; each variant
/// carries only the fields relevant to that transition.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveDeltaKindDto {
    StatusChanged {
        status: String,
        waiting_for: Option<String>,
    },
    TaskSummaryChanged {
        summary: String,
    },
    ModelChanged {
        model: String,
    },
    OverlayChanged {
        errored: bool,
        stuck: bool,
    },
    Ended,
}

#[derive(Serialize, Clone)]
pub struct LiveDeltaDto {
    pub session_id: String,
    pub seq: u64,
    pub produced_at_ms: i64,
    #[serde(flatten)]
    pub kind: LiveDeltaKindDto,
    pub resync_required: bool,
}

/// Time-series snapshot for the Activity Trends view. A histogram of
/// distinct live-session counts per time bucket plus the total error
/// count inside the window. Buckets are returned in chronological
/// order; `bucket_width_ms = (to_ms - from_ms) / series.len()`.
#[derive(Serialize, Clone)]
pub struct ActivityTrendsDto {
    pub from_ms: i64,
    pub to_ms: i64,
    pub bucket_width_ms: i64,
    pub active_series: Vec<u64>,
    pub error_count: u64,
}

impl From<claudepot_core::session_live::types::LiveDelta> for LiveDeltaDto {
    fn from(d: claudepot_core::session_live::types::LiveDelta) -> Self {
        use claudepot_core::session_live::types::{LiveDeltaKind, Status};
        let status_str = |s: Status| -> String {
            match s {
                Status::Busy => "busy",
                Status::Idle => "idle",
                Status::Waiting => "waiting",
            }
            .to_string()
        };
        let kind = match d.kind {
            LiveDeltaKind::StatusChanged {
                status,
                waiting_for,
            } => LiveDeltaKindDto::StatusChanged {
                status: status_str(status),
                waiting_for,
            },
            LiveDeltaKind::TaskSummaryChanged { summary } => {
                LiveDeltaKindDto::TaskSummaryChanged { summary }
            }
            LiveDeltaKind::ModelChanged { model } => {
                LiveDeltaKindDto::ModelChanged { model }
            }
            LiveDeltaKind::OverlayChanged { errored, stuck } => {
                LiveDeltaKindDto::OverlayChanged { errored, stuck }
            }
            LiveDeltaKind::Ended => LiveDeltaKindDto::Ended,
        };
        Self {
            session_id: d.session_id,
            seq: d.seq,
            produced_at_ms: d.produced_at_ms,
            kind,
            resync_required: d.resync_required,
        }
    }
}
