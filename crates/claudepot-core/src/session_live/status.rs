//! Status state machine for a single Claude Code session.
//!
//! Consumes a stream of `claudepot_core::session::SessionEvent` and
//! produces status snapshots. When the PID registry carries an
//! authoritative `status` field (feature-gated by `BG_SESSIONS`), we
//! prefer CC's own reading over any transcript-derived inference;
//! when the field is absent (current default on this machine), the
//! state machine derives status from the event sequence.
//!
//! ### Why not "last event wins"
//!
//! The Codex architectural review called out that raw "last line
//! wins" is unreliable because a single assistant turn emits multiple
//! JSONL fragments — `thinking`, `text`, `tool_use` — with mixed
//! `stop_reason`. The state machine here instead tracks the *open*
//! tool-uses (by id) and the shape of the most recent assistant
//! fragment, so a turn in progress is `Busy` regardless of which
//! fragment landed last.
//!
//! ### Vocabulary
//!
//! Base status is one of `busy`, `idle`, `waiting` — the three
//! values CC publishes at `concurrentSessions.ts:19`. Overlays
//! (`errored`, `stuck`) are separate booleans on `StatusSnapshot`
//! rather than extra status variants, so the base status field
//! stays aligned with CC's terminology.

use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, VecDeque};

use crate::session::SessionEvent;
use crate::session_live::redact::redact_secrets;
use crate::session_live::status_helpers::{event_ts, humanize_tool_input, truncate};
use crate::session_live::status_types::{LastAssistantShape, OpenTool};
use crate::session_live::types::Status;

// Public re-exports so external callers continue to reach these
// names via `session_live::status::*` after the loc-guardian split.
pub use crate::session_live::status_types::{
    StatusSnapshot, ERROR_WINDOW, ERROR_WINDOW_COUNT, STUCK_THRESHOLD,
};

/// Mutable state machine. Feed events via `ingest`; call `snapshot`
/// any time to read the current derived state. The machine carries
/// a small bounded history (recent error timestamps) but is otherwise
/// O(unmatched tool_uses) in memory.
#[derive(Debug, Clone)]
pub struct StatusMachine {
    /// Open tool calls by id. BTree so iteration order is deterministic
    /// for tests; the oldest-by-start-time is re-derived in
    /// `current_action` (BTree key is id, not time).
    unmatched: BTreeMap<String, OpenTool>,
    /// Sliding window of recent error timestamps. Trimmed on every
    /// ingest so it never grows unbounded even if a pathological
    /// session emits errors faster than the window slides.
    recent_errors: VecDeque<DateTime<Utc>>,
    /// Most recent model id from an assistant fragment.
    model: Option<String>,
    /// Most recent `task-summary` entry's text. Preferred over the
    /// tool head-line in `current_action` because CC wrote it
    /// explicitly to describe what the session is doing.
    last_task_summary: Option<String>,
    /// What the last-observed assistant fragment looked like.
    last_assistant: LastAssistantShape,
    /// Whether we've seen any user-originated event since the last
    /// turn close. Distinguishes the fresh-session Idle from the
    /// "user spoke, model silent" Busy.
    pending_reply: bool,
    /// Authoritative status from the PID file, if present. Overrides
    /// transcript derivation for the base status value.
    pid_status: Option<Status>,
    pid_waiting_for: Option<String>,
    last_activity_ts: Option<DateTime<Utc>>,
    /// Injectable wall-clock for deterministic tests.
    now: fn() -> DateTime<Utc>,
}

impl Default for StatusMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusMachine {
    pub fn new() -> Self {
        Self::with_now(Utc::now)
    }

    pub fn with_now(now: fn() -> DateTime<Utc>) -> Self {
        Self {
            unmatched: BTreeMap::new(),
            recent_errors: VecDeque::new(),
            model: None,
            last_task_summary: None,
            last_assistant: LastAssistantShape::None,
            pending_reply: false,
            pid_status: None,
            pid_waiting_for: None,
            last_activity_ts: None,
            now,
        }
    }

    /// Override the derived base status with a value from the PID
    /// registry. Pass `None` to clear. When set, `snapshot()` returns
    /// this status verbatim; overlays still come from the transcript.
    pub fn set_pid_status(&mut self, status: Option<Status>, waiting_for: Option<String>) {
        self.pid_status = status;
        self.pid_waiting_for = waiting_for;
    }

    /// Ingest one parsed event. Order matters: feed events in the
    /// order they appear in the transcript.
    pub fn ingest(&mut self, event: &SessionEvent) {
        if let Some(ts) = event_ts(event) {
            self.last_activity_ts = Some(ts);
        }

        match event {
            SessionEvent::UserText { .. } => {
                // A user message starts a new turn; any prior assistant
                // turn-close is irrelevant now.
                self.last_assistant = LastAssistantShape::None;
                self.pending_reply = true;
            }
            SessionEvent::UserToolResult {
                tool_use_id,
                is_error,
                ts,
                ..
            } => {
                // Only flip `pending_reply` if this result actually
                // closes a tool call we knew about. A result that
                // arrives with no matching open is a late straggler
                // (e.g., sub-agent bleed-through) and must not pin
                // the machine at Busy forever — this is the
                // "ghost busy" trap the reviews flagged.
                let matched = self.unmatched.remove(tool_use_id).is_some();
                if *is_error {
                    let at = *ts.as_ref().unwrap_or(&(self.now)());
                    self.recent_errors.push_back(at);
                    self.trim_error_window();
                }
                if matched {
                    self.pending_reply = true;
                }
            }
            SessionEvent::AssistantText {
                model, stop_reason, ..
            } => {
                if let Some(m) = model.clone() {
                    self.model = Some(m);
                }
                // CC writes `stop_reason: "tool_use"` when a turn
                // pauses for a tool call — NOT when the turn ends.
                // Only `end_turn` and `stop_sequence` mean the
                // assistant has handed control back to the user.
                let turn_truly_closed = matches!(
                    stop_reason.as_deref(),
                    Some("end_turn") | Some("stop_sequence")
                );
                self.last_assistant = if turn_truly_closed {
                    LastAssistantShape::TextClosed
                } else {
                    LastAssistantShape::TextStreaming
                };
                if turn_truly_closed {
                    self.pending_reply = false;
                }
            }
            SessionEvent::AssistantThinking { .. } => {
                self.last_assistant = LastAssistantShape::Thinking;
            }
            SessionEvent::AssistantToolUse {
                model,
                tool_name,
                tool_use_id,
                input_preview,
                ts,
                ..
            } => {
                if let Some(m) = model.clone() {
                    self.model = Some(m);
                }
                self.last_assistant = LastAssistantShape::ToolUse;
                self.unmatched.insert(
                    tool_use_id.clone(),
                    OpenTool {
                        tool_name: tool_name.clone(),
                        input_preview: input_preview.clone(),
                        started_at: *ts,
                    },
                );
                self.pending_reply = true;
            }
            SessionEvent::System { subtype, .. } => {
                if subtype.as_deref() == Some("turn_duration") {
                    // Turn close — no pending reply, no open text.
                    self.pending_reply = false;
                    // Don't clobber `last_assistant` — it might be a
                    // `ToolUse` whose result landed on the next turn,
                    // in which case the unmatched map already tracks
                    // the open call and status stays `busy`.
                }
            }
            SessionEvent::TaskSummary { summary, .. } => {
                // CC's own "what am I doing now" snapshot. Latched
                // into `last_task_summary` so `current_action`
                // prefers it over the tool head-line.
                self.last_task_summary = Some(summary.clone());
            }
            SessionEvent::Summary { .. }
            | SessionEvent::Attachment { .. }
            | SessionEvent::FileHistorySnapshot { .. }
            | SessionEvent::Other { .. }
            | SessionEvent::Malformed { .. } => {
                // Noise for status purposes.
            }
        }
    }

    /// Read the current derived state.
    pub fn snapshot(&self) -> StatusSnapshot {
        let status = if let Some(s) = self.pid_status {
            s
        } else {
            self.derived_status()
        };

        let waiting_for = if status == Status::Waiting {
            self.pid_waiting_for.clone()
        } else {
            None
        };

        StatusSnapshot {
            status,
            waiting_for,
            model: self.model.clone(),
            current_action: self.current_action(),
            task_summary: self.last_task_summary.clone(),
            errored: self.errored(),
            stuck: self.stuck(),
            last_activity_ts: self.last_activity_ts,
        }
    }

    /// Current base status derived from the event stream (PID-file
    /// override not applied — that's `snapshot`'s job).
    fn derived_status(&self) -> Status {
        if !self.unmatched.is_empty() {
            return Status::Busy;
        }
        match self.last_assistant {
            LastAssistantShape::TextStreaming | LastAssistantShape::Thinking => Status::Busy,
            LastAssistantShape::ToolUse => {
                // Tool use with no open call means the result landed
                // in the same ingestion batch; the model still has
                // the floor unless the turn was closed.
                if self.pending_reply {
                    Status::Busy
                } else {
                    Status::Idle
                }
            }
            LastAssistantShape::TextClosed | LastAssistantShape::None => {
                if self.pending_reply {
                    // User spoke; model hasn't produced anything yet.
                    Status::Busy
                } else {
                    Status::Idle
                }
            }
        }
    }

    fn current_action(&self) -> Option<String> {
        // Preference order:
        //   1. CC's own `task-summary` text (designed for `claude ps`)
        //   2. The oldest open tool call, formatted as "<tool>: <arg>"
        // Both pass through redact_secrets before leaving the core —
        // CC's natural-language summaries can quote Bash commands
        // that contain sk-ant-* tokens, and user-command args
        // frequently do too.
        if let Some(summary) = &self.last_task_summary {
            let trimmed = summary.trim();
            if !trimmed.is_empty() {
                return Some(redact_secrets(&truncate(trimmed, 80)));
            }
        }
        let mut openings: Vec<_> = self.unmatched.values().collect();
        openings.sort_by_key(|o| o.started_at);
        let first = openings.first()?;
        let arg = humanize_tool_input(&first.tool_name, &first.input_preview);
        let text = if arg.is_empty() {
            first.tool_name.clone()
        } else {
            format!("{}: {}", first.tool_name, arg)
        };
        Some(redact_secrets(&truncate(&text, 80)))
    }

    fn errored(&self) -> bool {
        let now = (self.now)();
        let cutoff = now - ERROR_WINDOW;
        self.recent_errors.iter().filter(|t| **t >= cutoff).count() >= ERROR_WINDOW_COUNT
    }

    /// Drop entries older than the error window. Called on every
    /// error ingest so `recent_errors` never exceeds the number of
    /// errors produced within the trailing `ERROR_WINDOW` — bounded
    /// even under pathological burst rates.
    fn trim_error_window(&mut self) {
        let now = (self.now)();
        let cutoff = now - ERROR_WINDOW;
        while let Some(front) = self.recent_errors.front() {
            if *front < cutoff {
                self.recent_errors.pop_front();
            } else {
                break;
            }
        }
    }

    fn stuck(&self) -> bool {
        let now = (self.now)();
        self.unmatched
            .values()
            .filter_map(|o| o.started_at)
            .any(|t| (now - t) > STUCK_THRESHOLD)
    }
}


#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
