//! The pending question, and the choices Claude offered with it.
//!
//! `LiveSessionSummary::waiting_for` is a short verb phrase — CC writes
//! `"approve Bash"` or `"input needed"` into the PID file. That is
//! enough for a status dot and nothing else. The question itself, and
//! the options attached to it, exist only in the transcript.
//!
//! ## What this reads, and what it deliberately does not
//!
//! Exactly one shape: an unanswered `AskUserQuestion` tool call. Its
//! input is `{questions: [{question, header, multiSelect, options:
//! [{label, description, preview?}]}]}`, verified against real
//! transcripts on this machine (CC 2.1.x, 2026-08). A tool call with no
//! matching `tool_result` is one the session is still blocked on.
//!
//! **Permission prompts are not read**, and offering chips for one
//! would be actively wrong. A peer message cannot grant an escalation:
//! CC attaches a standing caveat telling the session never to treat a
//! peer message as the user's approval, and calls doing so *permission
//! laundering*. A session asked to approve its own pending prompt is
//! expected to refuse, and that refusal is correct. The client shows the
//! verb phrase and says to answer at the machine.
//!
//! ## What a tap actually does
//!
//! It sends the option's label down the ordinary peer channel as a
//! prompt. That is a **handoff**, not an answer: Claude Code may hold it
//! for the local user's approval, and whether an arriving message can
//! resolve a tool call the session is blocked on is **not established**
//! — it has not been measured against a live session. So the wording
//! after a tap is "handed off", never "answered", and the question stays
//! on the card until the transcript says otherwise. See the
//! `pending AskUserQuestion shape` row in
//! `crates/xtask/cc-upstream-watch.md`.

use std::path::Path;

use serde::Serialize;

use crate::session::SessionEvent;
use crate::session_live::redact::redact_secrets;

/// CC's tool name for a structured question. The pin: anything else is
/// not this shape, and guessing at a renamed tool would put invented
/// choices on a card.
pub const ASK_TOOL: &str = "AskUserQuestion";

/// Bytes read from the end of the transcript. A pending question is by
/// construction the last thing in the file; its payload is a few KB even
/// with option previews.
const TAIL_BYTES: u64 = 128 * 1024;

const QUESTION_CHARS: usize = 400;
const LABEL_CHARS: usize = 60;
const DESCRIPTION_CHARS: usize = 240;

/// How many options a card will carry. More than this and the phone is
/// the wrong place to answer.
const MAX_OPTIONS: usize = 6;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AskOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PendingAsk {
    /// The tool call this belongs to. Display-only; the send path
    /// addresses a session, never a tool.
    pub tool_use_id: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub multi_select: bool,
    pub options: Vec<AskOption>,
    /// Questions asked alongside this one. Non-zero means one tap
    /// cannot settle the ask, and the client says so.
    pub more_questions: usize,
    /// Options dropped because the card cannot carry them.
    pub options_omitted: usize,
}

/// The open question in this transcript, if there is one.
pub fn pending(file_path: &Path) -> Option<PendingAsk> {
    let events = super::transcript::tail_events(file_path, TAIL_BYTES)?;
    from_events(&events)
}

/// Pure half: the last unanswered `AskUserQuestion` in an event slice.
///
/// Split out so the shape rules are testable without a file, which is
/// what makes them cheap to re-check when CC moves.
pub fn from_events(events: &[SessionEvent]) -> Option<PendingAsk> {
    let answered: std::collections::HashSet<&str> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::UserToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();

    events.iter().rev().find_map(|e| match e {
        SessionEvent::AssistantToolUse {
            tool_name,
            tool_use_id,
            input_full,
            ..
        } if tool_name == ASK_TOOL && !answered.contains(tool_use_id.as_str()) => {
            parse_input(input_full, tool_use_id)
        }
        _ => None,
    })
}

fn parse_input(input_full: &str, tool_use_id: &str) -> Option<PendingAsk> {
    let v: serde_json::Value = serde_json::from_str(input_full).ok()?;
    let questions = v.get("questions")?.as_array()?;
    let first = questions.first()?;

    let question = first.get("question")?.as_str()?.trim();
    if question.is_empty() {
        return None;
    }

    let raw_options = first
        .get("options")
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default();

    let options: Vec<AskOption> = raw_options
        .iter()
        .filter_map(|o| {
            let label = o.get("label")?.as_str()?.trim();
            if label.is_empty() {
                return None;
            }
            Some(AskOption {
                label: super::truncate(&redact_secrets(label), LABEL_CHARS),
                // `preview` is deliberately not carried. It is ASCII art
                // sized for a desktop terminal and would wrap into
                // nonsense on a phone.
                description: o
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(|d| super::truncate(&redact_secrets(d), DESCRIPTION_CHARS))
                    .filter(|d| !d.is_empty()),
            })
        })
        .collect();

    // A question with no options is a free-text ask. The composer is
    // already the answer to that; chips would be an empty row.
    if options.is_empty() {
        return None;
    }

    let options_omitted = options.len().saturating_sub(MAX_OPTIONS);
    Some(PendingAsk {
        tool_use_id: tool_use_id.to_string(),
        question: super::truncate(&redact_secrets(question), QUESTION_CHARS),
        header: first
            .get("header")
            .and_then(|h| h.as_str())
            .map(|h| super::truncate(&redact_secrets(h), LABEL_CHARS))
            .filter(|h| !h.is_empty()),
        multi_select: first
            .get("multiSelect")
            .and_then(|m| m.as_bool())
            .unwrap_or(false),
        options: options.into_iter().take(MAX_OPTIONS).collect(),
        more_questions: questions.len().saturating_sub(1),
        options_omitted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_use(id: &str, input: serde_json::Value) -> SessionEvent {
        SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: ASK_TOOL.to_string(),
            tool_use_id: id.to_string(),
            input_preview: String::new(),
            input_full: input.to_string(),
        }
    }

    fn result(id: &str) -> SessionEvent {
        SessionEvent::UserToolResult {
            ts: None,
            uuid: None,
            tool_use_id: id.to_string(),
            content: "Your questions have been answered".into(),
            is_error: false,
        }
    }

    /// The shape measured on real transcripts written by CC 2.1.x.
    fn real_shape() -> serde_json::Value {
        serde_json::json!({
            "questions": [{
                "question": "Which artifacts should I remove?",
                "header": "Scope",
                "multiSelect": false,
                "options": [
                    { "label": "Plugin + marketplace", "description": "Disable the plugin and remove the marketplace." },
                    { "label": "Plugin only", "description": "Keep the marketplace registered." }
                ]
            }]
        })
    }

    #[test]
    fn an_unanswered_ask_yields_its_question_and_choices() {
        let got = from_events(&[ask_use("t1", real_shape())]).unwrap();
        assert_eq!(got.question, "Which artifacts should I remove?");
        assert_eq!(got.header.as_deref(), Some("Scope"));
        assert!(!got.multi_select);
        assert_eq!(got.options.len(), 2);
        assert_eq!(got.options[0].label, "Plugin + marketplace");
        assert_eq!(got.more_questions, 0);
    }

    #[test]
    fn an_answered_ask_is_not_pending() {
        let got = from_events(&[ask_use("t1", real_shape()), result("t1")]);
        assert!(
            got.is_none(),
            "a resolved question must not stay on the card"
        );
    }

    #[test]
    fn the_most_recent_unanswered_ask_wins() {
        let second = serde_json::json!({
            "questions": [{ "question": "and this one?", "options": [{ "label": "yes" }] }]
        });
        let got = from_events(&[
            ask_use("t1", real_shape()),
            result("t1"),
            ask_use("t2", second),
        ])
        .unwrap();
        assert_eq!(got.tool_use_id, "t2");
    }

    #[test]
    fn a_multi_question_ask_reports_what_one_tap_cannot_settle() {
        let two = serde_json::json!({
            "questions": [
                { "question": "first?", "options": [{ "label": "a" }] },
                { "question": "second?", "options": [{ "label": "b" }] }
            ]
        });
        let got = from_events(&[ask_use("t1", two)]).unwrap();
        assert_eq!(got.question, "first?");
        assert_eq!(got.more_questions, 1);
    }

    #[test]
    fn a_free_text_question_offers_no_chips() {
        // No options means the composer already is the answer; an empty
        // chip row would be a control that does nothing.
        let free = serde_json::json!({ "questions": [{ "question": "what next?" }] });
        assert!(from_events(&[ask_use("t1", free)]).is_none());
    }

    #[test]
    fn a_permission_prompt_is_not_treated_as_an_ask() {
        // A peer message cannot grant an escalation, so chips here would
        // offer an action that is correctly refused.
        let bash = SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: "Bash".into(),
            tool_use_id: "t1".into(),
            input_preview: "rm -rf /".into(),
            input_full: "{}".into(),
        };
        assert!(from_events(&[bash]).is_none());
    }

    #[test]
    fn a_malformed_input_yields_nothing_rather_than_a_guess() {
        let e = SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: ASK_TOOL.into(),
            tool_use_id: "t1".into(),
            input_preview: String::new(),
            input_full: "not json".into(),
        };
        assert!(from_events(&[e]).is_none());

        let wrong = ask_use("t2", serde_json::json!({ "prompts": [] }));
        assert!(from_events(&[wrong]).is_none());
    }

    #[test]
    fn secrets_in_a_question_are_masked() {
        let leaky = serde_json::json!({
            "questions": [{
                "question": "use sk-ant-oat01-LEAKED or the other one?",
                "options": [{ "label": "sk-ant-oat01-LEAKED", "description": "the leaked one" }]
            }]
        });
        let got = from_events(&[ask_use("t1", leaky)]).unwrap();
        let blob = serde_json::to_string(&got).unwrap();
        assert!(!blob.contains("LEAKED"), "{blob}");
    }

    #[test]
    fn too_many_options_are_capped_and_the_omission_is_reported() {
        let many: Vec<serde_json::Value> = (0..10)
            .map(|i| serde_json::json!({ "label": format!("opt {i}") }))
            .collect();
        let v = serde_json::json!({ "questions": [{ "question": "pick", "options": many }] });
        let got = from_events(&[ask_use("t1", v)]).unwrap();
        assert_eq!(got.options.len(), MAX_OPTIONS);
        assert_eq!(got.options_omitted, 10 - MAX_OPTIONS);
    }

    #[test]
    fn an_option_preview_is_never_carried() {
        // CC's `preview` is terminal ASCII art. On a phone it wraps into
        // noise, and it can be kilobytes.
        let v = serde_json::json!({
            "questions": [{
                "question": "layout?",
                "options": [{ "label": "left", "preview": "┌───┐\n│ x │\n└───┘" }]
            }]
        });
        let got = from_events(&[ask_use("t1", v)]).unwrap();
        let blob = serde_json::to_string(&got).unwrap();
        assert!(!blob.contains('┌'), "{blob}");
    }

    #[test]
    fn the_tail_reader_finds_an_ask_at_the_end_of_a_long_file() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        for i in 0..500 {
            writeln!(
                f,
                "{}",
                serde_json::json!({
                    "type": "assistant",
                    "message": { "role": "assistant", "content": [{ "type": "text", "text": format!("filler {i}") }] }
                })
            )
            .unwrap();
        }
        writeln!(
            f,
            "{}",
            serde_json::json!({
                "type": "assistant",
                "message": { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "t9", "name": ASK_TOOL, "input": real_shape() }
                ]}
            })
        )
        .unwrap();
        drop(f);

        let got = pending(&p).unwrap();
        assert_eq!(got.tool_use_id, "t9");
        assert_eq!(got.options.len(), 2);
    }
}
