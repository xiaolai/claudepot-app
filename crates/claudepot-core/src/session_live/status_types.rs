//! Public and internal types for the status state machine.
//!
//! Split out of `status.rs` per the loc-guardian extraction rule for
//! type definitions (>30 LOC). The state-machine *logic* stays in
//! `status.rs`; this file carries only data shapes and trivial
//! `Default` impls.

use chrono::{DateTime, Utc};

use crate::session_live::types::Status;

/// How long an unmatched `tool_use` can live before we overlay
/// `stuck`. Ten minutes is the plan default and deliberately generous:
/// a `Bash: pnpm build` on a cold cache can easily run five minutes.
pub const STUCK_THRESHOLD: chrono::Duration = chrono::Duration::minutes(10);

/// Trailing window for the `errored` overlay.
pub const ERROR_WINDOW: chrono::Duration = chrono::Duration::seconds(60);

/// Minimum number of `is_error=true` results inside `ERROR_WINDOW`
/// required to flip the `errored` overlay on.
pub const ERROR_WINDOW_COUNT: usize = 2;

/// Current derived status of one session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusSnapshot {
    pub status: Status,
    /// Short verb populated when `status == Waiting`. Sourced from
    /// the PID file's `waitingFor` string; the transcript-derivation
    /// path never populates this (it only sets `Waiting` if a PID
    /// record says so).
    pub waiting_for: Option<String>,
    /// Most recently observed model id from `assistant.message.model`.
    pub model: Option<String>,
    /// One-line description of the open tool call (if any), sourced
    /// from the oldest unmatched `tool_use`. Format: `"<tool>: <arg>"`.
    /// Capped at 80 chars for the peripheral surfaces.
    pub current_action: Option<String>,
    /// ≥ `ERROR_WINDOW_COUNT` `is_error=true` results in the trailing
    /// `ERROR_WINDOW`. Reset automatically when the window slides past.
    pub errored: bool,
    /// The oldest unmatched `tool_use` is older than `STUCK_THRESHOLD`.
    pub stuck: bool,
    /// Wall-clock timestamp of the most recent meaningful event.
    /// Used by the runtime to compute `idle_ms`.
    pub last_activity_ts: Option<DateTime<Utc>>,
}

impl Default for Status {
    fn default() -> Self {
        Status::Idle
    }
}

/// Shape of the last-observed assistant fragment. Used to distinguish
/// "text still streaming" from "turn complete" in the derived-status
/// path when the PID file doesn't supply an authoritative status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum LastAssistantShape {
    #[default]
    None,
    /// `assistantText` with a terminal `stop_reason` (`end_turn` or
    /// `stop_sequence`): turn complete.
    TextClosed,
    /// `assistantText` with a non-terminal stop_reason (e.g.
    /// `tool_use`) or none: still streaming.
    TextStreaming,
    /// `assistantThinking`: mid-turn reasoning block.
    Thinking,
    /// `assistantToolUse` was the last shape we saw.
    ToolUse,
}

/// State for one open tool call; kept in `StatusMachine.unmatched`.
#[derive(Debug, Clone)]
pub(super) struct OpenTool {
    pub tool_name: String,
    pub input_preview: String,
    pub started_at: Option<DateTime<Utc>>,
}
