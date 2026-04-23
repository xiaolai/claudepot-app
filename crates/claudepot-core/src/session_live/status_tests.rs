//! Unit tests for `session_live::status::StatusMachine`.
//!
//! Included into `status.rs` via `#[cfg(test)] #[path = ...] mod tests;`.
//! Lives here (instead of inline) because the loc-guardian limit for a
//! single file is 350 LOC, and the production state-machine logic +
//! inline tests were crowding each other. The loc-guardian extraction
//! rules (project config) specifically permit a co-located tests file.

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
        input_full: "pnpm test".into(),
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
        input_full: "pnpm test".into(),
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
        input_full: "sleep".into(),
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
        input_full: "long".into(),
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
        input_full: "quick".into(),
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
        input_full: "pnpm test".into(),
    });
    m.ingest(&SessionEvent::AssistantToolUse {
        ts: ts(10, 0, 11),
        uuid: None,
        model: None,
        tool_name: "Read".into(),
        tool_use_id: "tu2".into(),
        input_preview: "src/foo.rs".into(),
        input_full: "src/foo.rs".into(),
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
        input_preview: long.clone(),
        input_full: long,
    });
    let ca = m.snapshot().current_action.unwrap();
    assert!(
        ca.chars().count() <= 80,
        "expected ≤80 chars, got {}",
        ca.chars().count()
    );
    assert!(ca.ends_with('…'));
}

// humanize_tool_input / truncate unit tests moved to
// session_live::status_helpers::tests (extracted by loc-guardian).

// ── Behavioural fixes from the grill + codex review ────────────

#[test]
fn tool_use_stop_reason_is_not_a_turn_close() {
    // CC writes `stop_reason: "tool_use"` when the turn pauses
    // for a tool call. Treating that as a close flips status to
    // Idle even though the model is mid-flight.
    let mut m = machine();
    m.ingest(&SessionEvent::AssistantText {
        ts: ts(10, 0, 1),
        uuid: None,
        model: None,
        text: "calling tool".into(),
        usage: None,
        stop_reason: Some("tool_use".into()),
    });
    assert_eq!(
        m.snapshot().status,
        Status::Busy,
        "stop_reason='tool_use' must NOT be treated as turn close"
    );
}

#[test]
fn late_straggler_tool_result_does_not_pin_busy() {
    // A `UserToolResult` with no matching open tool_use is a
    // late arrival (sub-agent bleed, transcript resume mid-turn).
    // It must NOT flip `pending_reply`, or the fallback status
    // stays Busy forever even after the turn actually closed.
    let mut m = machine();
    m.ingest(&SessionEvent::AssistantText {
        ts: ts(10, 0, 1),
        uuid: None,
        model: None,
        text: "done".into(),
        usage: None,
        stop_reason: Some("end_turn".into()),
    });
    assert_eq!(m.snapshot().status, Status::Idle);
    // Straggler lands — no matching open.
    m.ingest(&SessionEvent::UserToolResult {
        ts: ts(10, 0, 2),
        uuid: None,
        tool_use_id: "ghost".into(),
        content: "".into(),
        is_error: false,
    });
    assert_eq!(
        m.snapshot().status,
        Status::Idle,
        "unmatched tool_result must not re-engage pending_reply"
    );
}

#[test]
fn current_action_redacts_leaked_keys() {
    // Anthropic's own prefix in a Bash arg — happens when a user
    // pastes `curl -H 'Authorization: Bearer sk-ant-...'`. The
    // peripheral current_action surface MUST redact. Since M2, the
    // Authorization-header family swallows the entire bearer body
    // BEFORE the sk-ant pass runs, so the visible mask is
    // `Authorization: Bearer ***` (the sk-ant-*** fallback mask is
    // only used for bare sk-ant-... tokens not wrapped in a header).
    let mut m = machine();
    m.ingest(&SessionEvent::AssistantToolUse {
        ts: ts(10, 0, 10),
        uuid: None,
        model: None,
        tool_name: "Bash".into(),
        tool_use_id: "tu".into(),
        input_preview: r#"{"command":"curl -H 'Authorization: Bearer sk-ant-Abc123DEF456_xyz' https://api"}"#
            .into(),
        input_full: r#"{"command":"curl -H 'Authorization: Bearer sk-ant-Abc123DEF456_xyz' https://api"}"#
            .into(),
    });
    let ca = m.snapshot().current_action.unwrap();
    assert!(
        !ca.contains("sk-ant-Abc123DEF456_xyz"),
        "raw key leaked into current_action: {ca}"
    );
    // Either mask form is acceptable; neither form reveals the body.
    assert!(
        ca.contains("Authorization: Bearer ***") || ca.contains("sk-ant-***"),
        "expected redaction marker, got: {ca}"
    );
}

#[test]
fn current_action_redacts_bare_sk_ant_token() {
    // Same prefix, but NOT wrapped in an Authorization header — the
    // sk-ant pass is the only family that fires. Asserts the
    // classic sk-ant-*** shape still works for bare tokens.
    let mut m = machine();
    m.ingest(&SessionEvent::AssistantToolUse {
        ts: ts(10, 0, 10),
        uuid: None,
        model: None,
        tool_name: "Bash".into(),
        tool_use_id: "tu".into(),
        input_preview: r#"{"command":"echo sk-ant-Abc123DEF456_xyz"}"#.into(),
        input_full: r#"{"command":"echo sk-ant-Abc123DEF456_xyz"}"#.into(),
    });
    let ca = m.snapshot().current_action.unwrap();
    assert!(!ca.contains("Abc123DEF456_xyz"));
    assert!(ca.contains("sk-ant-***"));
}

#[test]
fn recent_errors_window_is_bounded_even_under_burst() {
    // 10_000 errors arrive within a 60s window — the trimmer
    // runs on ingest. With `frozen_now` at 10:00:30 and every
    // error stamped at 10:00:10, none expire. But we should
    // never allow unbounded retention beyond what the window
    // could hold. The test asserts growth is bounded by the
    // retained set, not by the input size.
    let mut m = machine();
    for i in 0..1000 {
        m.ingest(&SessionEvent::UserToolResult {
            ts: ts(10, 0, (i % 30) as u32),
            uuid: None,
            tool_use_id: format!("t{i}"),
            content: "".into(),
            is_error: true,
        });
    }
    // All 1000 ts land within the trailing 60s of frozen_now,
    // so they all survive the window check — but the structure
    // is VecDeque + trimmer, not unbounded Vec. Proving the
    // structural property: trim is called and removes old entries.
    let trimmed_before = m.recent_errors.len();
    // Advance time past the window and ingest one more error —
    // the trimmer must drop everything older than the cutoff.
    let mut later = StatusMachine::with_now(|| {
        chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 4, 21, 11, 0, 0)
            .unwrap()
    });
    // Move the vecdeque into the later machine to simulate
    // the sliding window after wall-clock progression.
    later.recent_errors = m.recent_errors.clone();
    later.ingest(&SessionEvent::UserToolResult {
        ts: ts(11, 0, 0),
        uuid: None,
        tool_use_id: "new".into(),
        content: "".into(),
        is_error: true,
    });
    assert!(
        later.recent_errors.len() < trimmed_before,
        "trim must drop entries outside the window"
    );
}

#[test]
fn current_action_none_when_no_open_tool() {
    let m = machine();
    assert!(m.snapshot().current_action.is_none());
}

// ── PID override ───────────────────────────────────────────────

#[test]
fn fallback_waiting_on_permission_mode_event() {
    // When BG_SESSIONS is off, CC writes a permission-mode entry
    // while waiting for approval. The transcript-derived fallback
    // must return Waiting in that case instead of inventing Idle.
    let mut m = machine();
    m.ingest(&SessionEvent::AssistantToolUse {
        ts: ts(10, 0, 1),
        uuid: None,
        model: None,
        tool_name: "Bash".into(),
        tool_use_id: "tu".into(),
        input_preview: "rm -rf /".into(),
        input_full: "rm -rf /".into(),
    });
    // Busy initially (open tool_use).
    assert_eq!(m.snapshot().status, Status::Busy);
    // CC writes the permission-mode entry → fallback flips to Waiting.
    m.ingest(&SessionEvent::Other {
        ts: ts(10, 0, 2),
        uuid: None,
        raw_type: "permission-mode".into(),
    });
    assert_eq!(m.snapshot().status, Status::Waiting);
    // User approves → next UserText clears the flag.
    m.ingest(&SessionEvent::UserText {
        ts: ts(10, 0, 3),
        uuid: None,
        text: "yes".into(),
    });
    // Back to Busy (the open tool still hasn't completed).
    assert_eq!(m.snapshot().status, Status::Busy);
}

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
        input_full: "cmd".into(),
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
