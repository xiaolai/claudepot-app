//! Exchange-level FTS5 search over `sessions.db` (WI-004).
//!
//! Public surface:
//!   * `SearchQuery` — user-facing filter struct.
//!   * `SearchHit` — one row of result with everything needed for
//!     a UI render or an MCP locator.
//!   * `search()` — the function the UI / MCP / CLI all call.
//!
//! Two safety invariants from R9 + the WI-004 acceptance:
//!
//!   1. **Phrase escaping.** User input is wrapped as an FTS5
//!      phrase query — `"input"` with internal `"` doubled —
//!      before reaching `MATCH`. This blocks FTS5 operator parsing
//!      (`AND`, `OR`, `NOT`, `NEAR`, `-`, `*`, `:`, `(`, `)`,
//!      embedded `"`) from changing the query's meaning when a
//!      user types e.g. `sk-ant-oat01`.
//!   2. **Snippet redaction.** Every hit's `snippet` is run
//!      through `redaction::apply` before it leaves this function.
//!      Callers (MCP responses, UI cards) emit `hit.snippet`
//!      verbatim; they never reach into the raw `exchanges`
//!      columns directly.

use rusqlite::params_from_iter;
use rusqlite::types::Value;

use crate::redaction::{apply as redact_apply, RedactionPolicy};
use crate::session_index::SessionIndex;

/// Filter knobs for one `search()` call.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Free-text query. Treated as a data string, NOT as FTS5
    /// syntax — the search function phrase-escapes it before MATCH.
    pub query: String,
    /// Restrict by transcript origin. `None` = all sources.
    pub source_kind: Option<String>,
    /// Substring match on `sessions.project_path` (uses LIKE
    /// `%value%`). `None` = no filter.
    pub project_path: Option<String>,
    /// Exact match on `sessions.git_branch`.
    pub git_branch: Option<String>,
    /// Substring match on `sessions.models_json` (covers Claude
    /// multi-model sessions); Codex sessions don't yet populate
    /// `models_json`, so this filter is mostly relevant for Claude.
    pub model: Option<String>,
    /// Inclusive lower bound on `exchanges.timestamp_ms`.
    pub since_ms: Option<i64>,
    /// Inclusive upper bound.
    pub until_ms: Option<i64>,
    /// Result page size. Capped at 50.
    pub limit: u32,
    /// Pagination offset.
    pub offset: u32,
    /// Sort order.
    pub sort: SearchSort,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            source_kind: None,
            project_path: None,
            git_branch: None,
            model: None,
            since_ms: None,
            until_ms: None,
            limit: 20,
            offset: 0,
            sort: SearchSort::Relevance,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SearchSort {
    /// FTS5 BM25 ranking (default).
    Relevance,
    /// Newest exchange first by `timestamp_ms`.
    DateDesc,
    /// Oldest first.
    DateAsc,
}

/// One search result. All fields are emission-safe; the `snippet`
/// has been through `redaction::apply` and is the canonical thing
/// to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub exchange_id: String,
    pub file_path: String,
    pub session_id: String,
    pub source_kind: String,
    pub project_path: String,
    pub git_branch: Option<String>,
    pub timestamp_ms: Option<i64>,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub snippet: String,
    pub turn_index: i64,
}

/// Execute one search against the v4 schema.
///
/// `policy` controls how the returned `snippet` is redacted. Most
/// callers use `RedactionPolicy::default()` which masks
/// `sk-ant-*` tokens; MCP callers may pass a stricter policy that
/// also rewrites paths or emails.
pub fn search(
    idx: &SessionIndex,
    query: &SearchQuery,
    policy: &RedactionPolicy,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    let limit = query.limit.clamp(1, 50);
    let offset = query.offset;

    // Empty query returns no hits (FTS MATCH on the empty string
    // is an error in FTS5; we short-circuit instead of letting
    // the caller hit a confusing SQL error).
    if query.query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let phrase = escape_phrase(&query.query);

    let mut sql = String::from(
        "SELECT \
            ex.id, ex.file_path, s.session_id, ex.source_kind, \
            s.project_path, s.git_branch, ex.timestamp_ms, \
            ex.line_start, ex.line_end, ex.snippet_text, ex.turn_index \
         FROM exchange_fts fts \
         JOIN exchanges ex ON ex.rowid = fts.rowid \
         JOIN sessions s   ON s.file_path = ex.file_path \
         WHERE exchange_fts MATCH ?1",
    );
    let mut binds: Vec<Value> = vec![Value::Text(phrase)];
    let mut next_idx = 2;

    if let Some(ref sk) = query.source_kind {
        sql.push_str(&format!(" AND ex.source_kind = ?{}", next_idx));
        binds.push(Value::Text(sk.clone()));
        next_idx += 1;
    }
    if let Some(ref pp) = query.project_path {
        // L5 — escape LIKE wildcards (`%`, `_`, `\`) in user input
        // so a search for `project_path: "_"` doesn't silently
        // become a single-character wildcard. Pair with an
        // explicit ESCAPE clause so SQLite honors our backslash
        // as the escape character.
        sql.push_str(&format!(" AND s.project_path LIKE ?{} ESCAPE '\\'", next_idx));
        binds.push(Value::Text(format!("%{}%", escape_like(pp))));
        next_idx += 1;
    }
    if let Some(ref gb) = query.git_branch {
        sql.push_str(&format!(" AND s.git_branch = ?{}", next_idx));
        binds.push(Value::Text(gb.clone()));
        next_idx += 1;
    }
    if let Some(ref mdl) = query.model {
        sql.push_str(&format!(" AND s.models_json LIKE ?{} ESCAPE '\\'", next_idx));
        binds.push(Value::Text(format!("%{}%", escape_like(mdl))));
        next_idx += 1;
    }
    if let Some(since) = query.since_ms {
        sql.push_str(&format!(" AND ex.timestamp_ms >= ?{}", next_idx));
        binds.push(Value::Integer(since));
        next_idx += 1;
    }
    if let Some(until) = query.until_ms {
        sql.push_str(&format!(" AND ex.timestamp_ms <= ?{}", next_idx));
        binds.push(Value::Integer(until));
        next_idx += 1;
    }

    sql.push_str(match query.sort {
        SearchSort::Relevance => " ORDER BY rank",
        SearchSort::DateDesc => " ORDER BY ex.timestamp_ms DESC NULLS LAST",
        SearchSort::DateAsc => " ORDER BY ex.timestamp_ms ASC NULLS LAST",
    });

    sql.push_str(&format!(" LIMIT ?{} OFFSET ?{}", next_idx, next_idx + 1));
    binds.push(Value::Integer(limit as i64));
    binds.push(Value::Integer(offset as i64));

    let db = idx.db();
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(binds.iter()), |row| {
        Ok(SearchHit {
            exchange_id: row.get(0)?,
            file_path: row.get(1)?,
            session_id: row.get(2)?,
            source_kind: row.get(3)?,
            project_path: row.get(4)?,
            git_branch: row.get(5)?,
            timestamp_ms: row.get(6)?,
            line_start: row.get(7)?,
            line_end: row.get(8)?,
            snippet: row.get(9)?,
            turn_index: row.get(10)?,
        })
    })?;

    let mut hits = Vec::new();
    for h in rows {
        let mut hit = h?;
        // R9: redact the snippet before it leaves this function.
        // `redaction::apply` is regex-based and constant-time per
        // pattern; for ≤ 50 hits this is sub-millisecond.
        hit.snippet = redact_apply(&hit.snippet, policy);
        hits.push(hit);
    }
    Ok(hits)
}

/// Escape an arbitrary user string as a single FTS5 phrase query.
///
/// FTS5 phrase syntax: `"text with spaces and operators"`. Inside
/// the quotes, FTS5 treats nearly everything as a literal token;
/// the only character that needs escaping is `"` itself, which is
/// doubled (`""`). Operators like `AND`, `NEAR`, `*`, `-`, `:`,
/// `(`, `)` are all literal inside a phrase.
///
/// Result is always wrapped in quotes — even single-word inputs
/// pass through this function so the FTS5 parser uses phrase mode
/// uniformly.
///
/// L6 — also strips ASCII control characters (`\0` through `\x1f`
/// except `\t`, `\n`, `\r`) defensively. FTS5's tokenizer doesn't
/// document explicit handling for these and a future tokenizer
/// change could surprise us; stripping them at the input boundary
/// is cheap and removes the corner case from the threat model.
pub fn escape_phrase(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for c in input.chars() {
        // Strip ASCII control chars except tab/newline/carriage
        // return (which whitespace-segment the phrase but otherwise
        // don't perturb the tokenizer).
        if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') {
            continue;
        }
        if c == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Escape `%`, `_`, and `\` inside a string destined for a LIKE
/// pattern. Paired with `ESCAPE '\\'` on the SQL side so the
/// escape character is unambiguous. Used by the project_path and
/// model filters in `search`.
fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

// ─── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_memory::indexer::backfill_codex;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn open_idx(tmp: &TempDir) -> SessionIndex {
        SessionIndex::open(&tmp.path().join("sessions.db")).expect("open")
    }

    fn stage_corpus(tmp: &TempDir) -> PathBuf {
        let root = tmp.path().join("codex").join("sessions");
        let day = root.join("2026/05/15");
        fs::create_dir_all(&day).unwrap();
        // Three transcripts with distinct content.
        fs::write(
            day.join("a.jsonl"),
            r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"sa","cwd":"/proj-a","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"refactor the user auth flow"}]}}
{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"split into login + signup modules"}]}}
"#,
        )
        .unwrap();
        fs::write(
            day.join("b.jsonl"),
            r#"{"timestamp":"2026-05-15T12:00:00.000Z","type":"session_meta","payload":{"id":"sb","cwd":"/proj-b","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T12:00:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"what does sk-ant-oat01-AbcDefGhi mean"}]}}
{"timestamp":"2026-05-15T12:00:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"that prefix is an Anthropic OAuth token; redact it before sharing"}]}}
"#,
        )
        .unwrap();
        fs::write(
            day.join("c.jsonl"),
            r#"{"timestamp":"2026-05-15T13:00:00.000Z","type":"session_meta","payload":{"id":"sc","cwd":"/proj-c","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T13:00:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"NEAR is an FTS operator and (parens) are syntax"}]}}
{"timestamp":"2026-05-15T13:00:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"yes - and the dash too"}]}}
"#,
        )
        .unwrap();
        root
    }

    fn prep(tmp: &TempDir) -> (SessionIndex, PathBuf) {
        let idx = open_idx(tmp);
        let root = stage_corpus(tmp);
        backfill_codex(&idx, &root).expect("backfill");
        (idx, root)
    }

    // ─── escape_phrase ─────────────────────────────────────────

    #[test]
    fn escape_phrase_wraps_in_quotes() {
        assert_eq!(escape_phrase("hello"), "\"hello\"");
    }

    #[test]
    fn escape_phrase_doubles_internal_quotes() {
        assert_eq!(escape_phrase(r#"he said "hi""#), "\"he said \"\"hi\"\"\"");
    }

    #[test]
    fn escape_phrase_preserves_fts_operators() {
        // Inside phrase quotes, FTS5 operators are literal.
        // The escape function must NOT special-case them.
        for op in ["NEAR", "AND", "OR", "NOT", "*", "(", ")", "-", ":"] {
            let s = format!("foo {op} bar");
            let escaped = escape_phrase(&s);
            assert_eq!(escaped, format!("\"{s}\""), "operator {op}");
        }
    }

    // ─── basic search ──────────────────────────────────────────

    #[test]
    fn search_finds_text_in_user_messages() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        let q = SearchQuery {
            query: "refactor".to_string(),
            ..Default::default()
        };
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sa");
    }

    #[test]
    fn search_finds_text_in_assistant_messages() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        let q = SearchQuery {
            query: "Anthropic".to_string(),
            ..Default::default()
        };
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sb");
    }

    // ─── redaction ─────────────────────────────────────────────

    #[test]
    fn search_redacts_anthropic_tokens_in_snippets() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        // Query the token verbatim. FTS may match the b.jsonl
        // exchange that contains it.
        let q = SearchQuery {
            query: "sk-ant-oat01".to_string(),
            ..Default::default()
        };
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        // R9 walkthrough: the raw token was indexed into FTS, the
        // FTS matched it, but the returned snippet must not
        // contain the literal token.
        assert!(
            !hit.snippet.contains("sk-ant-oat01-AbcDefGhi"),
            "raw token must not appear in snippet, got: {}",
            hit.snippet
        );
        // The default redaction policy masks to `sk-ant-***<last4>`.
        // We don't need the literal mask here — just verify the
        // raw token is absent.
    }

    // ─── adversarial query inputs ──────────────────────────────

    #[test]
    fn search_does_not_error_on_adversarial_input() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        // None of these should cause an FTS5 syntax error
        // because the search function phrase-escapes everything.
        for adversarial in [
            "sk-ant-oat01",
            "-",
            "*",
            "NEAR",
            "AND OR NOT",
            "(",
            ":",
            r#"a "quoted" b"#,
            "",  // empty short-circuits to []
        ] {
            let q = SearchQuery {
                query: adversarial.to_string(),
                ..Default::default()
            };
            let result = search(&idx, &q, &RedactionPolicy::default());
            assert!(result.is_ok(), "adversarial input {adversarial:?} errored");
        }
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        let q = SearchQuery::default();
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert!(hits.is_empty());
    }

    // ─── filters ───────────────────────────────────────────────

    #[test]
    fn source_kind_filter_narrows_results() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        // All three corpus rollouts are Codex; filter='codex' = all.
        let q = SearchQuery {
            query: "is".to_string(),
            source_kind: Some("codex".to_string()),
            ..Default::default()
        };
        let codex_hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        // claude_code filter should now return 0.
        let q2 = SearchQuery {
            source_kind: Some("claude_code".to_string()),
            ..q.clone()
        };
        let claude_hits = search(&idx, &q2, &RedactionPolicy::default()).unwrap();
        assert!(!codex_hits.is_empty());
        assert!(claude_hits.is_empty());
    }

    #[test]
    fn project_path_filter_substring_match() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        let q = SearchQuery {
            query: "is".to_string(),
            project_path: Some("proj-b".to_string()),
            ..Default::default()
        };
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert!(hits.iter().all(|h| h.project_path.contains("proj-b")));
    }

    // ─── pagination ────────────────────────────────────────────

    #[test]
    fn limit_caps_results_and_offset_paginates() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        // Three rollouts all contain "and" or similar common
        // words; use a guaranteed-hit query.
        let q = SearchQuery {
            query: "and".to_string(),
            limit: 1,
            offset: 0,
            ..Default::default()
        };
        let page1 = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert!(page1.len() <= 1);

        let q2 = SearchQuery {
            offset: 1,
            ..q.clone()
        };
        let page2 = search(&idx, &q2, &RedactionPolicy::default()).unwrap();
        // page2 should not be identical to page1.
        if !page1.is_empty() && !page2.is_empty() {
            assert_ne!(page1[0].exchange_id, page2[0].exchange_id);
        }
    }

    #[test]
    fn limit_caps_at_fifty_even_when_requested_higher() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        let q = SearchQuery {
            query: "is".to_string(),
            limit: 9999,
            ..Default::default()
        };
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert!(hits.len() <= 50);
    }

    // ─── locator stability ────────────────────────────────────

    #[test]
    fn locator_fields_present_on_every_hit() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = prep(&tmp);
        let q = SearchQuery {
            query: "refactor".to_string(),
            ..Default::default()
        };
        let hits = search(&idx, &q, &RedactionPolicy::default()).unwrap();
        assert!(!hits.is_empty());
        let hit = &hits[0];
        assert!(!hit.file_path.is_empty());
        assert!(!hit.session_id.is_empty());
        assert!(!hit.exchange_id.is_empty());
        assert!(!hit.source_kind.is_empty());
    }
}
