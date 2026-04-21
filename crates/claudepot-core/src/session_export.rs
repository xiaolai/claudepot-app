//! Render a `SessionDetail` into Markdown or JSON for archiving /
//! sharing. Inspired by claude-devtools' `sessionExporter`.
//!
//! The markdown output preserves logical turn boundaries and never
//! leaks credentials — before serializing we run every string through
//! [`redact_secrets`](#fn.redact_secrets) to truncate any `sk-ant-`
//! token the transcript happens to echo (these shouldn't be there, but
//! a shared transcript is exactly where a leak would hurt).
//!
//! JSON export is a straight serde serialization of the public session
//! types — it's lossless by design so downstream tooling can re-parse
//! with the same Rust structs we ship elsewhere.

use crate::session::SessionDetail;
use crate::session::SessionEvent;
use crate::session_chunks::{build_chunks, SessionChunk};
use serde::Serialize;
use std::fmt::Write as _;

/// Format hint forwarded from CLI / Tauri invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Json,
}

/// Top-level entry point.
///
/// Returns the rendered body as a string. Callers decide where to put
/// it (stdout, file, clipboard).
pub fn export(detail: &SessionDetail, format: ExportFormat) -> String {
    match format {
        ExportFormat::Markdown => export_markdown(detail),
        ExportFormat::Json => export_json(detail),
    }
}

/// Markdown renderer. Reconstructs user/assistant bubbles, tool call
/// blocks (collapsible), thinking blocks, and compaction dividers.
///
/// The output is GitHub-flavored Markdown with a `---` frontmatter-ish
/// metadata block at the top so the transcript is self-describing when
/// pasted anywhere.
pub fn export_markdown(detail: &SessionDetail) -> String {
    let mut s = String::with_capacity(8 * 1024);
    let row = &detail.row;

    // Header — the same fields the GUI header strip surfaces.
    let _ = writeln!(s, "# Session `{}`", row.session_id);
    let _ = writeln!(s);
    let _ = writeln!(s, "- **Project:** `{}`", row.project_path);
    if let Some(branch) = &row.git_branch {
        let _ = writeln!(s, "- **Branch:** `{branch}`");
    }
    if let Some(ver) = &row.cc_version {
        let _ = writeln!(s, "- **CC version:** `{ver}`");
    }
    if !row.models.is_empty() {
        let _ = writeln!(s, "- **Models:** {}", row.models.join(", "));
    }
    if let Some(first) = row.first_ts {
        let _ = writeln!(s, "- **Started:** {}", first.to_rfc3339());
    }
    if let Some(last) = row.last_ts {
        let _ = writeln!(s, "- **Last event:** {}", last.to_rfc3339());
    }
    let _ = writeln!(
        s,
        "- **Turns:** {} (user {}, assistant {})",
        row.message_count, row.user_message_count, row.assistant_message_count
    );
    let _ = writeln!(
        s,
        "- **Tokens:** {} input, {} output, {} cache read, {} cache create",
        row.tokens.input, row.tokens.output, row.tokens.cache_read, row.tokens.cache_creation
    );
    let _ = writeln!(s);
    let _ = writeln!(s, "---");
    let _ = writeln!(s);

    let chunks = build_chunks(&detail.events);
    if chunks.is_empty() {
        s.push_str("_(empty transcript)_\n");
        return s;
    }
    for chunk in chunks {
        render_chunk(&mut s, &chunk, &detail.events);
    }
    s
}

/// Strict JSON export — redacts secrets in string fields on a clone so
/// the on-disk row remains untouched.
pub fn export_json(detail: &SessionDetail) -> String {
    let mut cloned = detail.clone();
    redact_in_place(&mut cloned);
    let wrapped = ExportJson { detail: &cloned };
    serde_json::to_string_pretty(&wrapped).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Serialize)]
struct ExportJson<'a> {
    detail: &'a SessionDetail,
}

// ---------------------------------------------------------------------------
// Markdown helpers
// ---------------------------------------------------------------------------

fn render_chunk(s: &mut String, chunk: &SessionChunk, events: &[SessionEvent]) {
    match chunk {
        SessionChunk::User { event_index, .. } => {
            if let Some(SessionEvent::UserText { text, ts, .. }) = events.get(*event_index) {
                let _ = writeln!(
                    s,
                    "### 👤 User{}",
                    ts.map(|t| format!(" — {}", t.to_rfc3339())).unwrap_or_default()
                );
                let _ = writeln!(s);
                let _ = writeln!(s, "{}", redact_secrets(text.trim()));
                let _ = writeln!(s);
            }
        }
        SessionChunk::System { event_index, .. } => {
            if let Some(SessionEvent::UserText { text, .. }) = events.get(*event_index) {
                let _ = writeln!(s, "### ⎘ System output");
                let _ = writeln!(s);
                let _ = writeln!(s, "```");
                let _ = writeln!(s, "{}", redact_secrets(text.trim()));
                let _ = writeln!(s, "```");
                let _ = writeln!(s);
            }
        }
        SessionChunk::Compact { event_index, .. } => {
            if let Some(SessionEvent::Summary { text, .. }) = events.get(*event_index) {
                let _ = writeln!(s, "---");
                let _ = writeln!(s);
                let _ = writeln!(s, "### ⏵⏵ Compacted");
                let _ = writeln!(s);
                let _ = writeln!(s, "> {}", redact_secrets(text.trim()));
                let _ = writeln!(s);
                let _ = writeln!(s, "---");
                let _ = writeln!(s);
            }
        }
        SessionChunk::Ai {
            event_indices,
            tool_executions,
            ..
        } => {
            let _ = writeln!(s, "### 🤖 Assistant");
            let _ = writeln!(s);
            // Skip tool results whose linked call is in this chunk —
            // those render inside the call block.
            let absorbed_results: std::collections::HashSet<usize> = tool_executions
                .iter()
                .filter_map(|t| t.result_index)
                .collect();
            for &idx in event_indices {
                if absorbed_results.contains(&idx) {
                    continue;
                }
                let Some(ev) = events.get(idx) else { continue };
                render_ai_event(s, ev, tool_executions, idx);
            }
            let _ = writeln!(s);
        }
    }
}

fn render_ai_event(
    s: &mut String,
    ev: &SessionEvent,
    tools: &[crate::session_tool_link::LinkedTool],
    idx: usize,
) {
    match ev {
        SessionEvent::AssistantText { text, .. } => {
            let _ = writeln!(s, "{}", redact_secrets(text.trim()));
            let _ = writeln!(s);
        }
        SessionEvent::AssistantThinking { text, .. } => {
            let _ = writeln!(s, "<details><summary>💭 Thinking</summary>");
            let _ = writeln!(s);
            let _ = writeln!(s, "{}", redact_secrets(text.trim()));
            let _ = writeln!(s);
            let _ = writeln!(s, "</details>");
            let _ = writeln!(s);
        }
        SessionEvent::AssistantToolUse {
            tool_name,
            tool_use_id,
            input_preview,
            ..
        } => {
            // Look up matching linked tool.
            let linked = tools.iter().find(|t| t.call_index == idx);
            let error_badge = linked
                .map(|lt| if lt.is_error { " · ERROR" } else { "" })
                .unwrap_or("");
            let _ = writeln!(
                s,
                "<details><summary>🔧 {tool_name} <code>{id}</code>{error_badge}</summary>",
                tool_name = tool_name,
                id = short_id(tool_use_id),
            );
            let _ = writeln!(s);
            let _ = writeln!(s, "**Input**");
            let _ = writeln!(s);
            let _ = writeln!(s, "```json");
            let _ = writeln!(s, "{}", redact_secrets(input_preview));
            let _ = writeln!(s, "```");
            if let Some(lt) = linked {
                if let Some(result) = &lt.result_content {
                    let _ = writeln!(s);
                    let _ = writeln!(s, "**Result**");
                    let _ = writeln!(s);
                    let _ = writeln!(s, "```");
                    let _ = writeln!(s, "{}", redact_secrets(result));
                    let _ = writeln!(s, "```");
                }
                if let Some(d) = lt.duration_ms {
                    let _ = writeln!(s);
                    let _ = writeln!(s, "_Duration: {d} ms_");
                }
            }
            let _ = writeln!(s);
            let _ = writeln!(s, "</details>");
            let _ = writeln!(s);
        }
        _ => {}
    }
}

fn short_id(s: &str) -> String {
    if s.len() > 8 {
        s[..8].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Secret redaction
// ---------------------------------------------------------------------------

/// Truncate any `sk-ant-...` token found inside `text`. The mask keeps
/// the prefix and last four characters so readers can still tell the
/// tokens apart without being able to reuse them.
pub fn redact_secrets(text: &str) -> String {
    // Work on bytes so we can scan linearly. The token set we guard
    // against: `sk-ant-` followed by a base64url-ish run.
    let needle = "sk-ant-";
    if !text.contains(needle) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    while cursor < bytes.len() {
        if let Some(start) = find_from(bytes, cursor, needle.as_bytes()) {
            out.push_str(&text[cursor..start]);
            let tok_end = token_end(bytes, start);
            let token = &text[start..tok_end];
            out.push_str(&mask(token));
            cursor = tok_end;
        } else {
            out.push_str(&text[cursor..]);
            break;
        }
    }
    out
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from > hay.len().saturating_sub(needle.len()) {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn token_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_token_char(bytes[i]) {
        i += 1;
    }
    i
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

fn mask(token: &str) -> String {
    if token.len() <= 12 {
        return "sk-ant-***".to_string();
    }
    let last4 = &token[token.len() - 4..];
    format!("sk-ant-***{last4}")
}

fn redact_in_place(detail: &mut SessionDetail) {
    if let Some(p) = &mut detail.row.first_user_prompt {
        *p = redact_secrets(p);
    }
    for ev in &mut detail.events {
        match ev {
            SessionEvent::UserText { text, .. }
            | SessionEvent::AssistantText { text, .. }
            | SessionEvent::AssistantThinking { text, .. }
            | SessionEvent::Summary { text, .. } => {
                *text = redact_secrets(text);
            }
            SessionEvent::UserToolResult { content, .. } => {
                *content = redact_secrets(content);
            }
            SessionEvent::AssistantToolUse { input_preview, .. } => {
                *input_preview = redact_secrets(input_preview);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionEvent, SessionRow, TokenUsage};
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse::<DateTime<Utc>>().unwrap())
    }

    fn sample_detail() -> SessionDetail {
        let row = SessionRow {
            session_id: "sess-1".into(),
            slug: "-r".into(),
            file_path: PathBuf::from("/tmp/x.jsonl"),
            file_size_bytes: 100,
            last_modified: None,
            project_path: "/repo".into(),
            project_from_transcript: true,
            first_ts: ts("2026-04-10T10:00:00Z"),
            last_ts: ts("2026-04-10T10:00:05Z"),
            event_count: 3,
            message_count: 2,
            user_message_count: 1,
            assistant_message_count: 1,
            first_user_prompt: Some("debug".into()),
            models: vec!["claude-opus-4-7".into()],
            tokens: TokenUsage {
                input: 100,
                output: 50,
                ..TokenUsage::default()
            },
            git_branch: Some("main".into()),
            cc_version: Some("2.1.97".into()),
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        };
        let events = vec![
            SessionEvent::UserText {
                ts: ts("2026-04-10T10:00:00Z"),
                uuid: Some("u1".into()),
                text: "debug".into(),
            },
            SessionEvent::AssistantToolUse {
                ts: ts("2026-04-10T10:00:01Z"),
                uuid: Some("u2".into()),
                model: Some("claude-opus-4-7".into()),
                tool_name: "Bash".into(),
                tool_use_id: "toolu_abcd1234".into(),
                input_preview: r#"{"cmd":"ls"}"#.into(),
            },
            SessionEvent::UserToolResult {
                ts: ts("2026-04-10T10:00:02Z"),
                uuid: Some("u3".into()),
                tool_use_id: "toolu_abcd1234".into(),
                content: "one\ntwo".into(),
                is_error: false,
            },
            SessionEvent::AssistantText {
                ts: ts("2026-04-10T10:00:05Z"),
                uuid: Some("u4".into()),
                model: Some("claude-opus-4-7".into()),
                text: "found it".into(),
                usage: None,
                stop_reason: None,
            },
        ];
        SessionDetail { row, events }
    }

    #[test]
    fn markdown_export_has_header_and_user_turn() {
        let out = export_markdown(&sample_detail());
        assert!(out.contains("# Session `sess-1`"));
        assert!(out.contains("**Branch:** `main`"));
        assert!(out.contains("👤 User"));
        assert!(out.contains("debug"));
    }

    #[test]
    fn markdown_export_folds_tool_result_into_tool_call_block() {
        let out = export_markdown(&sample_detail());
        // The tool result line should NOT appear as its own section —
        // it belongs inside the Bash block.
        let occurrences = out.matches("one\ntwo").count();
        assert_eq!(occurrences, 1);
        assert!(out.contains("🔧 Bash"));
        assert!(out.contains("**Result**"));
    }

    #[test]
    fn markdown_export_emits_assistant_trailing_text() {
        let out = export_markdown(&sample_detail());
        assert!(out.contains("found it"));
    }

    #[test]
    fn json_export_round_trips() {
        let detail = sample_detail();
        let out = export_json(&detail);
        // serde_json parses back — that's the contract.
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            parsed["detail"]["row"]["session_id"],
            serde_json::json!("sess-1")
        );
        assert_eq!(parsed["detail"]["events"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn redact_secrets_masks_sk_ant_tokens() {
        let text = "key sk-ant-oat01-Abcdefghijkl1234 and another sk-ant-api03-XYZwxyz9876";
        let out = redact_secrets(text);
        assert!(!out.contains("sk-ant-oat01-Abcdefghijkl"));
        assert!(!out.contains("sk-ant-api03-XYZwxyz"));
        assert!(out.contains("sk-ant-***1234"));
        assert!(out.contains("sk-ant-***9876"));
    }

    #[test]
    fn redact_preserves_non_secret_text() {
        let t = "no secrets here, just some code";
        assert_eq!(redact_secrets(t), t);
    }

    #[test]
    fn redact_leaves_short_prefix_truncated() {
        // Too short to expose suffix safely.
        let t = "sk-ant-ab";
        assert_eq!(redact_secrets(t), "sk-ant-***");
    }

    #[test]
    fn json_export_redacts_secret_in_event_text() {
        let mut d = sample_detail();
        if let SessionEvent::UserText { text, .. } = &mut d.events[0] {
            *text = "see sk-ant-oat01-AbcdWxYz0000".into();
        }
        let out = export_json(&d);
        assert!(!out.contains("sk-ant-oat01-AbcdWxYz0000"));
        assert!(out.contains("sk-ant-***0000"));
    }

    #[test]
    fn compact_divider_surfaces_in_markdown() {
        let mut d = sample_detail();
        d.events.push(SessionEvent::Summary {
            ts: ts("2026-04-10T10:00:10Z"),
            uuid: Some("u5".into()),
            text: "compacted pass 1".into(),
        });
        let out = export_markdown(&d);
        assert!(out.contains("Compacted"));
        assert!(out.contains("compacted pass 1"));
    }
}
