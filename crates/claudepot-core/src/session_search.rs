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
use crate::session_search_ranking::{classify_match, rank_hits};
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
    /// Relevance score in [0.0, 1.0]. Higher is better. Rules:
    /// 1.0 = match is bounded by non-word chars on both sides (exact phrase);
    /// 0.7 = match starts at a word boundary (word-prefix);
    /// 0.4 = match is inside a word (pure substring).
    pub score: f32,
}

/// Validated user query. Rejects trimmed length < 2 so callers see the
/// same guard the CLI and UI already apply.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub limit: usize,
}

impl SearchQuery {
    /// Build a query. Trims the text, returns `None` if too short.
    /// `limit == 0` is coerced to 1 — zero-limit calls are never useful.
    pub fn new(text: impl Into<String>, limit: usize) -> Option<Self> {
        let text = text.into();
        if text.trim().len() < 2 {
            return None;
        }
        Some(Self {
            text,
            limit: limit.max(1),
        })
    }
}

/// Run a query across `rows`.
///
/// Scans every row, collects all hits, ranks by `(score desc, last_ts desc)`,
/// then truncates to `limit`. This ensures the globally best-scoring
/// matches win even when more than `limit` candidates exist — applying
/// the cap before ranking would drop better phrase matches in favor of
/// earlier substring hits.
///
/// For very large deployments this could be bounded by a max-scan
/// budget; today the scanner already stops at the first match per
/// file, so the work is O(rows) and fine.
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
        // Fast-path: row-level fields (first_user_prompt) give us a
        // synthetic hit without opening the file.
        if let Some(fp) = &row.first_user_prompt {
            if let Some((off, end)) = find_case_insensitive(fp, &needle) {
                hits.push(make_hit(
                    row,
                    "user",
                    fp,
                    off,
                    needle_char_len(&needle),
                    end - off,
                ));
                continue; // don't also scan the file for this session
            }
        }

        // `scan_file` still stops at the first match per file — one
        // hit per session keeps the result set compact. We pass
        // `usize::MAX` as the internal cap so scan_file doesn't cut
        // us off before the global ranking stage.
        scan_file(row, &needle, usize::MAX, &mut hits)?;
    }

    let mut ranked = rank_hits(hits);
    ranked.truncate(limit);
    Ok(ranked)
}

/// Case-insensitive substring scan that handles Unicode. Returns
/// `(byte_start, byte_end)` in the **original** haystack string.
///
/// Matches are found in the lowercased haystack, then remapped to the
/// original string by tracking how many source chars produced each
/// lowercase char. This survives **expanding case folds** — e.g. `İ`
/// (U+0130) lowercases to `i\u{0307}` (two chars), so a naive "count
/// lowercase chars before the match, then walk original chars" is off
/// by one for every expansion in the prefix.
fn find_case_insensitive(haystack: &str, needle_lower: &str) -> Option<(usize, usize)> {
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
    let lower_end = lower_off + needle_lower.len();
    let src_start = src_byte_of_lower_byte[lower_off];
    // The last contributing lower byte belongs to some source char;
    // if that source char has an *expanding* lowercase fold (e.g. `İ`
    // → `i\u{307}`) the plain lookup `src_byte_of_lower_byte[lower_end]`
    // can return the *start* byte of the same source char instead of
    // the byte after it, collapsing the span. Walk forward past every
    // lower byte that still maps to that same source char so `src_end`
    // points to the start of the NEXT source char (or past-the-end).
    let src_end = if lower_end == 0 {
        src_start
    } else {
        let last_contributing_src = src_byte_of_lower_byte[lower_end - 1];
        let mut k = lower_end;
        while k < src_byte_of_lower_byte.len()
            && src_byte_of_lower_byte[k] == last_contributing_src
        {
            k += 1;
        }
        if k >= src_byte_of_lower_byte.len() {
            haystack.len()
        } else {
            src_byte_of_lower_byte[k]
        }
    };
    Some((src_start, src_end))
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
    needle_byte_len: usize,
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
        score: classify_match(text, byte_off, needle_byte_len),
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
        if let Some((off, end)) = find_case_insensitive(&text, needle) {
            hits.push(make_hit(
                row,
                role,
                &text,
                off,
                needle_char_len(needle),
                end - off,
            ));
            return Ok(());
        }
        if hits.len() >= limit {
            return Ok(());
        }
    }
    Ok(())
}

/// Pull every searchable byte out of a user turn. Includes plain text
/// blocks **and** tool_result bodies (string or array-of-text shapes),
/// because in tool-heavy projects the query term often lives only in
/// command output — a scanner that ignores tool_result makes most of
/// the corpus invisible. Blocks are joined with a space so a later
/// match in the same turn is still reachable.
fn extract_user_text(v: &serde_json::Value) -> Option<String> {
    let msg = v.get("message")?;
    match msg.get("content")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => {
            let mut pieces: Vec<String> = Vec::new();
            for p in parts {
                let kind = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match kind {
                    "text" => {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            pieces.push(t.to_string());
                        }
                    }
                    "tool_result" => match p.get("content") {
                        Some(serde_json::Value::String(s)) => pieces.push(s.clone()),
                        Some(serde_json::Value::Array(inner)) => {
                            for ip in inner {
                                if let Some(t) = ip.get("text").and_then(|t| t.as_str()) {
                                    pieces.push(t.to_string());
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            if pieces.is_empty() {
                None
            } else {
                Some(pieces.join(" "))
            }
        }
        _ => None,
    }
}

/// Pull every searchable byte out of an assistant turn. Covers plain
/// `text` blocks, `thinking` (the model's internal reasoning), and
/// `tool_use.input` (serialized as JSON so Bash commands, file paths,
/// and other tool arguments are reachable by substring). The parity
/// target is `search_events`, which already surfaces these shapes for
/// in-memory callers.
fn extract_assistant_text(v: &serde_json::Value) -> Option<String> {
    let msg = v.get("message")?;
    let parts = msg.get("content").and_then(|c| c.as_array())?;
    let mut pieces: Vec<String> = Vec::new();
    for p in parts {
        let kind = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                    pieces.push(t.to_string());
                }
            }
            "thinking" => {
                if let Some(t) = p.get("thinking").and_then(|t| t.as_str()) {
                    pieces.push(t.to_string());
                }
            }
            "tool_use" => {
                if let Some(input) = p.get("input") {
                    pieces.push(input.to_string());
                }
            }
            _ => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join(" "))
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
        if let Some((off, _end)) = find_case_insensitive(text, &needle) {
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
    fn search_matches_tool_result_string_content() {
        // A CC user message whose only content is a tool_result with a
        // plain string body — the shape emitted for Bash/Read output
        // whose stdout fits in one string. Before the fix, the scanner
        // skipped these entirely, so any session whose mention of the
        // query only appeared in command output was invisible.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("tr_str.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"user","message":{"role":"user","content":"unrelated intro"},"sessionId":"s"}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"src-tauri/src/commands.rs line 42","is_error":false}]},"sessionId":"s"}"#,
            ],
        );
        let hits = search_rows(
            &[row("s", "-r", path, Some("unrelated"), None)],
            "tauri",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("tauri"));
    }

    #[test]
    fn search_matches_tool_result_array_content() {
        // A tool_result with array-shaped content (CC emits this when
        // the tool stitches together multiple text blocks, e.g. a Read
        // that returns line-numbered text). Match the inner `text` of
        // any part.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("tr_arr.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"nothing to see"},{"type":"text","text":"pnpm tauri dev finished"}],"is_error":false}]},"sessionId":"s"}"#,
            ],
        );
        let hits = search_rows(
            &[row("s", "-r", path, None, None)],
            "tauri",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("tauri"));
    }

    #[test]
    fn search_matches_assistant_tool_use_input() {
        // The assistant invokes Bash with `pnpm tauri dev` as the
        // command argument. "tauri" never appears in any plain text
        // block, only inside the serialized tool input. Before the fix
        // this session was invisible to the scanner.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("tu.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"let me check"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pnpm tauri dev","description":"start dev server"}}]},"sessionId":"s"}"#,
            ],
        );
        let hits = search_rows(
            &[row("s", "-r", path, None, None)],
            "tauri",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].snippet.contains("tauri"));
    }

    #[test]
    fn search_matches_assistant_thinking_block() {
        // Assistant `thinking` blocks carry the model's internal
        // reasoning. Users searching for a topic they mulled over want
        // these to count — the in-memory `search_events` helper already
        // includes them, so `search_rows` must match.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("think.jsonl");
        write_jsonl(
            &path,
            &[
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"I should inspect the tauri config next"}]},"sessionId":"s"}"#,
            ],
        );
        let hits = search_rows(
            &[row("s", "-r", path, None, None)],
            "tauri",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].snippet.contains("tauri"));
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
    fn search_ranks_globally_not_per_limit_window() {
        // Three substring matches followed by one phrase match. With
        // limit=3, a naive "stop at limit then rank" would drop the
        // phrase hit; the fix must scan everything first and truncate
        // after ranking.
        let sub1 = row(
            "sub1",
            "-r",
            PathBuf::new(),
            Some("unauthorized first"),
            ts("2026-04-10T10:00:00Z"),
        );
        let sub2 = row(
            "sub2",
            "-r",
            PathBuf::new(),
            Some("unauthorized second"),
            ts("2026-04-10T11:00:00Z"),
        );
        let sub3 = row(
            "sub3",
            "-r",
            PathBuf::new(),
            Some("unauthorized third"),
            ts("2026-04-10T12:00:00Z"),
        );
        let phrase = row(
            "phrase",
            "-r",
            PathBuf::new(),
            Some("auth here"),
            ts("2020-01-01T00:00:00Z"),
        );
        let hits = search_rows(
            &[sub1, sub2, sub3, phrase],
            "auth",
            3,
        )
        .unwrap();
        assert_eq!(hits.len(), 3);
        // The phrase match must survive the limit — it's the best score.
        assert!(
            hits.iter().any(|h| h.session_id == "phrase"),
            "phrase hit must win against three substring hits, got ids {:?}",
            hits.iter().map(|h| &h.session_id).collect::<Vec<_>>()
        );
        // And it must be first.
        assert_eq!(hits[0].session_id, "phrase");
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
    fn search_spans_the_full_source_char_when_match_ends_mid_expansion_fold() {
        // Lowercase of `İ` (U+0130) is `i\u{307}` — 2 lowercase chars
        // from a single source char. Searching `"xi"` inside `"Xİ …"`
        // finds the "xi" in the lowered haystack; the match's end
        // byte sits on the first byte of the combining-mark expansion,
        // which belongs to the SAME source char as its neighbor.
        //
        // The remap must treat the source char as atomic: the span
        // must extend to the byte AFTER `İ`, not stop inside it. If
        // the span collapses to just `"X"` (1 byte) then
        // `classify_match` sees `İ` as the "after" char (alphanumeric)
        // and scores SUBSTRING instead of PHRASE, which mis-ranks the
        // hit against other substring matches.
        //
        // The text here ends right after `İ` so the correct boundary
        // is "no alphanumeric follows" → SCORE_PHRASE.
        let text_only_phrase = "Xİ"; // X + İ
        let events = vec![SessionEvent::UserText {
            ts: None,
            uuid: None,
            text: text_only_phrase.into(),
        }];
        let hits = search_events(&events, "xi");
        assert_eq!(hits.len(), 1);
        // With the buggy remap the span is 1 byte (`X`) and
        // classify_match sees `İ` trailing the match → SUBSTRING.
        // A correct remap spans both `X` and `İ` (3 bytes total) →
        // the remaining haystack is empty → PHRASE.
        //
        // Go through the rows API so `classify_match` actually runs;
        // `search_events` skips scoring.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("fold.jsonl");
        write_jsonl(
            &path,
            &[
                &format!(
                    r#"{{"type":"user","message":{{"role":"user","content":"{}"}},"sessionId":"s"}}"#,
                    text_only_phrase
                ),
            ],
        );
        let hits = search_rows(
            &[row("s", "-r", path, None, ts("2026-04-10T10:00:00Z"))],
            "xi",
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        // 1.0 == SCORE_PHRASE. An off-by-one span collapse produces
        // 0.4 (SCORE_SUBSTRING); an off-by-one prefix keeps 0.7.
        assert!(
            (hits[0].score - 1.0).abs() < f32::EPSILON,
            "expected phrase score 1.0, got {}",
            hits[0].score
        );
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
    fn search_query_new_rejects_short_input() {
        assert!(SearchQuery::new("", 10).is_none());
        assert!(SearchQuery::new(" ", 10).is_none());
        assert!(SearchQuery::new("x", 10).is_none());
        assert!(SearchQuery::new(" x ", 10).is_none());
        assert!(SearchQuery::new("ok", 10).is_some());
    }

    #[test]
    fn search_query_coerces_zero_limit_to_one() {
        let q = SearchQuery::new("auth", 0).unwrap();
        assert_eq!(q.limit, 1);
    }

    #[test]
    fn search_rows_returns_ranked_output_phrase_before_substring() {
        let tmp = TempDir::new().unwrap();
        let phrase_path = tmp.path().join("phrase.jsonl");
        let sub_path = tmp.path().join("sub.jsonl");
        // Both files contain the query, but `phrase.jsonl` has it as a
        // standalone word; `sub.jsonl` has it inside another word.
        write_jsonl(
            &phrase_path,
            &[r#"{"type":"user","message":{"role":"user","content":"discuss auth today"},"sessionId":"p"}"#],
        );
        write_jsonl(
            &sub_path,
            &[r#"{"type":"user","message":{"role":"user","content":"unauthorized access"},"sessionId":"s"}"#],
        );
        // Feed substring-match first so recency/input order alone would
        // rank it first. Ranking should flip the order by score.
        let rows = vec![
            row("s", "-r", sub_path, None, ts("2026-04-10T10:00:00Z")),
            row("p", "-r", phrase_path, None, ts("2020-01-01T00:00:00Z")),
        ];
        let hits = search_rows(&rows, "auth", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].session_id, "p");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn search_rows_recency_breaks_ties_among_equal_scores() {
        let older = row("old", "-r", PathBuf::new(), Some("auth matters"), ts("2020-01-01T00:00:00Z"));
        let newer = row("new", "-r", PathBuf::new(), Some("auth matters"), ts("2026-04-10T10:00:00Z"));
        let hits = search_rows(&[older, newer], "auth", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].session_id, "new");
        assert_eq!(hits[1].session_id, "old");
    }

    #[test]
    fn search_rows_populates_score_between_zero_and_one() {
        let r = row("s", "-r", PathBuf::new(), Some("auth wins"), None);
        let hits = search_rows(&[r], "auth", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.0 && hits[0].score <= 1.0);
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
