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
    let needle = query.to_ascii_lowercase();
    let mut hits = Vec::new();

    for row in rows {
        if hits.len() >= limit {
            break;
        }
        // Fast-path: row-level fields (first_user_prompt, display_slug)
        // give us a synthetic hit without opening the file.
        if let Some(fp) = &row.first_user_prompt {
            if fp.to_ascii_lowercase().contains(&needle) {
                let offset = fp.to_ascii_lowercase().find(&needle).unwrap_or(0);
                hits.push(SearchHit {
                    session_id: row.session_id.clone(),
                    slug: row.slug.clone(),
                    file_path: row.file_path.clone(),
                    project_path: row.project_path.clone(),
                    role: "user".into(),
                    snippet: make_snippet(fp, offset, needle.len()),
                    match_offset: offset,
                    last_ts: row.last_ts,
                });
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
        let lower = text.to_ascii_lowercase();
        if let Some(off) = lower.find(needle) {
            hits.push(SearchHit {
                session_id: row.session_id.clone(),
                slug: row.slug.clone(),
                file_path: row.file_path.clone(),
                project_path: row.project_path.clone(),
                role: role.into(),
                snippet: make_snippet(&text, off, needle.len()),
                match_offset: off,
                last_ts: row.last_ts,
            });
            return Ok(());
        }
        if hits.len() >= limit {
            return Ok(());
        }
    }
    Ok(())
}

/// Pull out the user's typed prompt, skipping tool-result arrays.
fn extract_user_text(v: &serde_json::Value) -> Option<String> {
    let msg = v.get("message")?;
    match msg.get("content")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => parts
            .iter()
            .find_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    p.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            }),
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

fn make_snippet(text: &str, offset: usize, needle_len: usize) -> String {
    const WINDOW: usize = 48;
    // Work on chars, not bytes, to avoid slicing multi-byte code points.
    let chars: Vec<char> = text.chars().collect();
    // Approximate char-offset from byte-offset by counting chars up to offset.
    let byte_off = offset;
    let mut char_off = 0usize;
    let mut cum = 0usize;
    for (i, c) in text.char_indices() {
        if i >= byte_off {
            char_off = cum;
            break;
        }
        cum = i + c.len_utf8();
    }
    if char_off == 0 {
        char_off = cum;
    }
    let char_off = char_off.min(chars.len());
    let start = char_off.saturating_sub(WINDOW);
    let end = (char_off + needle_len + WINDOW).min(chars.len());
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < chars.len() { "…" } else { "" };
    let body: String = chars[start..end]
        .iter()
        .map(|c| if *c == '\n' || *c == '\r' { ' ' } else { *c })
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
    let needle = query.to_ascii_lowercase();
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
        let lower = text.to_ascii_lowercase();
        if let Some(off) = lower.find(&needle) {
            out.push((i, ev, make_snippet(text, off, needle.len())));
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
}
