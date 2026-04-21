//! Cross-session full-text search.
//!
//! Claudepot's existing persistent index (`sessions.db`) only keeps
//! lightweight row metadata (first prompt, tokens, model). A richer
//! content search — "find the session where I asked about JWT" — needs
//! to peek inside each transcript.
//!
//! This module does the work on demand: for each `SessionRow` passed
//! in, it opens the `.jsonl`, extracts the user-typed text and the
//! assistant's final text output per turn, and runs a case-insensitive
//! substring match against the query. Results are ranked by the match
//! location (earlier-matching hits rank higher) and capped by the
//! caller.
//!
//! The search is read-only; there's no mutation on disk.

use crate::session::{SessionError, SessionEvent, SessionRow};
use crate::session_export::redact_secrets;
use serde::Serialize;
use std::fs;
use std::io::{BufRead, BufReader};

/// A single hit. `snippet` is ±48 chars around the match, normalized
/// to one line for preview.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub session_id: String,
    pub slug: String,
    pub file_path: std::path::PathBuf,
    pub project_path: String,
    /// Role that produced the matched text: `"user"` or `"assistant"`.
    pub role: String,
    pub snippet: String,
    /// Character offset of the match within the matched turn.
    pub match_offset: usize,
    /// `last_ts` from the row, for sorting on caller side.
    pub last_ts: Option<chrono::DateTime<chrono::Utc>>,
}

/// Run a query across `rows`. Stops collecting once `limit` hits have
/// accumulated — the caller is expected to pre-sort rows the way they
/// want matches prioritized (e.g., newest-first).
pub fn search_rows(
    rows: &[SessionRow],
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, SessionError> {
    if query.trim().len() < 2 {
        return Ok(Vec::new());
    }
    let needle = query.to_lowercase();
    let mut hits = Vec::new();

    for row in rows {
        if hits.len() >= limit {
            break;
        }
        // Fast-path: row-level fields (first_user_prompt) give us a
        // synthetic hit without opening the file.
        if let Some(fp) = &row.first_user_prompt {
            if let Some(offset) = find_case_insensitive(fp, &needle) {
                hits.push(make_hit(
                    row,
                    "user",
                    fp,
                    offset,
                    needle_char_len(&needle),
                ));
                if hits.len() >= limit {
                    break;
                }
                continue; // don't also scan the file for this session
            }
        }

        scan_file(row, &needle, limit, &mut hits)?;
    }

    Ok(hits)
}

/// Case-insensitive substring scan that handles Unicode. Matches are
/// found in the lowercased haystack, then remapped to the original
/// string by tracking how many source chars produced each lowercase
/// char. This survives **expanding case folds** — e.g. `İ` (U+0130)
/// lowercases to `i\u{0307}` (two chars), so a naive "count lowercase
/// chars before the match, then walk original chars" is off by one for
/// every expansion in the prefix.
///
/// Returns the byte offset of the first match in the original string.
fn find_case_insensitive(haystack: &str, needle_lower: &str) -> Option<usize> {
    if needle_lower.is_empty() {
        return None;
    }

    // Build the lowercased haystack alongside a parallel array that
    // records, for each byte in the lowercased form, the byte offset
    // of the *source* character in the original string. That gives us
    // a direct byte->byte map even when case folding expands chars.
    let mut lower = String::with_capacity(haystack.len());
    let mut src_byte_of_lower_byte: Vec<usize> = Vec::with_capacity(haystack.len());
    for (src_idx, c) in haystack.char_indices() {
        for lc in c.to_lowercase() {
            let before = lower.len();
            lower.push(lc);
            for _ in before..lower.len() {
                src_byte_of_lower_byte.push(src_idx);
            }
        }
    }

    let lower_off = lower.find(needle_lower)?;
    Some(src_byte_of_lower_byte[lower_off])
}

fn needle_char_len(needle_lower: &str) -> usize {
    needle_lower.chars().count()
}

fn make_hit(
    row: &SessionRow,
    role: &str,
    text: &str,
    byte_off: usize,
    needle_char_len: usize,
) -> SearchHit {
    SearchHit {
        session_id: row.session_id.clone(),
        slug: row.slug.clone(),
        file_path: row.file_path.clone(),
        project_path: row.project_path.clone(),
        role: role.into(),
        snippet: redact_secrets(&make_snippet_chars(text, byte_off, needle_char_len)),
        match_offset: byte_off,
        last_ts: row.last_ts,
    }
}

/// Open the JSONL and scan every user / assistant turn for matches.
/// One hit per session — the first match wins to keep the result set
/// compact. Callers wanting full inline match lists should stream
/// directly.
fn scan_file(
    row: &SessionRow,
    needle: &str,
    limit: usize,
    hits: &mut Vec<SearchHit>,
) -> Result<(), SessionError> {
    let file = match fs::File::open(&row.file_path) {
        Ok(f) => f,
        Err(_) => return Ok(()), // missing file — skip silently
    };
    let reader = BufReader::new(file);
    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let event_type = v
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let (role, text) = match event_type {
            "user" => ("user", extract_user_text(&v)),
            "assistant" => ("assistant", extract_assistant_text(&v)),
            _ => continue,
        };
        let Some(text) = text else { continue };
        if let Some(off) = find_case_insensitive(&text, needle) {
            hits.push(make_hit(row, role, &text, off, needle_char_len(needle)));
            return Ok(());
        }
        if hits.len() >= limit {
            return Ok(());
        }
    }
    Ok(())
}

/// Pull out every user text block in a single turn (skipping
/// tool_result entries). Joined with a space so a later match in the
/// same turn is still reachable.
fn extract_user_text(v: &serde_json::Value) -> Option<String> {
    let msg = v.get("message")?;
    match msg.get("content")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => {
            let joined: String = parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                        p.get("text").and_then(|t| t.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

/// Concatenate the assistant's final text blocks for one turn.
fn extract_assistant_text(v: &serde_json::Value) -> Option<String> {
    let msg = v.get("message")?;
    let parts = msg.get("content").and_then(|c| c.as_array())?;
    let joined: String = parts
        .iter()
        .filter_map(|p| {
            if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                p.get("text").and_then(|t| t.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Build a ±WINDOW-char snippet around a match, counted in **chars**
/// (Unicode scalar values). Input is the original haystack string and
/// the byte offset of the match within it; we convert to a char index
/// correctly for multi-byte code points.
///
/// Replaces `\n`/`\r` with spaces so the preview fits on one line.
fn make_snippet_chars(text: &str, byte_off: usize, needle_char_len: usize) -> String {
    const WINDOW: usize = 48;
    // Count the characters strictly before `byte_off`. Walking
    // char_indices() lands on each code-point boundary, so the count
    // of boundaries with `idx < byte_off` is the char index.
    let char_off = text
        .char_indices()
        .position(|(idx, _)| idx >= byte_off)
        .unwrap_or_else(|| text.chars().count());
    let total_chars = text.chars().count();
    let start = char_off.saturating_sub(WINDOW);
    let end = (char_off + needle_char_len + WINDOW).min(total_chars);
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < total_chars { "…" } else { "" };
    let body: String = text
        .chars()
        .skip(start)
        .take(end - start)
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    format!("{prefix}{body}{suffix}")
}

/// Convenience: scan events already parsed in memory.
///
/// Helper for test code and internal callers that have a parsed
/// `Vec<SessionEvent>` handy and don't want to re-open the file.
pub fn search_events<'a>(
    events: &'a [SessionEvent],
    query: &str,
) -> Vec<(usize, &'a SessionEvent, String)> {
    if query.trim().len() < 2 {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let mut out = Vec::new();
    for (i, ev) in events.iter().enumerate() {
        let text = match ev {
            SessionEvent::UserText { text, .. }
            | SessionEvent::AssistantText { text, .. }
            | SessionEvent::AssistantThinking { text, .. }
            | SessionEvent::Summary { text, .. } => Some(text.as_str()),
            SessionEvent::UserToolResult { content, .. } => Some(content.as_str()),
            SessionEvent::AssistantToolUse { input_preview, .. } => Some(input_preview.as_str()),
            _ => None,
        };
        let Some(text) = text else { continue };
        if let Some(off) = find_case_insensitive(text, &needle) {
            out.push((
                i,
                ev,
                redact_secrets(&make_snippet_chars(
                    text,
                    off,
                    needle_char_len(&needle),
                )),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::TokenUsage;
    use chrono::{DateTime, Utc};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse::<DateTime<Utc>>().unwrap())
    }

    fn row(
        session_id: &str,
        slug: &str,
        file_path: PathBuf,
        first_prompt: Option<&str>,
        last: Option<DateTime<Utc>>,
    ) -> SessionRow {
        SessionRow {
            session_id: session_id.into(),
            slug: slug.into(),
            file_path,
            file_size_bytes: 0,
            last_modified: None,
            project_path: "/repo".into(),
            project_from_transcript: true,
            first_ts: None,
            last_ts: last,
            event_count: 0,
            message_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            first_user_prompt: first_prompt.map(String::from),
            models: vec![],
            tokens: TokenUsage::default(),
            git_branch: None,
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let mut f = fs::File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    #[test]
    fn short_query_returns_empty() {
        let hits = search_rows(&[], "a", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn first_prompt_fast_path_yields_hit_without_reading_file() {
        let hits = search_rows(
            &[row(
                "s1",
                "-r",
                PathBuf::from("/does/not/exist.jsonl"),
                Some("investigate the deadlock"),
                ts("2026-04-10T10:00:00Z"),
            )],
            "deadlock",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "user");
        assert!(hits[0].snippet.contains("deadlock"));
    }

    #[test]
    fn scans_file_when_first_prompt_misses() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("s.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"user","message":{"role":"user","content":"talk about JWT"},"sessionId":"s"}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"signed token demo"}]},"sessionId":"s"}"#,
            ],
        );
        let hits = search_rows(
            &[row(
                "s",
                "-r",
                path,
                Some("unrelated"),
                ts("2026-04-10T10:00:00Z"),
            )],
            "jwt",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "user");
    }

    #[test]
    fn match_can_come_from_assistant_text() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("s.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"user","message":{"role":"user","content":"any clue"},"sessionId":"s"}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"deadlock culprit is mutex B"}]},"sessionId":"s"}"#,
            ],
        );
        let hits = search_rows(
            &[row("s", "-r", path, Some("nothing interesting"), None)],
            "deadlock",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "assistant");
    }

    #[test]
    fn limit_stops_early() {
        let tmp = TempDir::new().unwrap();
        let mut rows = Vec::new();
        for i in 0..5 {
            let path = tmp.path().join(format!("s{i}.jsonl"));
            write_jsonl(
                &path,
                &[r#"{"type":"user","message":{"role":"user","content":"widget search"},"sessionId":"s"}"#],
            );
            rows.push(row(&format!("s{i}"), "-r", path, None, None));
        }
        let hits = search_rows(&rows, "widget", 3).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn missing_file_is_silently_skipped() {
        let hits = search_rows(
            &[row(
                "s1",
                "-r",
                PathBuf::from("/tmp/definitely-missing-xyz.jsonl"),
                None,
                None,
            )],
            "anything",
            10,
        )
        .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_events_returns_positional_hits() {
        let events = vec![
            SessionEvent::UserText {
                ts: None,
                uuid: None,
                text: "fix the login bug".into(),
            },
            SessionEvent::AssistantText {
                ts: None,
                uuid: None,
                model: None,
                text: "found root cause in login.rs".into(),
                usage: None,
                stop_reason: None,
            },
        ];
        let hits = search_events(&events, "login");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, 0);
        assert_eq!(hits[1].0, 1);
    }

    #[test]
    fn snippet_is_bounded_and_trims_newlines() {
        let events = vec![SessionEvent::UserText {
            ts: None,
            uuid: None,
            text: "padding ".repeat(50) + "LOGIN\nmore padding",
        }];
        let hits = search_events(&events, "login");
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].2.contains('\n'));
        assert!(hits[0].2.contains('…')); // bounded
    }

    #[test]
    fn search_redacts_sk_ant_tokens_in_snippet() {
        let events = vec![SessionEvent::AssistantText {
            ts: None,
            uuid: None,
            model: None,
            text: "leaked sk-ant-oat01-AbcdWxYz0000 keep searching".into(),
            usage: None,
            stop_reason: None,
        }];
        let hits = search_events(&events, "keep");
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].2.contains("sk-ant-oat01-AbcdWxYz0000"));
        assert!(hits[0].2.contains("sk-ant-***0000"));
    }

    #[test]
    fn search_finds_match_in_second_user_text_block() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("multi.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"first block unrelated"},{"type":"text","text":"second block has widget"}]},"sessionId":"m"}"#,
            ],
        );
        let hits = search_rows(
            &[row("m", "-r", path, Some("unrelated"), None)],
            "widget",
            5,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("widget"));
    }

    #[test]
    fn search_handles_expanding_case_fold_prefixes() {
        // `İ` lowercases to `i\u{0307}` — the case fold produces more
        // chars than the source. A naive byte-offset remap would point
        // past the `İ` into the following character, producing a
        // snippet that starts at the wrong place.
        let events = vec![SessionEvent::UserText {
            ts: None,
            uuid: None,
            text: "İstanbul recipe".into(),
        }];
        let hits = search_events(&events, "recipe");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].2.contains("recipe"));
        // The whole source line is short enough to fit the window, so
        // the snippet should start from the original start, not halfway
        // into `İstanbul`.
        assert!(hits[0].2.starts_with("İstanbul") || hits[0].2.starts_with("…"));
    }

    #[test]
    fn search_is_unicode_case_insensitive() {
        let events = vec![SessionEvent::UserText {
            ts: None,
            uuid: None,
            text: "Café opens early".into(),
        }];
        // Lowercase `é` in the query matches capital `É` implicitly by
        // Unicode lowercase folding.
        let hits = search_events(&events, "café");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].2.contains("Café"));
    }
}
