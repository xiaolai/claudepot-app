//! Per-turn token-usage persistence — writes and reads against the
//! `session_turns` table introduced alongside the per-turn extractor
//! in `session.rs`. Kept separate from `codec.rs` so the per-session
//! aggregate path stays focused on its own UPSERT pipeline; the
//! turn-row replace-all semantics are a different shape and mixing
//! the two in one file makes both harder to reason about.
//!
//! The two public-to-the-crate entry points mirror the `codec.rs`
//! pattern: a writer (`replace_turns`) called inside the refresh
//! transaction, and a reader (`load_turns`) used by consumer
//! surfaces.

use crate::session::{TokenUsage, TurnRecord};
use rusqlite::{params, Connection};

use super::SessionIndexError;

/// Replace every per-turn row for `file_path` with the freshly-scanned
/// `turns`. Called inside the same transaction as the session
/// aggregate upsert so the cache stays internally consistent: either
/// both the session row and its per-turn detail update, or neither
/// does.
///
/// Each `user_prompt_preview` is independently `sk-ant-`-redacted at
/// write time. The per-row redaction matters because a transcript
/// can paste a token mid-conversation and `SessionRow::first_user_prompt`
/// — which is what `codec::redact_secrets` operates on — only
/// captures the first user message in the file.
pub(super) fn replace_turns(
    db: &Connection,
    file_path: &str,
    turns: &[TurnRecord],
) -> Result<(), SessionIndexError> {
    db.execute(
        "DELETE FROM session_turns WHERE file_path = ?1",
        params![file_path],
    )?;
    let mut stmt = db.prepare_cached(SQL_INSERT_TURN)?;
    for t in turns {
        let preview = t
            .user_prompt_preview
            .as_deref()
            .map(super::codec::redact_secrets_for_turns);
        stmt.execute(params![
            file_path,
            t.turn_index as i64,
            t.ts_ms,
            t.model,
            i64::try_from(t.tokens.input).unwrap_or(i64::MAX),
            i64::try_from(t.tokens.output).unwrap_or(i64::MAX),
            i64::try_from(t.tokens.cache_creation).unwrap_or(i64::MAX),
            i64::try_from(t.tokens.cache_read).unwrap_or(i64::MAX),
            preview,
        ])?;
    }
    Ok(())
}

/// Drop every per-turn row for `file_path`. Called by `codec::delete_row`
/// when a transcript vanishes from disk so orphaned turn rows don't
/// pile up across the cache's lifetime. Pulled out here so the
/// cascade rule lives next to the rest of the turn-table writes.
pub(super) fn delete_turns_for_file(
    db: &Connection,
    file_path: &str,
) -> Result<(), SessionIndexError> {
    db.execute(
        "DELETE FROM session_turns WHERE file_path = ?1",
        params![file_path],
    )?;
    Ok(())
}

/// Load every persisted turn for one file, ordered by `turn_index`.
/// Empty result is normal — sessions on disk before this table
/// existed have no turns until their next re-scan.
pub fn load_turns(
    db: &Connection,
    file_path: &str,
) -> Result<Vec<TurnRecord>, SessionIndexError> {
    let mut stmt = db.prepare(SQL_SELECT_TURNS_BY_FILE)?;
    let rows = stmt.query_map(params![file_path], |r| {
        Ok(TurnRecord {
            turn_index: usize::try_from(r.get::<_, i64>("turn_index")?).unwrap_or(0),
            ts_ms: r.get("ts_ms")?,
            model: r.get("model")?,
            tokens: TokenUsage {
                input: u64::try_from(r.get::<_, i64>("tokens_input")?).unwrap_or(0),
                output: u64::try_from(r.get::<_, i64>("tokens_output")?).unwrap_or(0),
                cache_creation: u64::try_from(r.get::<_, i64>("tokens_cache_creation")?)
                    .unwrap_or(0),
                cache_read: u64::try_from(r.get::<_, i64>("tokens_cache_read")?).unwrap_or(0),
            },
            user_prompt_preview: r.get("user_prompt_preview")?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

const SQL_INSERT_TURN: &str = r#"
INSERT INTO session_turns (
    file_path, turn_index, ts_ms, model,
    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
    user_prompt_preview
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
"#;

const SQL_SELECT_TURNS_BY_FILE: &str = r#"
SELECT
    turn_index, ts_ms, model,
    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
    user_prompt_preview
FROM session_turns
WHERE file_path = ?1
ORDER BY turn_index ASC
"#;
