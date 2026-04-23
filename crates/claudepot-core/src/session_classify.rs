//! Message classification — maps `SessionEvent`s onto five categories
//! that drive chunk building and noise filtering in the Sessions UI.
//!
//! The taxonomy is adapted from claude-devtools'
//! [`MessageClassifier`](../../../../../../../../../github/claude-devtools/src/main/services/parsing/MessageClassifier.ts),
//! rewritten against our own `SessionEvent` variants so the logic can
//! live in pure Rust without the TypeScript type-guard gymnastics.
//!
//! Rules (highest priority first):
//!
//! 1. `Summary` event         → `Compact`    (conversation boundary)
//! 2. `FileHistorySnapshot`   → `HardNoise`  (CC-internal bookkeeping)
//! 3. `Other` with raw type
//!    `queue-operation`       → `HardNoise`
//! 4. `System`                → `HardNoise`  (level=info / turn_duration)
//! 5. `UserText` whose payload is **only** noise tags
//!    (`<local-command-caveat>`, `<system-reminder>`,
//!    `<command-message>`, `<command-args>`) → `HardNoise`
//! 6. `UserText` with a `<local-command-stdout>` tag → `System`
//!    (slash-command output, renders left like AI)
//! 7. Any other `UserText`    → `User`
//! 8. `UserToolResult` / `AssistantText` / `AssistantToolUse` /
//!    `AssistantThinking` / `Attachment` / `Other` / `Malformed` → `Ai`
//!
//! The `HardNoise` rows still exist in the raw `Vec<SessionEvent>` —
//! classification only *tags* them. Callers decide whether to filter.

use crate::session::SessionEvent;
use serde::{Deserialize, Serialize};

/// Display category for a `SessionEvent`. Drives chunk building:
/// `User` starts a new [`SessionChunk::User`], `System` starts a
/// [`SessionChunk::System`], `Compact` starts a
/// [`SessionChunk::Compact`], and `Ai` rows are coalesced into the
/// preceding AI chunk. `HardNoise` is filtered out.
///
/// [`SessionChunk::User`]: crate::session_chunks::SessionChunk
/// [`SessionChunk::System`]: crate::session_chunks::SessionChunk
/// [`SessionChunk::Compact`]: crate::session_chunks::SessionChunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageCategory {
    /// Genuine user input — starts a new `UserChunk`.
    User,
    /// Slash-command stdout — renders left like AI but stands alone.
    System,
    /// Conversation compaction marker.
    Compact,
    /// Internal bookkeeping that should never render.
    HardNoise,
    /// Assistant output or tool traffic — coalesced into `AiChunk`s.
    Ai,
}

/// Classify a single `SessionEvent`.
pub fn classify_event(event: &SessionEvent) -> MessageCategory {
    match event {
        SessionEvent::Summary { .. } => MessageCategory::Compact,
        SessionEvent::FileHistorySnapshot { .. } => MessageCategory::HardNoise,
        SessionEvent::System { .. } => MessageCategory::HardNoise,
        SessionEvent::Other { raw_type, .. } if is_hard_noise_raw_type(raw_type) => {
            MessageCategory::HardNoise
        }
        SessionEvent::UserText { text, .. } => classify_user_text(text),
        // Task-summary is CC-internal bookkeeping for `claude ps` —
        // not something to render in the transcript view. HardNoise
        // so the chunk builder filters it out.
        SessionEvent::TaskSummary { .. } => MessageCategory::HardNoise,
        SessionEvent::UserToolResult { .. }
        | SessionEvent::AssistantText { .. }
        | SessionEvent::AssistantToolUse { .. }
        | SessionEvent::AssistantThinking { .. }
        | SessionEvent::Attachment { .. }
        | SessionEvent::Other { .. }
        | SessionEvent::Malformed { .. } => MessageCategory::Ai,
    }
}

/// Batch classifier. Preserves order, O(n).
pub fn classify_all(events: &[SessionEvent]) -> Vec<(MessageCategory, usize)> {
    events
        .iter()
        .enumerate()
        .map(|(i, e)| (classify_event(e), i))
        .collect()
}

// ---------------------------------------------------------------------------
// Noise heuristics
// ---------------------------------------------------------------------------

fn is_hard_noise_raw_type(raw: &str) -> bool {
    matches!(
        raw,
        "queue-operation" | "file-history-snapshot" | "turn_duration" | "init"
    )
}

fn classify_user_text(text: &str) -> MessageCategory {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return MessageCategory::HardNoise;
    }
    // Slash-command stdout — left-column bubble, its own chunk.
    if contains_tag(trimmed, "local-command-stdout") {
        return MessageCategory::System;
    }
    // If the payload is *exclusively* noise tags, drop it.
    if is_noise_only(trimmed) {
        return MessageCategory::HardNoise;
    }
    MessageCategory::User
}

/// Returns true when every non-empty line of `text` is wrapped by a
/// known noise tag (or is blank). A single visible sentence outside the
/// tags promotes the message back to `User`.
fn is_noise_only(text: &str) -> bool {
    let stripped = strip_noise_tags(text);
    stripped.trim().is_empty()
}

/// Drop every noise tag's wrapper *and* content. Anything left over is
/// what the user actually typed (or `<command-name>` payload, which is
/// visible input — slash-command invocations count as user intent).
fn strip_noise_tags(text: &str) -> String {
    const NOISE_TAGS: &[&str] = &[
        "local-command-caveat",
        "system-reminder",
        "command-message",
        "command-args",
    ];
    let mut buf = text.to_string();
    for tag in NOISE_TAGS {
        buf = strip_tag_block(&buf, tag);
    }
    buf
}

fn strip_tag_block(text: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find(&open) {
            Some(o) => {
                out.push_str(&rest[..o]);
                rest = &rest[o + open.len()..];
                match rest.find(&close) {
                    Some(c) => {
                        rest = &rest[c + close.len()..];
                    }
                    None => {
                        // Unclosed tag — consume the rest defensively.
                        rest = "";
                        break;
                    }
                }
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn contains_tag(text: &str, tag: &str) -> bool {
    let open = format!("<{tag}");
    text.contains(&open)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn user_text(s: &str) -> SessionEvent {
        SessionEvent::UserText {
            ts: Some("2026-04-10T10:00:00Z".parse::<DateTime<Utc>>().unwrap()),
            uuid: Some("u1".into()),
            text: s.to_string(),
        }
    }

    fn assistant_text(s: &str) -> SessionEvent {
        SessionEvent::AssistantText {
            ts: None,
            uuid: None,
            model: Some("claude-opus-4-7".into()),
            text: s.to_string(),
            usage: None,
            stop_reason: None,
        }
    }

    #[test]
    fn plain_user_text_is_user() {
        let e = user_text("fix the build");
        assert_eq!(classify_event(&e), MessageCategory::User);
    }

    #[test]
    fn empty_user_text_is_hard_noise() {
        let e = user_text("   \n  ");
        assert_eq!(classify_event(&e), MessageCategory::HardNoise);
    }

    #[test]
    fn local_command_stdout_is_system() {
        let e = user_text("<local-command-stdout>fork sha</local-command-stdout>");
        assert_eq!(classify_event(&e), MessageCategory::System);
    }

    #[test]
    fn local_command_caveat_only_is_hard_noise() {
        let e = user_text("<local-command-caveat>ignore</local-command-caveat>");
        assert_eq!(classify_event(&e), MessageCategory::HardNoise);
    }

    #[test]
    fn system_reminder_only_is_hard_noise() {
        let e = user_text("<system-reminder>todo list reminder</system-reminder>");
        assert_eq!(classify_event(&e), MessageCategory::HardNoise);
    }

    #[test]
    fn command_name_payload_is_user_input() {
        // /model typed by the user — visible slash command, not noise.
        let e = user_text("<command-name>/model</command-name>");
        assert_eq!(classify_event(&e), MessageCategory::User);
    }

    #[test]
    fn noise_tags_mixed_with_real_text_is_user() {
        let e = user_text(
            "<system-reminder>hi</system-reminder>real follow-up question",
        );
        assert_eq!(classify_event(&e), MessageCategory::User);
    }

    #[test]
    fn summary_is_compact() {
        let e = SessionEvent::Summary {
            ts: None,
            uuid: None,
            text: "compacted".into(),
        };
        assert_eq!(classify_event(&e), MessageCategory::Compact);
    }

    #[test]
    fn system_event_is_hard_noise() {
        let e = SessionEvent::System {
            ts: None,
            uuid: None,
            subtype: Some("init".into()),
            detail: "info".into(),
        };
        assert_eq!(classify_event(&e), MessageCategory::HardNoise);
    }

    #[test]
    fn file_history_snapshot_is_hard_noise() {
        let e = SessionEvent::FileHistorySnapshot {
            ts: None,
            uuid: None,
            file_count: 3,
        };
        assert_eq!(classify_event(&e), MessageCategory::HardNoise);
    }

    #[test]
    fn queue_operation_is_hard_noise() {
        let e = SessionEvent::Other {
            ts: None,
            uuid: None,
            raw_type: "queue-operation".into(),
        };
        assert_eq!(classify_event(&e), MessageCategory::HardNoise);
    }

    #[test]
    fn unknown_other_is_ai() {
        let e = SessionEvent::Other {
            ts: None,
            uuid: None,
            raw_type: "future-cc-type".into(),
        };
        assert_eq!(classify_event(&e), MessageCategory::Ai);
    }

    #[test]
    fn assistant_text_is_ai() {
        let e = assistant_text("response");
        assert_eq!(classify_event(&e), MessageCategory::Ai);
    }

    #[test]
    fn tool_use_and_result_are_ai() {
        let use_ev = SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: "Read".into(),
            tool_use_id: "t1".into(),
            input_preview: "{}".into(),
            input_full: "{}".into(),
        };
        let res_ev = SessionEvent::UserToolResult {
            ts: None,
            uuid: None,
            tool_use_id: "t1".into(),
            content: "data".into(),
            is_error: false,
        };
        assert_eq!(classify_event(&use_ev), MessageCategory::Ai);
        assert_eq!(classify_event(&res_ev), MessageCategory::Ai);
    }

    #[test]
    fn thinking_is_ai() {
        let e = SessionEvent::AssistantThinking {
            ts: None,
            uuid: None,
            text: "reasoning".into(),
        };
        assert_eq!(classify_event(&e), MessageCategory::Ai);
    }

    #[test]
    fn malformed_is_ai_so_it_still_renders() {
        let e = SessionEvent::Malformed {
            line_number: 42,
            error: "bad json".into(),
            preview: "{...".into(),
        };
        assert_eq!(classify_event(&e), MessageCategory::Ai);
    }

    #[test]
    fn classify_all_preserves_order() {
        let events = vec![
            user_text("hi"),
            assistant_text("hello"),
            SessionEvent::Summary {
                ts: None,
                uuid: None,
                text: "done".into(),
            },
            user_text("<local-command-stdout>out</local-command-stdout>"),
        ];
        let cats = classify_all(&events);
        assert_eq!(
            cats,
            vec![
                (MessageCategory::User, 0),
                (MessageCategory::Ai, 1),
                (MessageCategory::Compact, 2),
                (MessageCategory::System, 3),
            ]
        );
    }

    #[test]
    fn strip_tag_block_handles_multiple_tags() {
        let s = "<system-reminder>a</system-reminder>mid<command-args>b</command-args>tail";
        let out = strip_noise_tags(s);
        assert_eq!(out, "midtail");
    }

    #[test]
    fn strip_tag_block_tolerates_unclosed_tag() {
        let s = "<system-reminder>unterminated";
        let out = strip_noise_tags(s);
        assert_eq!(out.trim(), "");
    }
}
