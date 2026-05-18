//! Read a transcript excerpt by locator (WI-005).
//!
//! A *locator* is the addressable identity of a hit returned by
//! `search`. The locator MUST resolve to a `file_path` that exists
//! in the v4 `sessions` table — that's the trust boundary. We
//! never accept a raw user-supplied path; the indexer's promise
//! that `sessions.file_path` is canonical, locked to disk, and
//! 0600-cohort is what makes this safe.
//!
//! Two read shapes:
//!   * `read_locator` — full-window read of the exchange's line
//!     range (or the file if line range is unavailable).
//!   * `read_locator_lines` — explicit line range with a hard
//!     byte cap.
//!
//! Both apply `redaction::apply` to the returned body before
//! emission.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::redaction::{apply as redact_apply, RedactionPolicy};
use crate::session_index::SessionIndex;

/// Identifies an exchange. Constructed from a `SearchHit`'s
/// `(exchange_id, file_path, line_start, line_end)`. The caller
/// can also build one by hand for an MCP `read_conversation` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationLocator {
    /// `sessions.file_path`. Must match exactly — we look it up
    /// in the cache before opening the file.
    pub file_path: String,
    /// `<session_id>:<turn_index>`; identifies which exchange the
    /// caller is asking for. Optional: if absent, the read is
    /// file-level.
    pub exchange_id: Option<String>,
    /// 1-based physical line bounds. Optional; defaults to the
    /// entire file when neither bound is set.
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
}

/// Result of one read. Body is already redacted per the supplied
/// `RedactionPolicy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRead {
    pub file_path: String,
    pub exchange_id: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub body: String,
    /// True when the body was truncated by `max_bytes`.
    pub truncated: bool,
}

/// Errors specific to locator reading. Mostly distinct from
/// `SessionIndexError` because the failure modes here are about
/// *what the locator points to*, not the DB itself.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("locator references unknown file_path: {0}")]
    NotIndexed(String),

    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// Read the locator with default byte cap (64 KiB).
pub fn read_locator(
    idx: &SessionIndex,
    locator: &ConversationLocator,
    policy: &RedactionPolicy,
) -> Result<ConversationRead, ReadError> {
    read_locator_bounded(idx, locator, 64 * 1024, policy)
}

/// Read the locator with a caller-specified byte cap. `max_bytes`
/// caps the **pre-redaction** read (it's the I/O safety knob: we
/// won't slurp more than `max_bytes` from disk regardless of what
/// redaction does to the byte count). The returned `body` is the
/// post-redaction form, which may be shorter (a masked secret) or
/// the same length; `truncated` reflects whether the pre-redaction
/// read hit the cap.
///
/// Containment: `locator.file_path` must exist in the v4
/// `sessions` table. We never open an arbitrary path — the cache
/// is the trust boundary. Errors are categorized so callers can
/// distinguish unknown path from SQL error from I/O error.
pub fn read_locator_bounded(
    idx: &SessionIndex,
    locator: &ConversationLocator,
    max_bytes: usize,
    policy: &RedactionPolicy,
) -> Result<ConversationRead, ReadError> {
    // Containment: the file_path must be in the sessions cache.
    // We never open an arbitrary path. Distinguish the legitimate
    // "row not found" case from other SQL errors (locked DB, FTS
    // corruption, etc.) so the caller doesn't conflate them.
    {
        let db = idx.db();
        match db.query_row(
            "SELECT 1 FROM sessions WHERE file_path = ?1",
            [&locator.file_path],
            |_| Ok(true),
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(ReadError::NotIndexed(locator.file_path.clone()));
            }
            Err(e) => return Err(ReadError::Sql(e)),
        }
    }

    // Resolve line bounds. If the locator carries an exchange_id
    // but no explicit lines, look up the exchange's line_start /
    // line_end. Final fallback: full file.
    let (line_start, line_end) = resolve_line_bounds(idx, locator)?;

    let (body, truncated) = read_lines(&locator.file_path, line_start, line_end, max_bytes)?;
    let redacted = redact_apply(&body, policy);
    Ok(ConversationRead {
        file_path: locator.file_path.clone(),
        exchange_id: locator.exchange_id.clone(),
        line_start,
        line_end,
        body: redacted,
        truncated,
    })
}

fn resolve_line_bounds(
    idx: &SessionIndex,
    loc: &ConversationLocator,
) -> Result<(u32, u32), ReadError> {
    // Explicit bounds win.
    if let (Some(s), Some(e)) = (loc.line_start, loc.line_end) {
        return Ok((s.max(1), e.max(s)));
    }

    if let Some(ref ex_id) = loc.exchange_id {
        let db = idx.db();
        // Constrain the lookup to the locator's file_path so a
        // mismatched exchange_id (different file, hand-crafted,
        // or stale) doesn't silently widen the read to a full-
        // file scan. If the (id, file_path) pair doesn't match,
        // return an error rather than fall through.
        let row = db.query_row(
            "SELECT line_start, line_end FROM exchanges WHERE id = ?1 AND file_path = ?2",
            rusqlite::params![ex_id, &loc.file_path],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        );
        match row {
            Ok((Some(s), Some(e))) => return Ok((s as u32, e as u32)),
            Ok((_, _)) => {
                // Exchange exists, matches file_path, but has no
                // line range (e.g. compacted summary) → fall
                // through to file-level read. Acceptable.
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // exchange_id was supplied but doesn't belong to
                // this file. Refuse rather than widen.
                return Err(ReadError::NotIndexed(format!(
                    "exchange {ex_id} not found under {fp}",
                    fp = loc.file_path
                )));
            }
            Err(e) => return Err(ReadError::Sql(e)),
        }
    }

    // No bounds available; cap at 100 000 lines so we don't
    // accidentally slurp a multi-GB transcript. The byte cap in
    // `read_lines` is the real safety; if both ceilings fire the
    // byte cap wins (it's nearly always reached first). L8 — log
    // when this fallback kicks in so an operator chasing
    // "why does my read stop at line 100000" finds the answer.
    tracing::debug!(
        ?loc.file_path,
        "read_locator: no line bounds; falling back to file-level read (capped at 100 000 lines / max_bytes)"
    );
    Ok((1, 100_000))
}

/// Read `path`'s lines `[line_start..=line_end]` (1-based) up to
/// a byte cap. Returns `(body, truncated)` where `truncated` is
/// true iff the byte cap was the reason iteration stopped. An
/// exactly-fitting read (cap not hit, last requested line ends at
/// or below cap) returns `truncated=false`.
fn read_lines(
    path: &str,
    line_start: u32,
    line_end: u32,
    max_bytes: usize,
) -> Result<(String, bool), ReadError> {
    let file = File::open(path).map_err(|source| ReadError::Io {
        path: PathBuf::from(path),
        source,
    })?;
    let reader = BufReader::new(file);
    let mut out = String::new();
    let mut line_no: u32 = 0;
    let mut truncated = false;
    for line in reader.lines() {
        line_no += 1;
        if line_no < line_start {
            continue;
        }
        if line_no > line_end {
            break;
        }
        let line = match line {
            Ok(s) => s,
            Err(e) => {
                return Err(ReadError::Io {
                    path: PathBuf::from(path),
                    source: e,
                });
            }
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
        if out.len() >= max_bytes {
            // Cap hit. Find the largest byte index ≤ max_bytes
            // that is a char boundary and truncate there. Walks
            // down at most 3 bytes (the max length of a UTF-8
            // codepoint minus 1). Always lands on a boundary, so
            // `String::truncate` is safe. The previous form did
            // `pop()` in a loop then `truncate(max_bytes)`, which
            // was subtle: `truncate` is a no-op when its arg
            // exceeds `len`, so the function was already
            // panic-free, but the resulting length could be off
            // by up to 3 bytes from what the comment claimed.
            let mut cut = max_bytes.min(out.len());
            while cut > 0 && !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            truncated = true;
            break;
        }
    }
    Ok((out, truncated))
}

// ─── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_memory::indexer::backfill_codex;
    use std::fs;
    use tempfile::TempDir;

    fn prep_corpus(tmp: &TempDir) -> SessionIndex {
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let root = tmp.path().join("codex").join("sessions");
        // Build the date path with chained .join() so each component
        // uses the native separator. A single literal "2026/05/15"
        // works on Unix (forward slashes are real separators) but
        // becomes one filename component on Windows, leaving the
        // corpus directory and the lookup path with mismatched
        // separators — `NotIndexed` at read time.
        let day = root.join("2026").join("05").join("15");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout.jsonl"),
            r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"sid","cwd":"/proj","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"my key sk-ant-oat01-VeryLongSecretValueHere"}]}}
{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"do not paste secrets in logs"}]}}
"#,
        )
        .unwrap();
        backfill_codex(&idx, &root).unwrap();
        idx
    }

    fn locator_for_exchange(file_path: &str, exchange_id: &str) -> ConversationLocator {
        ConversationLocator {
            file_path: file_path.to_string(),
            exchange_id: Some(exchange_id.to_string()),
            line_start: None,
            line_end: None,
        }
    }

    fn corpus_file(tmp: &TempDir) -> String {
        // Match prep_corpus's chained .join() so the lookup path
        // matches what the indexer stored. On Windows, a single
        // literal "codex/sessions/2026/05/15/rollout.jsonl" lands
        // as one filename component instead of a path, producing
        // mixed separators that NotIndexed against the canonical
        // backslash form.
        tmp.path()
            .join("codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("15")
            .join("rollout.jsonl")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn read_locator_returns_redacted_body() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        let locator = locator_for_exchange(&path, "codex:sid:0");
        let result = read_locator(&idx, &locator, &RedactionPolicy::default()).unwrap();

        // Body should NOT contain the literal secret.
        assert!(
            !result.body.contains("sk-ant-oat01-VeryLongSecretValueHere"),
            "redaction must strip the secret, got: {}",
            result.body
        );
        // Body SHOULD contain other text from the lines.
        assert!(result.body.contains("do not paste secrets in logs"));
        assert!(!result.truncated);
        assert!(result.line_start >= 1);
        assert!(result.line_end >= result.line_start);
    }

    #[test]
    fn read_locator_rejects_unknown_file_path() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let loc = ConversationLocator {
            file_path: "/etc/passwd".to_string(),
            exchange_id: None,
            line_start: None,
            line_end: None,
        };
        let err = read_locator(&idx, &loc, &RedactionPolicy::default()).unwrap_err();
        assert!(
            matches!(err, ReadError::NotIndexed(_)),
            "expected NotIndexed, got {err:?}"
        );
    }

    #[test]
    fn explicit_line_range_overrides_exchange_bounds() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        let loc = ConversationLocator {
            file_path: path,
            exchange_id: Some("codex:sid:0".to_string()),
            line_start: Some(1),
            line_end: Some(1),
        };
        let result = read_locator(&idx, &loc, &RedactionPolicy::default()).unwrap();
        assert_eq!(result.line_start, 1);
        assert_eq!(result.line_end, 1);
        // Line 1 is the session_meta — contains "session_meta"
        // verbatim because it's a JSON token, not a secret.
        assert!(result.body.contains("session_meta"));
    }

    #[test]
    fn exactly_fitting_read_is_not_flagged_truncated() {
        // M1 — a read that exactly equals max_bytes should NOT be
        // marked truncated. The pre-fix `>=` comparison would
        // false-positive at the boundary; the post-fix
        // `read_lines` returns an explicit signal that's only
        // true when the cap actually stopped iteration.
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        // Read with a cap large enough to fit everything.
        let loc = locator_for_exchange(&path, "codex:sid:0");
        let result = read_locator_bounded(
            &idx,
            &loc,
            10 * 1024, // generous cap
            &RedactionPolicy::default(),
        )
        .unwrap();
        assert!(
            !result.truncated,
            "fitting read should NOT be flagged truncated, body.len={} body={:?}",
            result.body.len(),
            result.body
        );
    }

    #[test]
    fn multibyte_truncation_lands_on_char_boundary() {
        // Codex audit M-correctness — verify the truncation logic
        // never produces an invalid UTF-8 string when max_bytes
        // lands mid-multi-byte-char. The earlier version of this
        // test used caps too small to reach the CJK content;
        // this version unit-tests `read_lines` directly with
        // caps that fall on CJK-byte positions.
        //
        // Strategy: write a single line of pure CJK (3 bytes per
        // codepoint) into a temp file, then call read_lines with
        // every cap from 0 to 12. With each codepoint at 3-byte
        // boundary (3, 6, 9, 12), only those caps yield clean
        // boundaries; caps in between (1, 2, 4, 5, 7, 8, 10, 11)
        // would crash a naive byte-truncate but our code walks
        // down to the nearest boundary.
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("cjk.txt");
        // "中文回应" = 4 × 3-byte codepoints = 12 bytes total
        std::fs::write(&p, "中文回应").unwrap();

        let path_str = p.to_string_lossy().into_owned();
        for cap in 0..=14usize {
            let (body, truncated) = super::read_lines(&path_str, 1, 1, cap).expect("read");
            // Validity: must be valid UTF-8 (String guarantees).
            // Must never exceed the cap (post-truncation length
            // is ≤ cap).
            assert!(
                body.len() <= cap.min(12),
                "cap={cap} produced body of {} bytes (raw is 12)",
                body.len()
            );
            // Char boundary: every prefix of "中文回应" up to a
            // codepoint boundary is a valid UTF-8 string. The
            // truncate logic must land on byte 0, 3, 6, 9, or 12
            // — never 1, 2, 4, 5, 7, 8, 10, 11.
            assert!(
                body.len() % 3 == 0,
                "cap={cap}: body.len()={} not on CJK codepoint boundary",
                body.len()
            );
            // Truncation signal: true iff the cap forced a stop
            // before the line ended (i.e. raw line > cap). The
            // raw line is 12 bytes.
            if cap < 12 {
                assert!(truncated, "cap={cap}: should be truncated (raw=12)");
            }
        }
    }

    #[test]
    fn invalid_exchange_id_returns_not_indexed() {
        // Codex audit M-security — an exchange_id that doesn't
        // belong to the locator's file_path must NOT silently
        // widen to a file-level read.
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);
        let loc = ConversationLocator {
            file_path: path,
            exchange_id: Some("not-a-real-id:0".to_string()),
            line_start: None,
            line_end: None,
        };
        let err = read_locator(&idx, &loc, &RedactionPolicy::default())
            .expect_err("must refuse mismatched exchange_id");
        assert!(
            matches!(err, ReadError::NotIndexed(_)),
            "expected NotIndexed for mismatched exchange_id, got {err:?}"
        );
    }

    #[test]
    fn byte_cap_truncates_oversized_reads() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        let loc = locator_for_exchange(&path, "codex:sid:0");
        let result = read_locator_bounded(&idx, &loc, 32, &RedactionPolicy::default()).unwrap();
        assert!(result.truncated, "32-byte cap should truncate");
        assert!(result.body.len() <= 32);
    }

    #[test]
    fn file_level_read_when_exchange_has_no_line_range() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);
        // Locator with no exchange_id and no explicit lines → full
        // file (capped at the 100k-line ceiling, plus byte cap).
        let loc = ConversationLocator {
            file_path: path,
            exchange_id: None,
            line_start: None,
            line_end: None,
        };
        let result = read_locator(&idx, &loc, &RedactionPolicy::default()).unwrap();
        assert!(result.body.contains("session_meta"));
        assert!(result.body.contains("do not paste secrets"));
    }
}
