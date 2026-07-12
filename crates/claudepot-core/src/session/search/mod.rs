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

// Ranking heuristics live in a private sibling — the search module is
// the sole consumer, so they are implementation detail of this boundary.
mod ranking;

use crate::session::{scan_all_sessions_uncached, SessionError, SessionEvent, SessionRow};
use crate::session_export::redact_secrets;
use crate::session_index::SessionIndex;
use ranking::{classify_match, rank_hits};
use serde::Serialize;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// `SessionError` deliberately has no `From<rusqlite::Error>` (its
/// `Index` variant carries a String to avoid an error-type cycle with
/// `SessionIndexError`). Funnel index-layer failures through here.
fn index_err(e: rusqlite::Error) -> SessionError {
    SessionError::Index(e.to_string())
}

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

/// The one cross-session search entry point. **Tiered, best-effort — NOT
/// exhaustive when it finds something.** Read that sentence before relying
/// on this for anything but a jump-to-session palette.
///
/// The predecessor opened and line-parsed every `.jsonl` on disk for every
/// query: complete, and 21 s on a real corpus. That is not a search box,
/// it is a coffee break. This trades a precisely-bounded slice of
/// completeness for three orders of magnitude of latency.
///
/// ## Tier 1 — fast (~10 ms), and what it can miss
///
/// `exchange_fts` (the FTS5 index) plus a scan of any transcript
/// `sessions` knows about that has no `exchanges` rows (a failed or
/// not-yet-run backfill). **If this finds ANY hit, it returns.** So when
/// results exist, these are NOT reported:
///
/// * **tool I/O** — `exchange_fts` covers user/assistant text only; a term
///   living solely in a Bash command or tool result is not in it.
/// * **infix matches** — FTS5 matches tokens and token *prefixes*, never
///   inside a token: `lock` does not find `deadlock` here.
/// * **turns appended since the last backfill** — the FTS lags a growing
///   transcript until the periodic backfill re-indexes it (see
///   `exchange_state`).
///
/// A prose query that already found six sessions does not go looking for a
/// seventh in a table scan. That is the deliberate trade.
///
/// ## Tier 2 — exhaustive, and only when Tier 1 found NOTHING
///
/// An empty result is the one place a slow, complete answer is worth more
/// than a fast one — "no results" must be *true*, not merely "not indexed".
/// So on zero hits it refreshes `sessions` against disk (catching
/// transcripts too new to be in either table), scans what is still
/// un-indexed, and runs `LIKE` passes over exchange text (infix) and
/// `tool_calls` (tool I/O). Hundreds of ms, once, instead of never.
///
/// `idx = None` (the index failed to open) degrades to a full
/// `search_rows` scan: slow, but never wrong.
///
/// ## Ranking
///
/// Tier 1 orders by FTS5 BM25 and takes `limit` sessions from that; the
/// per-hit `score` is then recomputed with `search_rows`' phrase / prefix /
/// substring classifier so hits from every source stay comparable. This is
/// deliberately not the old scanner's global ranking — BM25 is the better
/// relevance signal, and re-ranking the whole corpus is the cost this
/// function exists to avoid.
///
/// Every tier yields **at most one hit per session**, matching `search_rows`.
pub fn search_cross_session(
    idx: Option<&SessionIndex>,
    config_dir: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, SessionError> {
    // Preserve `search_rows`' contract exactly: a short query, or a
    // zero limit, yields nothing. (`search_index` internally coerces
    // limit to >= 1; that must not leak out here.)
    if query.trim().len() < 2 || limit == 0 {
        return Ok(Vec::new());
    }

    let Some(idx) = idx else {
        // No usable index. Do NOT reach for `list_all_sessions` here: it
        // opens its own `SessionIndex` at the global data dir — the very
        // database that just failed to open — and its `refresh()` prunes
        // rows for files it cannot see. Scan the filesystem directly
        // instead: slower, but it touches no shared state and cannot
        // corrupt a cache.
        let rows = scan_all_sessions_uncached(config_dir)?;
        return search_rows(&rows, query, limit);
    };

    // ── Fast path ────────────────────────────────────────────────
    // FTS, plus a scan of any transcript `sessions` knows about but
    // `exchanges` doesn't (a backfill that failed or hasn't reached it).
    // Both are indexed reads; together they answer a normal query in
    // single-digit milliseconds.
    let mut hits = search_index(idx, query, limit).map_err(index_err)?;
    hits.extend(scan_unindexed(idx, query, limit)?);

    let fast = dedupe_by_session(&hits, limit);
    if !fast.is_empty() {
        return Ok(fast);
    }

    // ── Exhaustive path ──────────────────────────────────────────
    // The fast path found NOTHING. Before telling the user "no results",
    // do the complete search — this is the only honest moment to spend
    // real time, and it is why the fast path is allowed to be optimistic.
    //
    // Each of these covers a gap the indexed read genuinely cannot:
    //
    //  * `refresh` — a transcript written since the last backfill is in
    //    neither `exchanges` NOR `sessions`, so nothing above can see it.
    //    Converge with disk, then scan whatever is still un-indexed.
    //  * infix — FTS5 matches tokens and token prefixes, never inside a
    //    token: `lock` cannot find `deadlock` there, though the scanner
    //    this replaced did.
    //  * tool I/O — `exchange_fts` indexes user/assistant text only; a
    //    term living solely in a Bash command or a tool result is
    //    invisible to it.
    //
    // These are unindexed table scans and a stat-walk. They are firmly
    // off the hot path: they run once, only when the answer would
    // otherwise be an empty list.
    // `list_all` refreshes as a side effect — that is exactly what we want
    // here, and the returned rows let us scan anything still un-indexed
    // (including transcripts this refresh just discovered).
    idx.list_all(config_dir)
        .map_err(|e| SessionError::Index(e.to_string()))?;
    hits.extend(scan_unindexed(idx, query, limit)?);
    hits.extend(search_exchanges_infix(idx, query, limit).map_err(index_err)?);
    hits.extend(search_tool_calls(idx, query, limit).map_err(index_err)?);

    Ok(dedupe_by_session(&hits, limit))
}

/// Scan the transcripts `sessions` knows about that `exchanges` does not
/// cover — a backfill that failed on a file, or hasn't reached it yet.
///
/// Two indexed SQL reads and then a scan of only those files. Crucially
/// it does NOT call `list_all`: that refreshes, which stat-walks every
/// transcript and re-parses whatever a live session just appended. An
/// earlier version did, and because one file on a real corpus permanently
/// fails to backfill, the probe was never empty — so *every* keystroke
/// paid for a full refresh and the "fast" path measured 1–2 seconds.
fn scan_unindexed(
    idx: &SessionIndex,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, SessionError> {
    let missing = claude_files_missing_exchanges(idx).map_err(index_err)?;
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
    let rows = idx
        .rows_by_paths(&paths)
        .map_err(|e| SessionError::Index(e.to_string()))?;
    search_rows(&rows, query, limit)
}

/// Rank, then keep only the best-scoring hit per session, then cap.
///
/// `rank_hits` sorts by `(score desc, last_ts desc)`, so the first hit
/// seen for a session is the one worth keeping. Takes `&[SearchHit]` so
/// the caller can probe the deduped count without consuming its
/// candidate list.
fn dedupe_by_session(hits: &[SearchHit], limit: usize) -> Vec<SearchHit> {
    let ranked = rank_hits(hits.to_vec());
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(limit.min(ranked.len()));
    for hit in ranked {
        if out.len() >= limit {
            break;
        }
        if seen.insert(hit.file_path.clone()) {
            out.push(hit);
        }
    }
    out
}

/// FTS-backed search over `exchange_fts`.
///
/// The query is escaped as an FTS5 phrase and given a trailing `*` so
/// the final token matches by **prefix**. Without it, FTS5 matches whole
/// tokens only, and a user typing `rotat` would get nothing while the
/// old substring scanner happily matched `rotation`. Prefix matching
/// closes most of that gap.
///
/// It does not close all of it: FTS5 cannot match *inside* a token, so
/// `lock` no longer finds `deadlock` the way a raw substring scan did.
/// That is the deliberate price of not re-parsing gigabytes of JSONL per
/// keystroke; queries are word-shaped in practice, and `search_tool_calls`
/// (which does use `LIKE`) covers the infix case for tool content.
fn search_index(
    idx: &SessionIndex,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    if query.trim().len() < 2 {
        return Ok(Vec::new());
    }
    let limit = limit.max(1);
    // Reuse the Shared Memory phrase-escaper so FTS5 operators inside
    // the user's query (`-`, `*`, `NEAR`, embedded quotes, …) can't
    // change the query's meaning or raise a MATCH syntax error. The
    // trailing `*` sits OUTSIDE the quoted phrase, which is FTS5's
    // prefix-query syntax.
    let phrase = format!("{}*", crate::shared_memory::search::escape_phrase(query));
    let needle = query.to_lowercase();

    let db = idx.db();
    // Collapse to the best-ranked exchange PER SESSION in SQL, so `LIMIT`
    // counts sessions rather than turns.
    //
    // An earlier version fetched `limit * K` raw exchange rows and deduped
    // in Rust. That is a heuristic, not a guarantee: a session with more
    // than K matching turns still crowds every other session out of the
    // fetch window before the dedupe can see them. `GROUP BY ex.file_path`
    // with `MIN(fts.rank)` is exact — FTS5 rank is "smaller is better", and
    // SQLite guarantees that when `min()`/`max()` is used the bare columns
    // come from the row holding that extreme, so each group's columns are
    // the session's best-matching turn.
    let mut stmt = db.prepare(
        "SELECT ex.user_text, ex.assistant_text, ex.file_path, \
                s.project_path, s.session_id, ex.timestamp_ms, ex.snippet_text, \
                MIN(fts.rank) AS best_rank \
         FROM exchange_fts fts \
         JOIN exchanges ex ON ex.rowid = fts.rowid \
         JOIN sessions s   ON s.file_path = ex.file_path \
         WHERE exchange_fts MATCH ?1 \
         GROUP BY ex.file_path \
         ORDER BY best_rank \
         LIMIT ?2",
    )?;
    let raw = stmt.query_map(rusqlite::params![phrase, limit as i64], |row| {
        Ok(ExchangeRow {
            user_text: row.get(0)?,
            assistant_text: row.get(1)?,
            file_path: row.get(2)?,
            project_path: row.get(3)?,
            session_id: row.get(4)?,
            timestamp_ms: row.get(5)?,
            snippet_text: row.get(6)?,
        })
    })?;

    let mut hits = Vec::new();
    for r in raw {
        hits.push(build_index_hit(&needle, r?));
    }
    Ok(hits)
}

/// Infix substring search over indexed exchange text.
///
/// FTS5 matches tokens (and, with our trailing `*`, token prefixes) — it
/// cannot match *inside* a token, so `lock` will never find `deadlock`
/// there, though the raw scanner it replaced did. This closes that gap
/// with a `LIKE '%needle%'` over the same `exchanges` rows.
///
/// It is an unindexed scan, which is exactly why `search_cross_session`
/// only reaches for it when the FTS pass came up short — the case where
/// the infix gap is the plausible reason results are thin.
fn search_exchanges_infix(
    idx: &SessionIndex,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    if query.trim().len() < 2 {
        return Ok(Vec::new());
    }
    let needle = query.to_lowercase();
    let pattern = format!("%{}%", escape_like(&needle));

    let db = idx.db();
    let mut stmt = db.prepare(
        // `MAX(ex.timestamp_ms)` in the SELECT list, not just the ORDER BY:
        // SQLite only pins a group's bare columns to a specific row when a
        // min()/max() aggregate is present. Every row in a group already
        // satisfies the LIKE, so any of them is a real match — but this
        // makes *which* one deterministic (the session's newest matching
        // turn) instead of arbitrary.
        "SELECT ex.user_text, ex.assistant_text, ex.file_path, \
                s.project_path, s.session_id, ex.timestamp_ms, ex.snippet_text, \
                MAX(ex.timestamp_ms) AS newest \
         FROM exchanges ex \
         JOIN sessions s ON s.file_path = ex.file_path \
         WHERE lower(ex.user_text)      LIKE ?1 ESCAPE '\\' \
            OR lower(ex.assistant_text) LIKE ?1 ESCAPE '\\' \
         GROUP BY ex.file_path \
         ORDER BY newest DESC \
         LIMIT ?2",
    )?;
    let raw = stmt.query_map(rusqlite::params![pattern, limit as i64], |row| {
        Ok(ExchangeRow {
            user_text: row.get(0)?,
            assistant_text: row.get(1)?,
            file_path: row.get(2)?,
            project_path: row.get(3)?,
            session_id: row.get(4)?,
            timestamp_ms: row.get(5)?,
            snippet_text: row.get(6)?,
        })
    })?;

    let mut hits = Vec::new();
    for r in raw {
        hits.push(build_index_hit(&needle, r?));
    }
    Ok(hits)
}

/// Substring search over tool inputs and tool results.
///
/// `exchange_fts` indexes user/assistant text only, so a term that
/// appears solely in a Bash command or a tool result — which the old
/// JSONL scanner *did* match — would otherwise be invisible. This is a
/// `LIKE` scan over `tool_calls` (tens of thousands of rows, no index),
/// which is why `search_cross_session` calls it only when the cheaper
/// sources fall short of `limit`.
fn search_tool_calls(
    idx: &SessionIndex,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    if query.trim().len() < 2 {
        return Ok(Vec::new());
    }
    let needle = query.to_lowercase();
    let pattern = format!("%{}%", escape_like(&needle));

    let db = idx.db();
    let mut stmt = db.prepare(
        // MAX() in the SELECT list pins each group's bare columns to the
        // newest matching tool call (see `search_exchanges_infix`).
        "SELECT tc.tool_input_json, tc.tool_result_text, ex.file_path, \
                s.project_path, s.session_id, tc.timestamp_ms, \
                MAX(tc.timestamp_ms) AS newest \
         FROM tool_calls tc \
         JOIN exchanges ex ON ex.id = tc.exchange_id \
         JOIN sessions s   ON s.file_path = ex.file_path \
         WHERE lower(tc.tool_input_json)  LIKE ?1 ESCAPE '\\' \
            OR lower(tc.tool_result_text) LIKE ?1 ESCAPE '\\' \
         GROUP BY ex.file_path \
         ORDER BY newest DESC \
         LIMIT ?2",
    )?;
    let raw = stmt.query_map(rusqlite::params![pattern, limit as i64], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;

    let mut hits = Vec::new();
    for r in raw {
        let (input, result, file_path, project_path, session_id, ts_ms) = r?;
        // A tool_use input is the assistant's; a tool_result body comes
        // back on the user turn. Attribute to whichever actually matched.
        let (role, text) = match input
            .as_deref()
            .filter(|t| find_case_insensitive(t, &needle).is_some())
        {
            Some(t) => ("assistant", t.to_string()),
            None => match result.filter(|t| find_case_insensitive(t, &needle).is_some()) {
                Some(t) => ("user", t),
                // LIKE matched but the substring scan didn't — only
                // reachable if the two disagree on case folding. Skip
                // rather than emit a hit with no locatable match.
                None => continue,
            },
        };
        let Some((off, end)) = find_case_insensitive(&text, &needle) else {
            continue;
        };
        hits.push(SearchHit {
            session_id,
            slug: slug_from_file_path(&file_path),
            file_path: std::path::PathBuf::from(file_path),
            project_path,
            role: role.to_string(),
            snippet: redact_secrets(&make_snippet_chars(&text, off, needle_char_len(&needle))),
            match_offset: off,
            last_ts: ts_ms.and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis),
            score: classify_match(&text, off, end - off),
        });
    }
    Ok(hits)
}

/// Escape `%`, `_` and `\` for a LIKE pattern, paired with `ESCAPE '\'`
/// on the SQL side. Without this, a query containing `_` would silently
/// become a single-character wildcard.
fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Claude transcripts the exchange index does not cover: a `sessions`
/// row with no `exchanges` rows at all.
///
/// This is the honest completeness probe. The previous gate was "does
/// `exchanges` hold ANY row" — which flips the whole search onto the FTS
/// path the moment the backfill writes its first row, silently dropping
/// every transcript not yet indexed (mid-backfill, per-file failures, or
/// sessions created since the last run). Callers scan whatever this
/// returns, so a partial index costs a little speed instead of quietly
/// losing results.
pub fn claude_files_missing_exchanges(idx: &SessionIndex) -> Result<Vec<String>, rusqlite::Error> {
    let db = idx.db();
    let mut stmt = db.prepare(
        "SELECT s.file_path FROM sessions s \
         WHERE s.source_kind = 'claude_code' \
           AND NOT EXISTS (SELECT 1 FROM exchanges e WHERE e.file_path = s.file_path)",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// One `exchange_fts` match, joined to its `exchanges` + `sessions`
/// columns, before it's shaped into a `SearchHit`.
struct ExchangeRow {
    user_text: String,
    assistant_text: String,
    file_path: String,
    project_path: String,
    session_id: String,
    timestamp_ms: Option<i64>,
    /// Pre-redacted at rest (indexer::build_snippet). Used only as the
    /// snippet when neither turn yields an exact substring match.
    snippet_text: String,
}

/// Locate `needle` in a turn, trying the whole query first and then its
/// individual tokens.
///
/// The token pass exists because the FTS query is a *prefix* phrase: a
/// search for `rotat` legitimately matches the token `rotation`, and a
/// multi-word query matches a phrase whose tokens the raw string may not
/// reproduce verbatim. Without it the literal-substring lookup fails and
/// the caller is left guessing which turn actually matched.
fn locate_match(text: &str, needle_lower: &str) -> Option<(usize, usize)> {
    if let Some(span) = find_case_insensitive(text, needle_lower) {
        return Some(span);
    }
    needle_lower
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .find_map(|token| find_case_insensitive(text, token))
}

fn build_index_hit(needle_lower: &str, r: ExchangeRow) -> SearchHit {
    // Attribute the hit to the turn that actually contains the match —
    // user first (people search for what they typed), then assistant.
    //
    // The old fallback guessed `"user"` whenever `user_text` was
    // non-empty, which mislabels an assistant-side hit as the user's.
    // `locate_match` now also tries the query's tokens, so a prefix
    // match (`rotat` → `rotation`) resolves properly instead of falling
    // through. The final arm is reached only when FTS matched something
    // no substring pass can locate; there is genuinely nothing to
    // attribute, so report the pre-redacted exchange snippet and let the
    // role follow whichever turn has content.
    let (role, snippet, match_offset, score) =
        if let Some((off, end)) = locate_match(&r.user_text, needle_lower) {
            (
                "user",
                redact_secrets(&make_snippet_chars(
                    &r.user_text,
                    off,
                    needle_char_len(needle_lower),
                )),
                off,
                classify_match(&r.user_text, off, end - off),
            )
        } else if let Some((off, end)) = locate_match(&r.assistant_text, needle_lower) {
            (
                "assistant",
                redact_secrets(&make_snippet_chars(
                    &r.assistant_text,
                    off,
                    needle_char_len(needle_lower),
                )),
                off,
                classify_match(&r.assistant_text, off, end - off),
            )
        } else {
            let role = if r.assistant_text.trim().is_empty() {
                "user"
            } else {
                "assistant"
            };
            (role, r.snippet_text, 0usize, 0.4_f32)
        };

    SearchHit {
        session_id: r.session_id,
        slug: slug_from_file_path(&r.file_path),
        file_path: std::path::PathBuf::from(r.file_path),
        project_path: r.project_path,
        role: role.to_string(),
        snippet,
        match_offset,
        last_ts: r
            .timestamp_ms
            .and_then(|ms| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)),
        score,
    }
}

/// The project slug is the immediate parent directory of the
/// transcript file (CC stores `projects/<slug>/<session>.jsonl`).
fn slug_from_file_path(file_path: &str) -> String {
    std::path::Path::new(file_path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
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
        while k < src_byte_of_lower_byte.len() && src_byte_of_lower_byte[k] == last_contributing_src
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
        let event_type = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
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
                redact_secrets(&make_snippet_chars(text, off, needle_char_len(&needle))),
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
        let hits = search_rows(&[row("s", "-r", path, None, None)], "tauri", 10).unwrap();
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
        let hits = search_rows(&[row("s", "-r", path, None, None)], "tauri", 10).unwrap();
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
        let hits = search_rows(&[row("s", "-r", path, None, None)], "tauri", 10).unwrap();
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
                &[
                    r#"{"type":"user","message":{"role":"user","content":"widget search"},"sessionId":"s"}"#,
                ],
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
        let hits = search_rows(&[sub1, sub2, sub3, phrase], "auth", 3).unwrap();
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
            &[&format!(
                r#"{{"type":"user","message":{{"role":"user","content":"{}"}},"sessionId":"s"}}"#,
                text_only_phrase
            )],
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
            &[
                r#"{"type":"user","message":{"role":"user","content":"discuss auth today"},"sessionId":"p"}"#,
            ],
        );
        write_jsonl(
            &sub_path,
            &[
                r#"{"type":"user","message":{"role":"user","content":"unauthorized access"},"sessionId":"s"}"#,
            ],
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
        let older = row(
            "old",
            "-r",
            PathBuf::new(),
            Some("auth matters"),
            ts("2020-01-01T00:00:00Z"),
        );
        let newer = row(
            "new",
            "-r",
            PathBuf::new(),
            Some("auth matters"),
            ts("2026-04-10T10:00:00Z"),
        );
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

    // ─── FTS-backed search_index ───────────────────────────────

    /// Stage a Claude session under `<config>/projects/<slug>/`, then
    /// run the `sessions` refresh + exchange backfill so `exchange_fts`
    /// is populated. Returns the opened index and the config dir.
    fn stage_indexed_session(
        tmp: &TempDir,
        slug: &str,
        session_id: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> (SessionIndex, PathBuf) {
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let config = tmp.path().join("claude");
        let dir = config.join("projects").join(slug);
        fs::create_dir_all(&dir).unwrap();
        let body = format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{user_text}"}}]}},"timestamp":"2026-05-15T11:30:00.000Z","sessionId":"{session_id}","cwd":"/proj"}}
{{"type":"assistant","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"{assistant_text}"}}]}},"timestamp":"2026-05-15T11:30:01.000Z","sessionId":"{session_id}","cwd":"/proj"}}
"#,
        );
        fs::write(dir.join(format!("{session_id}.jsonl")), body).unwrap();
        idx.refresh(&config).unwrap();
        crate::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &config).unwrap();
        (idx, config)
    }

    #[test]
    fn claude_files_missing_exchanges_reflects_backfill() {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let config = tmp.path().join("claude");
        let dir = config.join("projects").join("-repo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("sid.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello there"}]},"timestamp":"2026-05-15T11:30:00.000Z","sessionId":"sid","cwd":"/proj"}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"general kenobi"}]},"timestamp":"2026-05-15T11:30:01.000Z","sessionId":"sid","cwd":"/proj"}
"#,
        )
        .unwrap();
        idx.refresh(&config).unwrap();
        // `sessions` knows the file; `exchanges` doesn't cover it yet.
        assert_eq!(claude_files_missing_exchanges(&idx).unwrap().len(), 1);
        crate::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &config).unwrap();
        assert!(claude_files_missing_exchanges(&idx).unwrap().is_empty());
    }

    #[test]
    fn search_cross_session_finds_sessions_the_index_has_not_covered() {
        // The High finding: the old gate was "does `exchanges` hold ANY
        // row", so the moment the backfill wrote its first row the whole
        // search flipped to FTS and every not-yet-indexed transcript went
        // silently invisible. Here one session IS indexed and a second is
        // not; both must still be findable.
        let tmp = TempDir::new().unwrap();
        let (idx, config) =
            stage_indexed_session(&tmp, "-repo", "indexed", "the indexed widget", "ok");

        // Add a second transcript and refresh `sessions` — but do NOT
        // backfill, so it has no `exchanges` rows.
        let dir = config.join("projects").join("-repo");
        fs::write(
            dir.join("orphan.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"the orphan widget"}]},"timestamp":"2026-05-15T12:00:00.000Z","sessionId":"orphan","cwd":"/proj"}
"#,
        )
        .unwrap();
        idx.refresh(&config).unwrap();
        assert_eq!(claude_files_missing_exchanges(&idx).unwrap().len(), 1);

        let hits = search_cross_session(Some(&idx), &config, "widget", 10).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.session_id.as_str()).collect();
        assert!(ids.contains(&"indexed"), "indexed session missing: {ids:?}");
        assert!(
            ids.contains(&"orphan"),
            "un-indexed session must still be found: {ids:?}"
        );
    }

    #[test]
    fn search_cross_session_returns_at_most_one_hit_per_session() {
        // `search_rows` yields one hit per session. A plain LIMIT over
        // `exchanges` regressed that: one chatty session could fill every
        // palette slot and hide all the others.
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let config = tmp.path().join("claude");
        let dir = config.join("projects").join("-repo");
        fs::create_dir_all(&dir).unwrap();

        // One session, five separate turns all matching "widget".
        let mut chatty = String::new();
        for i in 0..5 {
            chatty.push_str(&format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"widget question {i}"}}]}},"timestamp":"2026-05-15T11:3{i}:00.000Z","sessionId":"chatty","cwd":"/proj"}}
"#,
            ));
        }
        fs::write(dir.join("chatty.jsonl"), chatty).unwrap();
        fs::write(
            dir.join("other.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"widget elsewhere"}]},"timestamp":"2026-05-15T13:00:00.000Z","sessionId":"other","cwd":"/proj"}
"#,
        )
        .unwrap();
        idx.refresh(&config).unwrap();
        crate::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &config).unwrap();

        let hits = search_cross_session(Some(&idx), &config, "widget", 5).unwrap();
        let chatty_hits = hits.iter().filter(|h| h.session_id == "chatty").count();
        assert_eq!(chatty_hits, 1, "one hit per session, got {chatty_hits}");
        assert!(
            hits.iter().any(|h| h.session_id == "other"),
            "the quiet session must not be crowded out: {:?}",
            hits.iter().map(|h| &h.session_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_index_matches_a_word_prefix() {
        // FTS5 matches whole tokens; the raw substring scanner it replaced
        // matched partial words. Without a prefix query, typing `rotat`
        // would find nothing while `rotation` sat right there.
        let tmp = TempDir::new().unwrap();
        let (idx, _) =
            stage_indexed_session(&tmp, "-repo", "sid", "please fix the rotation", "done");
        let hits = search_index(&idx, "rotat", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "user");
        assert!(hits[0].snippet.contains("rotation"));
    }

    #[test]
    fn search_index_attributes_an_assistant_hit_to_the_assistant() {
        // The old fallback reported `"user"` whenever `user_text` was
        // non-empty, mislabeling assistant-side hits.
        let tmp = TempDir::new().unwrap();
        let (idx, _) = stage_indexed_session(
            &tmp,
            "-repo",
            "sid",
            "a user turn with unrelated words",
            "the mutex ordering is inverted",
        );
        let hits = search_index(&idx, "mutex", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "assistant");
    }

    #[test]
    fn search_cross_session_finds_text_that_only_exists_in_tool_io() {
        // `exchange_fts` covers user/assistant text only. The old scanner
        // also matched tool_use inputs and tool_result bodies; dropping
        // that would make e.g. a Bash command unfindable.
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let config = tmp.path().join("claude");
        let dir = config.join("projects").join("-repo");
        fs::create_dir_all(&dir).unwrap();
        // "zebrafish" appears ONLY inside the tool_use input.
        fs::write(
            dir.join("tools.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"run the thing"}]},"timestamp":"2026-05-15T11:30:00.000Z","sessionId":"tools","cwd":"/proj"}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"./zebrafish --migrate"}}]},"timestamp":"2026-05-15T11:30:01.000Z","sessionId":"tools","cwd":"/proj"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"ok","is_error":false}]},"timestamp":"2026-05-15T11:30:02.000Z","sessionId":"tools","cwd":"/proj"}
"#,
        )
        .unwrap();
        idx.refresh(&config).unwrap();
        crate::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &config).unwrap();
        // Nothing is missing from the index — so the ONLY way this hit
        // can surface is the tool_calls pass.
        assert!(claude_files_missing_exchanges(&idx).unwrap().is_empty());

        let hits = search_cross_session(Some(&idx), &config, "zebrafish", 10).unwrap();
        assert_eq!(hits.len(), 1, "tool-only match must still be findable");
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].snippet.contains("zebrafish"));
    }

    #[test]
    fn search_cross_session_honors_zero_limit_and_short_query() {
        // `search_rows` returns nothing for a zero limit; `search_index`
        // coerces limit to >= 1 internally, and that must not leak out.
        let tmp = TempDir::new().unwrap();
        let (idx, config) = stage_indexed_session(&tmp, "-repo", "sid", "widget here", "ok");
        assert!(search_cross_session(Some(&idx), &config, "widget", 0)
            .unwrap()
            .is_empty());
        assert!(search_cross_session(Some(&idx), &config, "w", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_cross_session_finds_an_infix_match_fts_cannot() {
        // FTS5 matches tokens (and token prefixes), never inside a token:
        // `lock` will not find `deadlock` there. The scanner it replaced
        // did. The infix LIKE pass restores it.
        let tmp = TempDir::new().unwrap();
        let (idx, config) =
            stage_indexed_session(&tmp, "-repo", "sid", "hit the deadlock again", "yes");

        // Prove the premise: the FTS pass alone genuinely misses it.
        assert!(
            search_index(&idx, "lock", 10).unwrap().is_empty(),
            "premise broken: FTS should not match inside a token"
        );
        // The public entry point must still find it.
        let hits = search_cross_session(Some(&idx), &config, "lock", 10).unwrap();
        assert_eq!(hits.len(), 1, "infix match must survive");
        assert!(hits[0].snippet.contains("deadlock"));
    }

    #[test]
    fn search_cross_session_finds_a_transcript_written_after_the_backfill() {
        // A brand-new file is in neither `exchanges` NOR `sessions`, so it
        // is invisible to the FTS pass *and* to the missing-exchanges
        // probe (which can only see what `sessions` knows). Only a refresh
        // before querying surfaces it — which is what the old command got
        // for free by going through `list_all_sessions`.
        let tmp = TempDir::new().unwrap();
        let (idx, config) = stage_indexed_session(&tmp, "-repo", "old", "the old thing", "ok");

        // Written after the index + backfill ran. Nothing refreshes it.
        fs::write(
            config.join("projects").join("-repo").join("brandnew.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"the brandnew thing"}]},"timestamp":"2026-05-15T14:00:00.000Z","sessionId":"brandnew","cwd":"/proj"}
"#,
        )
        .unwrap();

        let hits = search_cross_session(Some(&idx), &config, "brandnew", 10).unwrap();
        assert_eq!(
            hits.len(),
            1,
            "a session written since the last backfill must still be findable"
        );
        assert_eq!(hits[0].session_id, "brandnew");
    }

    #[test]
    fn search_cross_session_dedupes_even_a_very_chatty_session() {
        // The overfetch-then-dedupe heuristic broke down once a single
        // session had more matching turns than the overfetch window. The
        // per-session collapse now happens in SQL, so it holds at any size.
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let config = tmp.path().join("claude");
        let dir = config.join("projects").join("-repo");
        fs::create_dir_all(&dir).unwrap();

        // 40 matching turns in ONE session — far past any overfetch window.
        let mut chatty = String::new();
        for i in 0..40 {
            chatty.push_str(&format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"widget turn {i}"}}]}},"timestamp":"2026-05-15T11:00:00.000Z","sessionId":"chatty","cwd":"/proj"}}
"#,
            ));
        }
        fs::write(dir.join("chatty.jsonl"), chatty).unwrap();
        fs::write(
            dir.join("quiet.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"widget quiet"}]},"timestamp":"2026-05-15T13:00:00.000Z","sessionId":"quiet","cwd":"/proj"}
"#,
        )
        .unwrap();
        idx.refresh(&config).unwrap();
        crate::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &config).unwrap();

        let hits = search_cross_session(Some(&idx), &config, "widget", 2).unwrap();
        assert_eq!(hits.iter().filter(|h| h.session_id == "chatty").count(), 1);
        assert!(
            hits.iter().any(|h| h.session_id == "quiet"),
            "the quiet session must not be crowded out by 40 chatty turns"
        );
    }

    #[test]
    fn search_cross_session_without_an_index_scans_without_touching_global_state() {
        // Index failed to open → slow, but never wrong.
        //
        // This path MUST go through `scan_all_sessions_uncached`. An
        // earlier version called `list_all_sessions`, which opens a
        // `SessionIndex` at the *global* data dir (`~/.claudepot/
        // sessions.db`) regardless of the `config_dir` argument — so
        // running this very test refreshed the developer's real session
        // cache against a temp config dir and pruned it to a single row.
        // The assertion below only proves the search works; the guard
        // against that regression is that the no-index arm no longer
        // names `list_all_sessions` at all.
        let tmp = TempDir::new().unwrap();
        let (_, config) = stage_indexed_session(&tmp, "-repo", "sid", "the widget lives", "ok");
        let hits = search_cross_session(None, &config, "widget", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "sid");
    }

    #[test]
    fn search_index_finds_user_match_with_user_role() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = stage_indexed_session(
            &tmp,
            "-repo",
            "sid",
            "investigate the deadlock",
            "mutex B wins",
        );
        let hits = search_index(&idx, "deadlock", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "user");
        assert!(hits[0].snippet.contains("deadlock"));
        assert_eq!(hits[0].session_id, "sid");
        assert_eq!(hits[0].slug, "-repo");
    }

    #[test]
    fn search_index_finds_assistant_match_with_assistant_role() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = stage_indexed_session(
            &tmp,
            "-repo",
            "sid",
            "any clue",
            "the deadlock is in mutex B",
        );
        let hits = search_index(&idx, "mutex", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].snippet.contains("mutex"));
    }

    #[test]
    fn search_index_short_query_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = stage_indexed_session(&tmp, "-repo", "sid", "hello", "world");
        assert!(search_index(&idx, "a", 10).unwrap().is_empty());
    }

    #[test]
    fn search_cross_session_respects_limit() {
        // The cap belongs to `search_cross_session`. `search_index`
        // deliberately OVERFETCHES (it must, so the per-session dedupe
        // still has candidates from other sessions to fall back on), so
        // the limit is only meaningful once at the public boundary.
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let config = tmp.path().join("claude");
        let dir = config.join("projects").join("-repo");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            let body = format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"widget number {i}"}}]}},"timestamp":"2026-05-15T11:30:0{i}.000Z","sessionId":"s{i}","cwd":"/proj"}}
"#,
            );
            fs::write(dir.join(format!("s{i}.jsonl")), body).unwrap();
        }
        idx.refresh(&config).unwrap();
        crate::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &config).unwrap();
        let hits = search_cross_session(Some(&idx), &config, "widget", 3).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn search_index_redacts_tokens_in_snippet() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = stage_indexed_session(
            &tmp,
            "-repo",
            "sid",
            "the leaked key sk-ant-oat01-AbcdWxYz0000 keep looking",
            "noted",
        );
        let hits = search_index(&idx, "keep", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].snippet.contains("sk-ant-oat01-AbcdWxYz0000"));
    }

    #[test]
    fn search_index_does_not_error_on_fts_operator_input() {
        let tmp = TempDir::new().unwrap();
        let (idx, _) = stage_indexed_session(&tmp, "-repo", "sid", "hello there", "general kenobi");
        // None of these should raise an FTS5 MATCH syntax error — the
        // query is phrase-escaped before it reaches MATCH.
        for adversarial in ["NEAR", "AND OR", "-", "*", "(", ":", r#"a "b" c"#] {
            assert!(search_index(&idx, adversarial, 10).is_ok());
        }
    }
}
