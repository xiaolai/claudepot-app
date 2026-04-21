//! Free helpers for the status state machine.
//!
//! Split out of `status.rs` per the loc-guardian extraction rule for
//! private helper functions. Each of these is testable in isolation
//! and has no dependency on `StatusMachine` internal state.

use chrono::{DateTime, Utc};

use crate::session::SessionEvent;

/// Pluck the timestamp from any `SessionEvent` variant that carries
/// one. Returns `None` for `Malformed` (which has no ts field).
pub(super) fn event_ts(event: &SessionEvent) -> Option<DateTime<Utc>> {
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
pub(super) fn humanize_tool_input(tool_name: &str, preview: &str) -> String {
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
    obj.values()
        .find_map(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

/// Truncate `s` to at most `max` characters, appending `…` if cut.
/// Counts chars, not bytes, so multi-byte UTF-8 doesn't explode.
pub(super) fn truncate(s: &str, max: usize) -> String {
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
    fn truncate_keeps_short_strings_unchanged() {
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn truncate_adds_ellipsis_when_cut() {
        let long = "a".repeat(200);
        let out = truncate(&long, 80);
        assert_eq!(out.chars().count(), 80);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        // 10 × 3-byte char = 30 bytes, 10 chars.
        let multi = "日".repeat(10);
        let out = truncate(&multi, 5);
        assert_eq!(out.chars().count(), 5);
        assert!(out.ends_with('…'));
    }
}
