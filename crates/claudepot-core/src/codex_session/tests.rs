//! Unit tests for the Codex rollout parser.

use std::path::PathBuf;

use super::error::CodexError;
use super::parser::{iter_events, parse_codex_rollout_jsonl, parse_head};
use super::types::{CodexEvent, EnvironmentTextKind};

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("src");
    p.push("codex_session");
    p.push("testdata");
    p.push(name);
    p
}

// ─── parse_head ───────────────────────────────────────────────

#[test]
fn parse_head_single_turn() {
    let h = parse_head(&fixture("single_turn.jsonl")).expect("ok");
    assert_eq!(h.session_id, "01-abc");
    assert_eq!(h.cwd.as_deref().map(|p| p.to_str().unwrap()), Some("/Users/jane/proj"));
    assert_eq!(h.originator.as_deref(), Some("codex_cli"));
    assert_eq!(h.cli_version.as_deref(), Some("0.44.0"));
    assert_eq!(h.approval_policy.as_deref(), Some("on-request"));
    assert_eq!(h.sandbox_mode.as_deref(), Some("workspace-write"));
    assert!(h.started_at.is_some());
}

#[test]
fn parse_head_missing_meta_errors() {
    let err = parse_head(&fixture("missing_session_meta.jsonl"))
        .expect_err("should fail");
    assert!(
        matches!(err, CodexError::MissingSessionMeta { .. }),
        "expected MissingSessionMeta, got {err:?}"
    );
}

#[test]
fn parse_head_tolerates_malformed_lines() {
    // The fixture has malformed lines before the session_meta is
    // also absent — but a malformed line between meta and turn_context
    // should still produce a head. The malformed_lines fixture
    // does have session_meta first, so it should succeed.
    let h = parse_head(&fixture("malformed_lines.jsonl")).expect("ok");
    assert_eq!(h.session_id, "01-mal");
}

#[test]
fn parse_head_on_empty_errors() {
    let err = parse_head(&fixture("empty.jsonl")).expect_err("should fail");
    assert!(
        matches!(err, CodexError::MissingSessionMeta { .. }),
        "expected MissingSessionMeta, got {err:?}"
    );
}

// ─── iter_events ──────────────────────────────────────────────

#[test]
fn iter_events_single_turn_line_numbers() {
    let events: Vec<_> = iter_events(&fixture("single_turn.jsonl"))
        .expect("open")
        .collect();
    // 4 lines, 4 events
    assert_eq!(events.len(), 4);
    match &events[0] {
        CodexEvent::SessionMeta { line, .. } => assert_eq!(*line, 1),
        e => panic!("expected SessionMeta, got {e:?}"),
    }
    match &events[1] {
        CodexEvent::TurnContext { line, .. } => assert_eq!(*line, 2),
        e => panic!("expected TurnContext, got {e:?}"),
    }
    match &events[2] {
        CodexEvent::UserMessage { line, kind, .. } => {
            assert_eq!(*line, 3);
            assert!(matches!(kind, EnvironmentTextKind::UserPrompt));
        }
        e => panic!("expected UserMessage, got {e:?}"),
    }
    match &events[3] {
        CodexEvent::AssistantMessage { line, .. } => assert_eq!(*line, 4),
        e => panic!("expected AssistantMessage, got {e:?}"),
    }
}

#[test]
fn iter_events_skips_malformed() {
    let events: Vec<_> = iter_events(&fixture("malformed_lines.jsonl"))
        .expect("open")
        .collect();
    // 6 lines on disk, but 3 are malformed → 3 valid events
    // (session_meta, user message, assistant message). The
    // user-message event must have line=4 because lines 2 + 3
    // were malformed-but-counted.
    assert_eq!(events.len(), 3);
    match &events[1] {
        CodexEvent::UserMessage { line, .. } => assert_eq!(*line, 4),
        e => panic!("expected UserMessage at line 4, got {e:?}"),
    }
    match &events[2] {
        CodexEvent::AssistantMessage { line, .. } => assert_eq!(*line, 6),
        e => panic!("expected AssistantMessage at line 6, got {e:?}"),
    }
}

#[test]
fn iter_events_empty_file() {
    let events: Vec<_> = iter_events(&fixture("empty.jsonl"))
        .expect("open")
        .collect();
    assert!(events.is_empty());
}

#[test]
fn iter_events_unknown_types_become_other() {
    let events: Vec<_> = iter_events(&fixture("unknown_event_types.jsonl"))
        .expect("open")
        .collect();
    let other_tags: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            CodexEvent::Other { type_tag, .. } => Some(type_tag.as_str()),
            _ => None,
        })
        .collect();
    assert!(other_tags.contains(&"event_msg"));
    assert!(other_tags.contains(&"hypothetical_future_v2_event"));
}

// ─── parse_codex_rollout_jsonl ────────────────────────────────

#[test]
fn full_parse_single_turn_yields_one_exchange() {
    let conv = parse_codex_rollout_jsonl(&fixture("single_turn.jsonl"))
        .expect("ok");
    assert_eq!(conv.head.session_id, "01-abc");
    assert_eq!(conv.exchanges.len(), 1);
    let ex = &conv.exchanges[0];
    assert_eq!(ex.id, "01-abc:0");
    assert_eq!(ex.turn_index, 0);
    assert_eq!(ex.user_text, "audit this codebase");
    assert_eq!(ex.assistant_text, "found 3 issues");
    assert_eq!(ex.line_start, Some(3));
    assert_eq!(ex.line_end, Some(4));
    assert!(ex.tool_calls.is_empty());
}

#[test]
fn full_parse_two_turns_yields_two_exchanges_in_order() {
    let conv = parse_codex_rollout_jsonl(&fixture("two_turns.jsonl"))
        .expect("ok");
    assert_eq!(conv.exchanges.len(), 2);
    assert_eq!(conv.exchanges[0].id, "01-two:0");
    assert_eq!(conv.exchanges[1].id, "01-two:1");
    assert_eq!(conv.exchanges[0].user_text, "first question");
    assert_eq!(conv.exchanges[0].assistant_text, "first answer");
    assert_eq!(conv.exchanges[1].user_text, "second question");
    assert_eq!(conv.exchanges[1].assistant_text, "second answer");
}

#[test]
fn full_parse_tool_call_turn_links_call_and_output() {
    let conv = parse_codex_rollout_jsonl(&fixture("tool_call_turn.jsonl"))
        .expect("ok");
    // Two synthetic user messages (instructions + environment)
    // precede the real user prompt; only the real prompt should
    // open an exchange.
    assert_eq!(conv.exchanges.len(), 1);
    let ex = &conv.exchanges[0];
    assert_eq!(ex.user_text, "run the test suite");
    assert_eq!(ex.assistant_text, "tests passed");
    assert_eq!(ex.tool_calls.len(), 1);
    let tc = &ex.tool_calls[0];
    assert_eq!(tc.name, "shell");
    assert_eq!(tc.call_id, "call_abc");
    assert!(tc.output.is_some());
    assert!(!tc.is_error);
    assert_eq!(tc.call_line, 6);
    assert_eq!(tc.output_line, Some(7));
}

#[test]
fn full_parse_tool_error_flagged_on_nonzero_exit() {
    let raw = r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"01-err","cwd":"/x","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do thing"}]}}
{"timestamp":"2026-05-15T11:30:01.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}","call_id":"call_x"}}
{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_x","output":"{\"output\":\"err\",\"metadata\":{\"exit_code\":2}}"}}
{"timestamp":"2026-05-15T11:30:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"failed"}]}}
"#;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("err.jsonl");
    std::fs::write(&p, raw).unwrap();
    let conv = parse_codex_rollout_jsonl(&p).expect("ok");
    assert_eq!(conv.exchanges.len(), 1);
    let tc = &conv.exchanges[0].tool_calls[0];
    assert!(tc.is_error, "exit_code=2 should flag is_error=true");
}

#[test]
fn full_parse_missing_meta_errors() {
    let err = parse_codex_rollout_jsonl(&fixture("missing_session_meta.jsonl"))
        .expect_err("must error");
    assert!(
        matches!(err, CodexError::MissingSessionMeta { .. }),
        "expected MissingSessionMeta, got {err:?}"
    );
}

#[test]
fn full_parse_unknown_types_dont_break_exchanges() {
    let conv = parse_codex_rollout_jsonl(&fixture("unknown_event_types.jsonl"))
        .expect("ok");
    // session_meta sets the head, event_msg + hypothetical event
    // both become Other and are ignored, the real user/assistant
    // pair produces one exchange.
    assert_eq!(conv.exchanges.len(), 1);
    assert_eq!(conv.exchanges[0].user_text, "hello v2");
    assert_eq!(conv.exchanges[0].assistant_text, "hi");
}

#[test]
fn full_parse_classifies_synthetic_user_messages() {
    use super::parser::iter_events;
    let kinds: Vec<EnvironmentTextKind> = iter_events(&fixture("tool_call_turn.jsonl"))
        .expect("open")
        .filter_map(|e| match e {
            CodexEvent::UserMessage { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            EnvironmentTextKind::Instructions,
            EnvironmentTextKind::Environment,
            EnvironmentTextKind::UserPrompt,
        ]
    );
}

#[test]
fn diagnostics_count_malformed_lines() {
    let conv = parse_codex_rollout_jsonl(&fixture("malformed_lines.jsonl"))
        .expect("ok");
    // Fixture has session_meta, then non-JSON, then truncated-JSON,
    // then valid user message, then a JSON object lacking `type`,
    // then valid assistant — 3 malformed lines total.
    assert!(
        conv.diagnostics.malformed_lines >= 3,
        "expected ≥ 3 malformed lines, got {}",
        conv.diagnostics.malformed_lines
    );
    assert!(!conv.diagnostics.truncated_by_io);
    assert_eq!(conv.diagnostics.oversize_lines, 0);
}

#[test]
fn diagnostics_flag_oversize_lines() {
    // Build a fixture with one valid session_meta line, one
    // adversarial line larger than MAX_LINE_BYTES, then one valid
    // exchange. The oversize line should be dropped with
    // diagnostics.oversize_lines == 1.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("oversize.jsonl");
    let huge = "x".repeat(super::parser::MAX_LINE_BYTES + 10);
    let body = format!(
        "{{\"timestamp\":\"2026-05-15T11:30:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"01-big\",\"cwd\":\"/x\",\"originator\":\"codex_cli\",\"cli_version\":\"0.44.0\"}}}}\n{huge}\n{{\"timestamp\":\"2026-05-15T11:30:00.200Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"hello\"}}]}}}}\n{{\"timestamp\":\"2026-05-15T11:30:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"hi\"}}]}}}}\n",
    );
    std::fs::write(&p, body).unwrap();

    let conv = parse_codex_rollout_jsonl(&p).expect("ok");
    assert!(
        conv.diagnostics.oversize_lines >= 1,
        "oversize line should be counted, got {}",
        conv.diagnostics.oversize_lines
    );
    // The valid user/assistant pair after the oversized line
    // should still produce an exchange.
    assert!(
        !conv.exchanges.is_empty(),
        "parser should keep going past oversized line"
    );
}

#[test]
fn ide_context_opens_an_exchange_for_vscode_originator() {
    // codex_vscode wraps the real user prompt inside a
    // `# Context from ... ## My request for Codex: ...` block.
    // The parser must treat that as a turn seed, otherwise
    // VSCode-originated rollouts produce zero exchanges.
    let raw = "{\"timestamp\":\"2026-05-15T11:30:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"01-vsc\",\"cwd\":\"/x\",\"originator\":\"codex_vscode\",\"cli_version\":\"0.44.0\"}}\n{\"timestamp\":\"2026-05-15T11:30:00.200Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# Context from my IDE setup:\\n## Active file: foo.py\\n## My request for Codex:\\ninspect foo.py\"}]}}\n{\"timestamp\":\"2026-05-15T11:30:02.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}}\n";
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("vsc.jsonl");
    std::fs::write(&p, raw).unwrap();
    let conv = parse_codex_rollout_jsonl(&p).expect("ok");
    assert_eq!(conv.exchanges.len(), 1, "IdeContext must open an exchange");
    assert!(conv.exchanges[0].user_text.contains("inspect foo.py"));
    assert_eq!(conv.exchanges[0].assistant_text, "ok");
}

#[test]
fn exchange_ids_stable_across_reparse() {
    let p = fixture("two_turns.jsonl");
    let a = parse_codex_rollout_jsonl(&p).expect("ok");
    let b = parse_codex_rollout_jsonl(&p).expect("ok");
    let ids_a: Vec<&str> = a.exchanges.iter().map(|e| e.id.as_str()).collect();
    let ids_b: Vec<&str> = b.exchanges.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids_a, ids_b);
    assert_eq!(ids_a, vec!["01-two:0", "01-two:1"]);
}
