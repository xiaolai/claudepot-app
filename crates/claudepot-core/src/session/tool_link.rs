//! Pair `AssistantToolUse` events with their matching `UserToolResult`
//! by `tool_use_id`. Adapted from claude-devtools' `toolLinkingEngine`.
//!
//! A linked tool is a complete call/response round-trip. The UI can
//! render one collapsible block per pair rather than two unrelated
//! bubbles; token attribution can bill the call and the output together;
//! exporters can fold the result into a single markdown section.
//!
//! Orphaned calls (no matching result, because the session was
//! interrupted or the result never came back) are kept with
//! `result: None` — silently dropping them hides broken turns.

use crate::session::SessionEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One call → result pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedTool {
    /// The tool_use_id that joins call and result.
    pub tool_use_id: String,
    /// Tool name as reported by the assistant (`Read`, `Bash`, …).
    pub tool_name: String,
    /// Model that issued the call, if recorded on the assistant turn.
    pub model: Option<String>,
    /// When the assistant emitted the tool call.
    pub call_ts: Option<DateTime<Utc>>,
    /// Truncated preview of the input JSON.
    pub input_preview: String,
    /// Raw JSON of the tool input, untruncated. Feeds the detail-level
    /// substring search; never rendered verbatim.
    #[serde(default)]
    pub input_full: String,
    /// When the matching result arrived. `None` → orphaned call.
    pub result_ts: Option<DateTime<Utc>>,
    /// Raw result payload (string form). `None` → orphaned call.
    pub result_content: Option<String>,
    /// `is_error` flag on the result, `false` for orphaned calls.
    pub is_error: bool,
    /// Milliseconds from call → result. `None` when orphaned or when
    /// timestamps are unavailable on either side.
    pub duration_ms: Option<i64>,
    /// Index of the originating `AssistantToolUse` inside the event
    /// vector — lets UIs render the linked block in place without
    /// re-scanning.
    pub call_index: usize,
    /// Index of the matching `UserToolResult`, or `None` if orphaned.
    pub result_index: Option<usize>,
}

/// Walk `events` once, emit pair metadata for every tool call.
///
/// Pairing strategy:
///
/// * First pass — map `tool_use_id` → index of the `UserToolResult`.
/// * Second pass — for every `AssistantToolUse`, look up its matching
///   result and build a `LinkedTool`.
///
/// Running time is O(n). Duplicates (multiple results claiming the same
/// `tool_use_id`) resolve to the last one written — CC shouldn't emit
/// duplicates, and if it does the last one wins, which matches what the
/// user sees in the live terminal.
pub fn link_tools(events: &[SessionEvent]) -> Vec<LinkedTool> {
    let results_by_id = index_results(events);
    let mut out = Vec::new();
    for (idx, ev) in events.iter().enumerate() {
        if let SessionEvent::AssistantToolUse {
            ts,
            model,
            tool_name,
            tool_use_id,
            input_preview,
            input_full,
            ..
        } = ev
        {
            if tool_use_id.is_empty() {
                continue;
            }
            let result_idx = results_by_id.get(tool_use_id).copied();
            let (result_ts, result_content, is_error) = match result_idx {
                Some(i) => extract_result(&events[i]),
                None => (None, None, false),
            };
            let duration_ms = match (*ts, result_ts) {
                (Some(a), Some(b)) => Some((b - a).num_milliseconds()),
                _ => None,
            };
            out.push(LinkedTool {
                tool_use_id: tool_use_id.clone(),
                tool_name: tool_name.clone(),
                model: model.clone(),
                call_ts: *ts,
                input_preview: input_preview.clone(),
                input_full: input_full.clone(),
                result_ts,
                result_content,
                is_error,
                duration_ms,
                call_index: idx,
                result_index: result_idx,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn index_results(events: &[SessionEvent]) -> HashMap<String, usize> {
    let mut m: HashMap<String, usize> = HashMap::new();
    for (idx, ev) in events.iter().enumerate() {
        if let SessionEvent::UserToolResult { tool_use_id, .. } = ev {
            if !tool_use_id.is_empty() {
                m.insert(tool_use_id.clone(), idx);
            }
        }
    }
    m
}

fn extract_result(ev: &SessionEvent) -> (Option<DateTime<Utc>>, Option<String>, bool) {
    match ev {
        SessionEvent::UserToolResult {
            ts,
            content,
            is_error,
            ..
        } => (*ts, Some(content.clone()), *is_error),
        _ => (None, None, false),
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

    fn tool_use(id: &str, name: &str, t: Option<DateTime<Utc>>) -> SessionEvent {
        let input = format!("{{\"for\":\"{id}\"}}");
        SessionEvent::AssistantToolUse {
            ts: t,
            uuid: None,
            model: Some("claude-opus-4-7".into()),
            tool_name: name.into(),
            tool_use_id: id.into(),
            input_preview: input.clone(),
            input_full: input,
        }
    }

    fn tool_result(id: &str, content: &str, err: bool, t: Option<DateTime<Utc>>) -> SessionEvent {
        SessionEvent::UserToolResult {
            ts: t,
            uuid: None,
            tool_use_id: id.into(),
            content: content.into(),
            is_error: err,
        }
    }

    #[test]
    fn pairs_matching_call_and_result() {
        let events = vec![
            tool_use("a", "Read", ts("2026-04-10T10:00:00Z")),
            tool_result("a", "file body", false, ts("2026-04-10T10:00:01Z")),
        ];
        let linked = link_tools(&events);
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].tool_use_id, "a");
        assert_eq!(linked[0].tool_name, "Read");
        assert_eq!(linked[0].result_content.as_deref(), Some("file body"));
        assert_eq!(linked[0].duration_ms, Some(1000));
        assert!(!linked[0].is_error);
        assert_eq!(linked[0].call_index, 0);
        assert_eq!(linked[0].result_index, Some(1));
    }

    #[test]
    fn orphaned_call_surfaces_as_linked_with_none_result() {
        let events = vec![tool_use("a", "Bash", ts("2026-04-10T10:00:00Z"))];
        let linked = link_tools(&events);
        assert_eq!(linked.len(), 1);
        assert!(linked[0].result_content.is_none());
        assert!(linked[0].result_index.is_none());
        assert_eq!(linked[0].duration_ms, None);
    }

    #[test]
    fn preserves_error_flag_on_result() {
        let events = vec![
            tool_use("a", "Bash", None),
            tool_result("a", "denied", true, None),
        ];
        let linked = link_tools(&events);
        assert!(linked[0].is_error);
    }

    #[test]
    fn orphaned_result_without_call_is_ignored() {
        // result comes first, no matching call — we still skip it.
        let events = vec![tool_result("a", "data", false, None)];
        let linked = link_tools(&events);
        assert!(linked.is_empty());
    }

    #[test]
    fn empty_tool_use_id_is_skipped() {
        let events = vec![
            tool_use("", "Read", None),
            tool_result("", "x", false, None),
        ];
        let linked = link_tools(&events);
        assert!(linked.is_empty());
    }

    #[test]
    fn duration_handles_missing_timestamp_on_either_side() {
        let cases = vec![
            (None, None),
            (ts("2026-04-10T10:00:00Z"), None),
            (None, ts("2026-04-10T10:00:01Z")),
        ];
        for (t1, t2) in cases {
            let events = vec![tool_use("a", "X", t1), tool_result("a", "y", false, t2)];
            let linked = link_tools(&events);
            assert_eq!(linked[0].duration_ms, None);
        }
    }
}
