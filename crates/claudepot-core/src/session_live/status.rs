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

use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

use crate::session::SessionEvent;
use crate::session_live::types::Status;

/// How long an unmatched `tool_use` can live before we overlay
/// `stuck`. Ten minutes is the plan default and deliberately generous:
/// a `Bash: pnpm build` on a cold cache can easily run five minutes.
pub const STUCK_THRESHOLD: Duration = Duration::minutes(10);

/// Trailing window for the `errored` overlay.
pub const ERROR_WINDOW: Duration = Duration::seconds(60);

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
    /// ≥ ERROR_WINDOW_COUNT `is_error=true` results in the trailing
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

/// Mutable state machine. Feed events via `ingest`; call `snapshot`
/// any time to read the current derived state. The machine carries
/// a small bounded history (recent error timestamps) but is otherwise
/// O(unmatched tool_uses) in memory.
#[derive(Debug, Clone)]
pub struct StatusMachine {
    /// Open tool calls by id. BTree so ordering is deterministic for
    /// tests and we can cheaply take the oldest for the stuck check.
    unmatched: BTreeMap<String, OpenTool>,
    /// Sliding window of recent error timestamps. Trimmed on every
    /// ingest; never grows unbounded.
    recent_errors: Vec<DateTime<Utc>>,
    /// Most recent model id from an assistant fragment.
    model: Option<String>,
    /// What the last-observed assistant fragment looked like. Used
    /// to distinguish "text still streaming" from "turn complete".
    last_assistant: LastAssistantShape,
    /// Whether we've seen any user-originated event (user text or
    /// tool result) since the last turn close. Distinguishes the
    /// fresh-session Idle from the "user spoke, model silent" Busy.
    pending_reply: bool,
    /// Authoritative status from the PID file, if present. Overrides
    /// transcript derivation for the base status value.
    pid_status: Option<Status>,
    /// Authoritative waiting-for verb from the PID file.
    pid_waiting_for: Option<String>,
    /// Wall-clock of the most recent meaningful event.
    last_activity_ts: Option<DateTime<Utc>>,
    /// The now-function. Injectable for deterministic tests.
    now: fn() -> DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum LastAssistantShape {
    #[default]
    None,
    /// `assistantText` with `stop_reason = Some(_)`: turn complete.
    TextClosed,
    /// `assistantText` with no `stop_reason`: still streaming.
    TextStreaming,
    /// `assistantThinking`: mid-turn reasoning block.
    Thinking,
    /// `assistantToolUse` was the last shape we saw.
    ToolUse,
}

#[derive(Debug, Clone)]
struct OpenTool {
    tool_name: String,
    input_preview: String,
    started_at: Option<DateTime<Utc>>,
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
            recent_errors: Vec::new(),
            model: None,
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
                self.unmatched.remove(tool_use_id);
                if *is_error {
                    let now = *ts.as_ref().unwrap_or(&(self.now)());
                    self.recent_errors.push(now);
                }
                // Tool result → model is about to continue the turn.
                self.pending_reply = true;
            }
            SessionEvent::AssistantText {
                model, stop_reason, ..
            } => {
                if let Some(m) = model.clone() {
                    self.model = Some(m);
                }
                self.last_assistant = if stop_reason.is_some() {
                    LastAssistantShape::TextClosed
                } else {
                    LastAssistantShape::TextStreaming
                };
                if stop_reason.is_some() {
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
        // Prefer the oldest open tool call — that's usually the
        // user-relevant one ("running pnpm test" not "scheduled a
        // queued prompt"). BTreeMap iteration is insertion-agnostic,
        // so sort by started_at.
        let mut openings: Vec<_> = self.unmatched.values().collect();
        openings.sort_by_key(|o| o.started_at);
        let first = openings.first()?;
        let arg = humanize_tool_input(&first.tool_name, &first.input_preview);
        let text = if arg.is_empty() {
            first.tool_name.clone()
        } else {
            format!("{}: {}", first.tool_name, arg)
        };
        Some(truncate(&text, 80))
    }

    fn errored(&self) -> bool {
        let now = (self.now)();
        let cutoff = now - ERROR_WINDOW;
        self.recent_errors.iter().filter(|t| **t >= cutoff).count() >= ERROR_WINDOW_COUNT
    }

    fn stuck(&self) -> bool {
        let now = (self.now)();
        self.unmatched
            .values()
            .filter_map(|o| o.started_at)
            .any(|t| (now - t) > STUCK_THRESHOLD)
    }
}

fn event_ts(event: &SessionEvent) -> Option<DateTime<Utc>> {
    match event {
        SessionEvent::UserText { ts, .. }
        | SessionEvent::UserToolResult { ts, .. }
        | SessionEvent::AssistantText { ts, .. }
        | SessionEvent::AssistantToolUse { ts, .. }
        | SessionEvent::AssistantThinking { ts, .. }
        | SessionEvent::Summary { ts, .. }
        | SessionEvent::System { ts, .. }
        | SessionEvent::Attachment { ts, .. }
        | SessionEvent::FileHistorySnapshot { ts, .. }
        | SessionEvent::Other { ts, .. } => *ts,
        SessionEvent::Malformed { .. } => None,
    }
}

/// Extract the first human-meaningful argument from a tool's
/// `input_preview`. CC writes the input as `JSON.stringify(input)`
/// (see `session.rs::emit_assistant_events`), so for `Bash` we get
/// `{"command":"pnpm test","description":"..."}` — displaying that
/// verbatim is terrible UX.
///
/// Rule: if the preview parses as a JSON object, return the value of
/// the first known-relevant key (`command` for Bash, `file_path` for
/// Read/Edit/Write, `pattern` for Grep, `url` for WebFetch/WebSearch).
/// Fallback to the raw string if nothing matches — honest rendering
/// for tools we haven't mapped.
fn humanize_tool_input(tool_name: &str, preview: &str) -> String {
    let trimmed = preview.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !trimmed.starts_with('{') {
        return trimmed.to_string();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return trimmed.to_string();
    };
    let Some(obj) = v.as_object() else {
        return trimmed.to_string();
    };
    // Tool-specific first, then generic string-value fallback.
    let preferred_key: Option<&str> = match tool_name {
        "Bash" => Some("command"),
        "Read" | "Edit" | "Write" | "NotebookEdit" => Some("file_path"),
        "Grep" | "Glob" => Some("pattern"),
        "WebFetch" | "WebSearch" => Some("url"),
        _ => None,
    };
    if let Some(key) = preferred_key {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    // Fallback: the first string-valued field.
    obj.values()
        .find_map(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(hour: u32, min: u32, sec: u32) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(2026, 4, 21, hour, min, sec).unwrap())
    }

    /// A frozen clock the tests inject via `with_now`. The value is
    /// a second past the last event used in most fixtures.
    fn frozen_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 21, 10, 0, 30).unwrap()
    }

    fn machine() -> StatusMachine {
        StatusMachine::with_now(frozen_now)
    }

    // ── Derivation from event shape ────────────────────────────────

    #[test]
    fn fresh_machine_is_idle() {
        let m = machine();
        assert_eq!(m.snapshot().status, Status::Idle);
    }

    #[test]
    fn user_text_flips_to_busy() {
        let mut m = machine();
        m.ingest(&SessionEvent::UserText {
            ts: ts(10, 0, 0),
            uuid: None,
            text: "go".into(),
        });
        assert_eq!(m.snapshot().status, Status::Busy);
    }

    #[test]
    fn thinking_alone_is_busy() {
        let mut m = machine();
        m.ingest(&SessionEvent::UserText {
            ts: ts(10, 0, 0),
            uuid: None,
            text: "go".into(),
        });
        m.ingest(&SessionEvent::AssistantThinking {
            ts: ts(10, 0, 1),
            uuid: None,
            text: "thinking".into(),
        });
        assert_eq!(m.snapshot().status, Status::Busy);
    }

    #[test]
    fn streaming_text_is_busy() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantText {
            ts: ts(10, 0, 1),
            uuid: None,
            model: Some("claude-opus-4-7".into()),
            text: "partial".into(),
            usage: None,
            stop_reason: None,
        });
        let snap = m.snapshot();
        assert_eq!(snap.status, Status::Busy);
        assert_eq!(snap.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn closed_assistant_text_without_open_tool_is_idle() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantText {
            ts: ts(10, 0, 1),
            uuid: None,
            model: Some("claude-opus-4-7".into()),
            text: "done".into(),
            usage: None,
            stop_reason: Some("end_turn".into()),
        });
        assert_eq!(m.snapshot().status, Status::Idle);
    }

    #[test]
    fn unmatched_tool_use_keeps_busy_even_after_text_close() {
        // Simulates: assistant emits a final text block AND a
        // tool_use in the same turn, result hasn't landed yet.
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantText {
            ts: ts(10, 0, 1),
            uuid: None,
            model: None,
            text: "calling tool".into(),
            usage: None,
            stop_reason: Some("tool_use".into()),
        });
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 2),
            uuid: None,
            model: Some("claude-opus-4-7".into()),
            tool_name: "Bash".into(),
            tool_use_id: "tu1".into(),
            input_preview: "pnpm test".into(),
        });
        assert_eq!(m.snapshot().status, Status::Busy);
    }

    #[test]
    fn tool_result_closes_open_tool() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 1),
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "tu1".into(),
            input_preview: "pnpm test".into(),
        });
        assert_eq!(m.snapshot().status, Status::Busy);
        m.ingest(&SessionEvent::UserToolResult {
            ts: ts(10, 0, 2),
            uuid: None,
            tool_use_id: "tu1".into(),
            content: "ok".into(),
            is_error: false,
        });
        // pending_reply is true after the result — model is expected
        // to respond. Still busy.
        assert_eq!(m.snapshot().status, Status::Busy);
        // Now close the assistant turn.
        m.ingest(&SessionEvent::AssistantText {
            ts: ts(10, 0, 3),
            uuid: None,
            model: None,
            text: "done".into(),
            usage: None,
            stop_reason: Some("end_turn".into()),
        });
        assert_eq!(m.snapshot().status, Status::Idle);
    }

    #[test]
    fn system_turn_duration_alone_does_not_clear_open_tool() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 1),
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "tu1".into(),
            input_preview: "sleep".into(),
        });
        m.ingest(&SessionEvent::System {
            ts: ts(10, 0, 2),
            uuid: None,
            subtype: Some("turn_duration".into()),
            detail: "info".into(),
        });
        assert_eq!(m.snapshot().status, Status::Busy);
    }

    // ── Overlays ───────────────────────────────────────────────────

    #[test]
    fn error_overlay_triggers_on_two_within_window() {
        let mut m = machine();
        m.ingest(&SessionEvent::UserToolResult {
            ts: ts(10, 0, 10),
            uuid: None,
            tool_use_id: "a".into(),
            content: "x".into(),
            is_error: true,
        });
        assert!(!m.snapshot().errored, "single error ≠ overlay");
        m.ingest(&SessionEvent::UserToolResult {
            ts: ts(10, 0, 20),
            uuid: None,
            tool_use_id: "b".into(),
            content: "y".into(),
            is_error: true,
        });
        assert!(m.snapshot().errored, "two errors within window → overlay");
    }

    #[test]
    fn error_overlay_expires_when_window_slides_past() {
        let mut m = machine();
        // Two errors at 08:58 — both outside the 60s window relative
        // to frozen now (10:00:30).
        m.ingest(&SessionEvent::UserToolResult {
            ts: ts(8, 58, 0),
            uuid: None,
            tool_use_id: "a".into(),
            content: "x".into(),
            is_error: true,
        });
        m.ingest(&SessionEvent::UserToolResult {
            ts: ts(8, 58, 1),
            uuid: None,
            tool_use_id: "b".into(),
            content: "y".into(),
            is_error: true,
        });
        assert!(!m.snapshot().errored);
    }

    #[test]
    fn stuck_overlay_triggers_on_ancient_open_tool() {
        let mut m = machine();
        // Tool started at 09:40 — 20 minutes before frozen now.
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(9, 40, 0),
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "tu1".into(),
            input_preview: "long".into(),
        });
        let snap = m.snapshot();
        assert_eq!(snap.status, Status::Busy);
        assert!(snap.stuck, "20min open tool_use must flag stuck");
    }

    #[test]
    fn stuck_does_not_trigger_on_young_tool() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 10),
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "tu1".into(),
            input_preview: "quick".into(),
        });
        assert!(!m.snapshot().stuck);
    }

    // ── Current action derivation ──────────────────────────────────

    #[test]
    fn current_action_is_oldest_open_tool() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 10),
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "tu1".into(),
            input_preview: "pnpm test".into(),
        });
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 11),
            uuid: None,
            model: None,
            tool_name: "Read".into(),
            tool_use_id: "tu2".into(),
            input_preview: "src/foo.rs".into(),
        });
        assert_eq!(
            m.snapshot().current_action.as_deref(),
            Some("Bash: pnpm test")
        );
    }

    #[test]
    fn current_action_truncates_long_args() {
        let mut m = machine();
        // A long plain string (no JSON) so the humanizer returns it
        // verbatim and the truncator has to step in.
        let long = "a".repeat(200);
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 10),
            uuid: None,
            model: None,
            tool_name: "MysteryTool".into(),
            tool_use_id: "tu".into(),
            input_preview: long,
        });
        let ca = m.snapshot().current_action.unwrap();
        assert!(
            ca.chars().count() <= 80,
            "expected ≤80 chars, got {}",
            ca.chars().count()
        );
        assert!(ca.ends_with('…'));
    }

    #[test]
    fn humanize_extracts_bash_command() {
        let out = humanize_tool_input(
            "Bash",
            r#"{"command":"pnpm test","description":"run tests"}"#,
        );
        assert_eq!(out, "pnpm test");
    }

    #[test]
    fn humanize_extracts_read_file_path() {
        let out = humanize_tool_input("Read", r#"{"file_path":"src/foo.rs"}"#);
        assert_eq!(out, "src/foo.rs");
    }

    #[test]
    fn humanize_falls_back_to_first_string_for_unmapped_tool() {
        let out = humanize_tool_input(
            "CustomTool",
            r#"{"arg1":42,"arg2":"meaningful text"}"#,
        );
        assert_eq!(out, "meaningful text");
    }

    #[test]
    fn humanize_passes_through_non_json_input() {
        assert_eq!(humanize_tool_input("Bash", "raw text"), "raw text");
        assert_eq!(humanize_tool_input("Bash", ""), "");
    }

    #[test]
    fn current_action_none_when_no_open_tool() {
        let m = machine();
        assert!(m.snapshot().current_action.is_none());
    }

    // ── PID override ───────────────────────────────────────────────

    #[test]
    fn pid_waiting_overrides_derived_busy() {
        let mut m = machine();
        m.ingest(&SessionEvent::AssistantToolUse {
            ts: ts(10, 0, 10),
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "tu".into(),
            input_preview: "cmd".into(),
        });
        assert_eq!(m.snapshot().status, Status::Busy);
        m.set_pid_status(Some(Status::Waiting), Some("approve Bash".into()));
        let snap = m.snapshot();
        assert_eq!(snap.status, Status::Waiting);
        assert_eq!(snap.waiting_for.as_deref(), Some("approve Bash"));
    }

    #[test]
    fn waiting_for_cleared_when_status_changes_from_pid() {
        let mut m = machine();
        m.set_pid_status(Some(Status::Waiting), Some("approve".into()));
        assert_eq!(m.snapshot().waiting_for.as_deref(), Some("approve"));
        m.set_pid_status(Some(Status::Busy), Some("approve".into()));
        assert!(
            m.snapshot().waiting_for.is_none(),
            "waiting_for must only surface while status == Waiting"
        );
    }

    #[test]
    fn clearing_pid_status_falls_back_to_derived() {
        let mut m = machine();
        m.set_pid_status(Some(Status::Waiting), Some("approve".into()));
        m.set_pid_status(None, None);
        // No events ingested, so derived is Idle.
        assert_eq!(m.snapshot().status, Status::Idle);
    }

    // ── Tolerance / noise ──────────────────────────────────────────

    #[test]
    fn noise_events_do_not_shift_status() {
        let mut m = machine();
        m.ingest(&SessionEvent::Summary {
            ts: ts(10, 0, 1),
            uuid: None,
            text: "compact".into(),
        });
        m.ingest(&SessionEvent::FileHistorySnapshot {
            ts: ts(10, 0, 2),
            uuid: None,
            file_count: 0,
        });
        m.ingest(&SessionEvent::Attachment {
            ts: ts(10, 0, 3),
            uuid: None,
            name: None,
            mime: None,
        });
        m.ingest(&SessionEvent::Other {
            ts: ts(10, 0, 4),
            uuid: None,
            raw_type: "task-summary".into(),
        });
        m.ingest(&SessionEvent::Malformed {
            line_number: 42,
            error: "bad".into(),
            preview: "...".into(),
        });
        assert_eq!(m.snapshot().status, Status::Idle);
    }

    /// Golden test over a real JSONL fixture. This is the
    /// specimen-driven test Codex asked for: feed the full fixture,
    /// check the terminal status snapshot against an expected value.
    #[test]
    fn golden_busy_to_idle_fixture() {
        let raw = include_str!("testdata/jsonl/status-busy-to-idle.jsonl");
        let events = parse_for_test(raw);
        assert!(
            !events.is_empty(),
            "fixture should parse to non-empty event list"
        );
        let mut m = machine();
        for e in &events {
            m.ingest(e);
        }
        let snap = m.snapshot();
        assert_eq!(
            snap.status,
            Status::Idle,
            "turn closes at system/turn_duration → idle"
        );
        assert_eq!(snap.model.as_deref(), Some("claude-opus-4-7"));
        assert!(!snap.errored && !snap.stuck);
    }

    #[test]
    fn golden_unmatched_tool_fixture_is_busy_with_action() {
        let raw = include_str!("testdata/jsonl/status-busy-unmatched-tool.jsonl");
        let events = parse_for_test(raw);
        let mut m = machine();
        for e in &events {
            m.ingest(e);
        }
        let snap = m.snapshot();
        assert_eq!(snap.status, Status::Busy);
        assert_eq!(
            snap.current_action.as_deref(),
            Some("Bash: sleep 600")
        );
    }

    #[test]
    fn golden_errored_fixture_triggers_overlay() {
        let raw = include_str!("testdata/jsonl/status-errored.jsonl");
        let events = parse_for_test(raw);
        // Use a clock close to the fixture events so the errors are
        // inside the 60s window.
        let mut m = StatusMachine::with_now(|| {
            Utc.with_ymd_and_hms(2026, 4, 21, 10, 0, 30).unwrap()
        });
        for e in &events {
            m.ingest(e);
        }
        assert!(m.snapshot().errored);
    }

    /// Parse a JSONL fixture string using the same tolerant path as
    /// `session::parse_events`, but from an in-memory string so we
    /// don't round-trip through `tempfile`.
    fn parse_for_test(raw: &str) -> Vec<SessionEvent> {
        // Write to a temp file because the only public parse path
        // takes a `Path`. Keeping the helper tiny + file-scoped beats
        // exposing an internal `parse_events_line` from session.rs.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.jsonl");
        std::fs::write(&path, raw).unwrap();
        crate::session::parse_events_public(&path).unwrap()
    }
}
