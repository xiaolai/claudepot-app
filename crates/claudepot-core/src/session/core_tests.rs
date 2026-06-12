//! Inline test module for `session.rs`. Lives in this sibling file
//! so `session.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "session_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use std::io::Write;
use tempfile::TempDir;

fn write_session(dir: &Path, slug: &str, session_id: &str, lines: &[&str]) -> PathBuf {
    let slug_dir = dir.join("projects").join(slug);
    fs::create_dir_all(&slug_dir).unwrap();
    let path = slug_dir.join(format!("{session_id}.jsonl"));
    let mut f = fs::File::create(&path).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    path
}

#[test]
fn empty_projects_dir_is_ok() {
    let tmp = TempDir::new().unwrap();
    let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn single_session_scan_captures_everything() {
    let tmp = TempDir::new().unwrap();
    let user1 = r#"{"type":"user","message":{"role":"user","content":"Fix the build"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/repo/foo","gitBranch":"main","version":"2.1.97","sessionId":"AAA","slug":"brave-otter"}"#;
    let asst1 = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"OK"}],"usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":200}},"timestamp":"2026-04-10T10:00:05Z","cwd":"/repo/foo","gitBranch":"main","version":"2.1.97","sessionId":"AAA"}"#;
    let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"done","is_error":false}]},"timestamp":"2026-04-10T10:00:10Z","cwd":"/repo/foo","sessionId":"AAA"}"#;

    write_session(tmp.path(), "-repo-foo", "AAA", &[user1, asst1, tool]);

    let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.session_id, "AAA");
    assert_eq!(r.slug, "-repo-foo");
    assert_eq!(r.project_path, "/repo/foo");
    assert!(r.project_from_transcript);
    assert_eq!(r.event_count, 3);
    assert_eq!(r.message_count, 3);
    assert_eq!(r.user_message_count, 2);
    assert_eq!(r.assistant_message_count, 1);
    assert_eq!(r.first_user_prompt.as_deref(), Some("Fix the build"));
    assert_eq!(r.models, vec!["claude-opus-4-7".to_string()]);
    assert_eq!(r.tokens.input, 100);
    assert_eq!(r.tokens.output, 50);
    assert_eq!(r.tokens.cache_creation, 10);
    assert_eq!(r.tokens.cache_read, 200);
    assert_eq!(r.git_branch.as_deref(), Some("main"));
    assert_eq!(r.cc_version.as_deref(), Some("2.1.97"));
    assert_eq!(r.display_slug.as_deref(), Some("brave-otter"));
    assert!(!r.has_error);
}

#[test]
fn turn_record_user_prompt_carry_over_clears_when_intervening_user_line_has_no_text() {
    // Regression: a user line without extractable text (tool-result-only,
    // image-only, etc.) used to leave the per-turn carry-over pointing at
    // the previous text prompt, so the next assistant turn would render
    // someone else's prompt in the top-costly-prompts panel. The carry
    // must clear.
    let tmp = TempDir::new().unwrap();
    let user_text = r#"{"type":"user","message":{"role":"user","content":"explain it"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/p","sessionId":"S1"}"#;
    let asst_first = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"a1"}],"usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-04-10T10:00:01Z","cwd":"/p","sessionId":"S1"}"#;
    // Tool-result-only user line — no extractable text.
    let user_tool_only = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]},"timestamp":"2026-04-10T10:00:02Z","cwd":"/p","sessionId":"S1"}"#;
    let asst_second = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"a2"}],"usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-04-10T10:00:03Z","cwd":"/p","sessionId":"S1"}"#;

    let path = write_session(
        tmp.path(),
        "-p",
        "S1",
        &[user_text, asst_first, user_tool_only, asst_second],
    );
    let scan = scan_session("-p", &path).unwrap();
    assert_eq!(scan.turns.len(), 2);
    assert_eq!(
        scan.turns[0].user_prompt_preview.as_deref(),
        Some("explain it")
    );
    // Crucial: turn 1's preview is None (the tool-result-only user line
    // cleared the carry), not "explain it" carried over from turn 0.
    assert_eq!(
        scan.turns[1].user_prompt_preview, None,
        "tool-result-only user line must clear the carry, not leave the prior text in place"
    );
}

#[test]
fn first_user_prompt_skips_tool_result_and_caveat() {
    let tmp = TempDir::new().unwrap();
    let caveat = r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>ignore</local-command-caveat>"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/a","sessionId":"S1"}"#;
    let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x","is_error":false}]},"timestamp":"2026-04-10T10:00:01Z","cwd":"/a","sessionId":"S1"}"#;
    let real = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"the real question"}]},"timestamp":"2026-04-10T10:00:02Z","cwd":"/a","sessionId":"S1"}"#;
    write_session(tmp.path(), "-a", "S1", &[caveat, tool, real]);
    let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
    assert_eq!(
        rows[0].first_user_prompt.as_deref(),
        Some("the real question")
    );
}

#[test]
fn malformed_line_does_not_poison_scan() {
    let tmp = TempDir::new().unwrap();
    let bad = "{not valid json";
    let good = r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/z","sessionId":"S1"}"#;
    write_session(tmp.path(), "-z", "S1", &[bad, good]);
    let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
    assert_eq!(rows.len(), 1);
    // event_count counts ALL non-empty lines, including malformed.
    assert_eq!(rows[0].event_count, 2);
    assert_eq!(rows[0].user_message_count, 1);
    assert_eq!(rows[0].first_user_prompt.as_deref(), Some("hi"));
}

#[test]
fn sort_newest_first() {
    let tmp = TempDir::new().unwrap();
    let older = r#"{"type":"user","message":{"role":"user","content":"old"},"timestamp":"2026-04-01T00:00:00Z","cwd":"/a","sessionId":"A"}"#;
    let newer = r#"{"type":"user","message":{"role":"user","content":"new"},"timestamp":"2026-04-20T00:00:00Z","cwd":"/b","sessionId":"B"}"#;
    write_session(tmp.path(), "-a", "A", &[older]);
    write_session(tmp.path(), "-b", "B", &[newer]);
    let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].session_id, "B");
    assert_eq!(rows[1].session_id, "A");
}

#[test]
fn read_session_detail_parses_event_kinds() {
    let tmp = TempDir::new().unwrap();
    let user = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"}]},"timestamp":"2026-04-10T10:00:00Z","cwd":"/r","sessionId":"D1","uuid":"u1"}"#;
    let asst = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"hi back"},{"type":"tool_use","id":"t1","name":"Bash","input":{"cmd":"ls"}}],"usage":{"input_tokens":1,"output_tokens":2}},"timestamp":"2026-04-10T10:00:01Z","cwd":"/r","sessionId":"D1","uuid":"u2"}"#;
    let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a\nb","is_error":false}]},"timestamp":"2026-04-10T10:00:02Z","cwd":"/r","sessionId":"D1","uuid":"u3"}"#;
    let summary = r#"{"type":"summary","summary":"compacted","timestamp":"2026-04-10T10:00:03Z","uuid":"u4"}"#;
    write_session(tmp.path(), "-r", "D1", &[user, asst, tool, summary]);

    let detail = read_session_detail(tmp.path(), "D1").unwrap();
    assert_eq!(detail.row.session_id, "D1");
    assert_eq!(detail.events.len(), 5);
    match &detail.events[0] {
        SessionEvent::UserText { text, .. } => assert_eq!(text, "hello"),
        e => panic!("expected UserText, got {e:?}"),
    }
    match &detail.events[1] {
        SessionEvent::AssistantText { text, .. } => assert_eq!(text, "hi back"),
        e => panic!("expected AssistantText, got {e:?}"),
    }
    match &detail.events[2] {
        SessionEvent::AssistantToolUse { tool_name, .. } => assert_eq!(tool_name, "Bash"),
        e => panic!("expected AssistantToolUse, got {e:?}"),
    }
    match &detail.events[3] {
        SessionEvent::UserToolResult { content, .. } => assert_eq!(content, "a\nb"),
        e => panic!("expected UserToolResult, got {e:?}"),
    }
    match &detail.events[4] {
        SessionEvent::Summary { text, .. } => assert_eq!(text, "compacted"),
        e => panic!("expected Summary, got {e:?}"),
    }
}

#[test]
fn read_session_detail_at_path_rejects_outside_projects() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("projects")).unwrap();
    let outside = tmp.path().join("rogue.jsonl");
    fs::write(&outside, "{}\n").unwrap();
    assert!(matches!(
        read_session_detail_at_path(tmp.path(), &outside),
        Err(SessionError::InvalidPath(_))
    ));
}

#[test]
fn read_session_detail_at_path_rejects_non_jsonl() {
    let tmp = TempDir::new().unwrap();
    let slug_dir = tmp.path().join("projects").join("-repo");
    fs::create_dir_all(&slug_dir).unwrap();
    let wrong = slug_dir.join("notes.md");
    fs::write(&wrong, "hi\n").unwrap();
    assert!(matches!(
        read_session_detail_at_path(tmp.path(), &wrong),
        Err(SessionError::InvalidPath(_))
    ));
}

#[test]
fn read_session_detail_at_path_reads_the_targeted_file_among_dupes() {
    let tmp = TempDir::new().unwrap();
    let a_line = r#"{"type":"user","message":{"role":"user","content":"from A"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/a","sessionId":"DUP"}"#;
    let b_line = r#"{"type":"user","message":{"role":"user","content":"from B"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/b","sessionId":"DUP"}"#;
    let a_path = write_session(tmp.path(), "-a", "DUP", &[a_line]);
    let b_path = write_session(tmp.path(), "-b", "DUP", &[b_line]);

    let read_a = read_session_detail_at_path(tmp.path(), &a_path).unwrap();
    let read_b = read_session_detail_at_path(tmp.path(), &b_path).unwrap();
    assert_eq!(read_a.row.project_path, "/a");
    assert_eq!(read_b.row.project_path, "/b");
    assert_eq!(read_a.row.slug, "-a");
    assert_eq!(read_b.row.slug, "-b");
}

#[test]
fn locate_session_rejects_traversal() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("projects")).unwrap();
    assert!(matches!(
        read_session_detail(tmp.path(), "../../etc/passwd"),
        Err(SessionError::InvalidPath(_))
    ));
}

#[test]
fn read_session_detail_not_found() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("projects")).unwrap();
    assert!(matches!(
        read_session_detail(tmp.path(), "missing"),
        Err(SessionError::NotFound(_))
    ));
}

#[test]
fn fallback_project_path_from_slug_when_cwd_missing() {
    let tmp = TempDir::new().unwrap();
    let asst = r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":[{"type":"text","text":"x"}]},"timestamp":"2026-04-10T10:00:00Z","sessionId":"S"}"#;
    write_session(tmp.path(), "-Users-joker-repo", "S", &[asst]);
    let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].project_from_transcript);
    // unsanitize_path turns "-Users-joker-repo" back into an absolute path
    assert!(rows[0].project_path.contains("Users") && rows[0].project_path.contains("joker"));
}
