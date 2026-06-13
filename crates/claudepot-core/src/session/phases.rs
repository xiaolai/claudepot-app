//! Split a session's events into **compaction phases**.
//!
//! CC emits a `summary` JSONL line whenever it compacts conversation
//! history. Everything after that line runs under a fresh context
//! window — the tokens attributed before the compaction aren't part of
//! the model's current view anymore.
//!
//! This module carves the raw event stream into `ContextPhase`s so the
//! UI can:
//!
//! * Show a compaction divider with cumulative "lost" tokens.
//! * Let the user flip between *current phase only* and *whole session*
//!   token-attribution views.
//! * Reset per-category injection accumulators inside
//!   [`session_context`](crate::session_context).
//!
//! Adapted from the `ContextPhaseInfo` / `processSessionContextWithPhases`
//! logic in claude-devtools.

use crate::session::SessionEvent;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// One slab of events that shares a context window.
///
/// Events in `events` are slices into the original transcript. Callers
/// that need metadata (e.g., tokens, message counts) should compute it
/// themselves from the event list or cross-reference chunks.
#[derive(Debug, Clone, Serialize)]
pub struct ContextPhase {
    /// 0-based. Phase 0 is always the initial (pre-compaction) run.
    pub phase_number: usize,
    /// Index of the first event in this phase inside the full stream.
    pub start_index: usize,
    /// Exclusive end index — `events[start_index..end_index]` is the
    /// slice that belongs to this phase.
    pub end_index: usize,
    /// Timestamp of the first event that has one, if any.
    pub start_ts: Option<DateTime<Utc>>,
    /// Timestamp of the last event that has one, if any.
    pub end_ts: Option<DateTime<Utc>>,
    /// Compaction summary text that opened this phase (empty for phase 0).
    pub summary: Option<String>,
}

/// Aggregate view of every phase in a session.
#[derive(Debug, Clone, Serialize)]
pub struct ContextPhaseInfo {
    /// All phases in order. Guaranteed non-empty whenever `events`
    /// contained at least one event.
    pub phases: Vec<ContextPhase>,
    /// Convenience: number of `summary` events encountered.
    pub compaction_count: usize,
}

/// Build phases from a session's event vector.
pub fn compute_phases(events: &[SessionEvent]) -> ContextPhaseInfo {
    if events.is_empty() {
        return ContextPhaseInfo {
            phases: Vec::new(),
            compaction_count: 0,
        };
    }

    // Collect every summary divider up front — simpler than interleaving
    // phase bookkeeping with the walk.
    let summaries: Vec<(usize, String)> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            SessionEvent::Summary { text, .. } => Some((i, text.clone())),
            _ => None,
        })
        .collect();

    let compaction_count = summaries.len();
    let mut phases = Vec::with_capacity(compaction_count + 1);
    let mut cursor = 0usize;
    let mut pending_summary: Option<String> = None;

    for (phase_number, (idx, summary_text)) in summaries.iter().enumerate() {
        phases.push(make_phase(
            phase_number,
            cursor,
            *idx,
            events,
            pending_summary.take(),
        ));
        cursor = idx + 1;
        pending_summary = Some(summary_text.clone());
    }

    // Trailing phase (may be empty when the transcript ends on a summary).
    phases.push(make_phase(
        phases.len(),
        cursor,
        events.len(),
        events,
        pending_summary,
    ));

    ContextPhaseInfo {
        phases,
        compaction_count,
    }
}

fn make_phase(
    phase_number: usize,
    start_index: usize,
    end_index: usize,
    events: &[SessionEvent],
    summary: Option<String>,
) -> ContextPhase {
    let slice = &events[start_index..end_index];
    let start_ts = slice.iter().find_map(ts_of);
    let end_ts = slice.iter().rev().find_map(ts_of);
    ContextPhase {
        phase_number,
        start_index,
        end_index,
        start_ts,
        end_ts,
        summary,
    }
}

fn ts_of(ev: &SessionEvent) -> Option<DateTime<Utc>> {
    match ev {
        SessionEvent::UserText { ts, .. }
        | SessionEvent::UserToolResult { ts, .. }
        | SessionEvent::AssistantText { ts, .. }
        | SessionEvent::AssistantToolUse { ts, .. }
        | SessionEvent::AssistantThinking { ts, .. }
        | SessionEvent::Summary { ts, .. }
        | SessionEvent::System { ts, .. }
        | SessionEvent::Attachment { ts, .. }
        | SessionEvent::FileHistorySnapshot { ts, .. }
        | SessionEvent::TaskSummary { ts, .. }
        | SessionEvent::Other { ts, .. } => *ts,
        SessionEvent::Malformed { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse::<DateTime<Utc>>().unwrap())
    }

    fn user(t: Option<DateTime<Utc>>, s: &str) -> SessionEvent {
        SessionEvent::UserText {
            ts: t,
            uuid: None,
            text: s.into(),
        }
    }

    fn asst(t: Option<DateTime<Utc>>, s: &str) -> SessionEvent {
        SessionEvent::AssistantText {
            ts: t,
            uuid: None,
            model: None,
            text: s.into(),
            usage: None,
            stop_reason: None,
        }
    }

    fn summary(t: Option<DateTime<Utc>>, s: &str) -> SessionEvent {
        SessionEvent::Summary {
            ts: t,
            uuid: None,
            text: s.into(),
        }
    }

    #[test]
    fn empty_events_yield_no_phases() {
        let info = compute_phases(&[]);
        assert!(info.phases.is_empty());
        assert_eq!(info.compaction_count, 0);
    }

    #[test]
    fn no_summary_yields_one_phase() {
        let events = vec![
            user(ts("2026-04-10T10:00:00Z"), "hi"),
            asst(ts("2026-04-10T10:00:01Z"), "hello"),
        ];
        let info = compute_phases(&events);
        assert_eq!(info.phases.len(), 1);
        assert_eq!(info.compaction_count, 0);
        let p = &info.phases[0];
        assert_eq!(p.phase_number, 0);
        assert_eq!(p.start_index, 0);
        assert_eq!(p.end_index, 2);
        assert!(p.summary.is_none());
    }

    #[test]
    fn one_compaction_yields_two_phases() {
        let events = vec![
            user(ts("2026-04-10T10:00:00Z"), "round1"),
            asst(ts("2026-04-10T10:00:01Z"), "r1 reply"),
            summary(ts("2026-04-10T10:00:02Z"), "compacted first pass"),
            user(ts("2026-04-10T10:00:03Z"), "round2"),
        ];
        let info = compute_phases(&events);
        assert_eq!(info.phases.len(), 2);
        assert_eq!(info.compaction_count, 1);
        assert_eq!(info.phases[0].start_index, 0);
        assert_eq!(info.phases[0].end_index, 2);
        assert_eq!(info.phases[1].start_index, 3);
        assert_eq!(info.phases[1].end_index, 4);
        assert_eq!(
            info.phases[1].summary.as_deref(),
            Some("compacted first pass")
        );
    }

    #[test]
    fn multiple_compactions_cascade() {
        let events = vec![
            user(None, "a"),
            summary(None, "c1"),
            user(None, "b"),
            summary(None, "c2"),
            user(None, "c"),
        ];
        let info = compute_phases(&events);
        assert_eq!(info.phases.len(), 3);
        assert_eq!(info.compaction_count, 2);
        assert_eq!(info.phases[1].summary.as_deref(), Some("c1"));
        assert_eq!(info.phases[2].summary.as_deref(), Some("c2"));
    }

    #[test]
    fn trailing_summary_closes_phase_with_empty_tail() {
        let events = vec![user(None, "hi"), summary(None, "c1")];
        let info = compute_phases(&events);
        assert_eq!(info.phases.len(), 2);
        assert_eq!(info.phases[1].start_index, 2);
        assert_eq!(info.phases[1].end_index, 2);
        assert_eq!(info.phases[1].summary.as_deref(), Some("c1"));
    }

    #[test]
    fn phase_timestamps_track_first_and_last_event() {
        let events = vec![
            user(ts("2026-04-10T10:00:00Z"), "a"),
            asst(ts("2026-04-10T10:05:00Z"), "b"),
            summary(ts("2026-04-10T10:05:01Z"), "c1"),
            user(ts("2026-04-10T10:10:00Z"), "c"),
            asst(ts("2026-04-10T10:15:00Z"), "d"),
        ];
        let info = compute_phases(&events);
        assert_eq!(info.phases[0].start_ts, ts("2026-04-10T10:00:00Z"));
        assert_eq!(info.phases[0].end_ts, ts("2026-04-10T10:05:00Z"));
        assert_eq!(info.phases[1].start_ts, ts("2026-04-10T10:10:00Z"));
        assert_eq!(info.phases[1].end_ts, ts("2026-04-10T10:15:00Z"));
    }
}
