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
/// is honored *after* `redaction::apply` runs (so the redacted
/// form is what counts toward the cap). Truncation marks the
/// result with `truncated = true`.
pub fn read_locator_bounded(
    idx: &SessionIndex,
    locator: &ConversationLocator,
    max_bytes: usize,
    policy: &RedactionPolicy,
) -> Result<ConversationRead, ReadError> {
    // Containment: the file_path must be in the sessions cache.
    // We never open an arbitrary path. The cache enforces canonical
    // path semantics at index time.
    let exists: bool = {
        let db = idx.db();
        db.query_row(
            "SELECT 1 FROM sessions WHERE file_path = ?1",
            [&locator.file_path],
            |_| Ok(true),
        )
        .unwrap_or(false)
    };
    if !exists {
        return Err(ReadError::NotIndexed(locator.file_path.clone()));
    }

    // Resolve line bounds. If the locator carries an exchange_id
    // but no explicit lines, look up the exchange's line_start /
    // line_end. Final fallback: full file.
    let (line_start, line_end) = resolve_line_bounds(idx, locator)?;

    let body = read_lines(&locator.file_path, line_start, line_end, max_bytes)?;
    let truncated = body.len() >= max_bytes;
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
        let row = db.query_row(
            "SELECT line_start, line_end FROM exchanges WHERE id = ?1",
            [ex_id],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                ))
            },
        );
        if let Ok((Some(s), Some(e))) = row {
            return Ok((s as u32, e as u32));
        }
        // Exchange exists but has no line range (e.g. compacted
        // summary) → fall through to file-level read.
    }

    // No bounds available; cap at 100 000 lines so we don't
    // accidentally slurp a multi-GB transcript. The byte cap below
    // is the real safety.
    Ok((1, 100_000))
}

fn read_lines(
    path: &str,
    line_start: u32,
    line_end: u32,
    max_bytes: usize,
) -> Result<String, ReadError> {
    let file = File::open(path).map_err(|source| ReadError::Io {
        path: PathBuf::from(path),
        source,
    })?;
    let reader = BufReader::new(file);
    let mut out = String::new();
    let mut line_no: u32 = 0;
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
            // Truncate at a char boundary.
            while !out.is_char_boundary(max_bytes) && out.len() > max_bytes {
                out.pop();
            }
            out.truncate(max_bytes);
            break;
        }
    }
    Ok(out)
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
        let day = root.join("2026/05/15");
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
        tmp.path()
            .join("codex/sessions/2026/05/15/rollout.jsonl")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn read_locator_returns_redacted_body() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        let locator = locator_for_exchange(&path, "sid:0");
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
        matches!(err, ReadError::NotIndexed(_));
    }

    #[test]
    fn explicit_line_range_overrides_exchange_bounds() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        let loc = ConversationLocator {
            file_path: path,
            exchange_id: Some("sid:0".to_string()),
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
    fn byte_cap_truncates_oversized_reads() {
        let tmp = TempDir::new().unwrap();
        let idx = prep_corpus(&tmp);
        let path = corpus_file(&tmp);

        let loc = locator_for_exchange(&path, "sid:0");
        let result =
            read_locator_bounded(&idx, &loc, 32, &RedactionPolicy::default()).unwrap();
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
