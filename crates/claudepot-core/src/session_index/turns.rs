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

/// One turn candidate joined with its session's project_path. Used by
/// the install-wide top-N query so the consumer surface can display
/// the project context alongside the cost.
///
/// `cost_usd` is left for the caller to compute against the active
/// price table — keeping it out of SQL means tier changes don't
/// invalidate any cache, and the rate table lives in `pricing`,
/// not in the index DB.
#[derive(Debug, Clone)]
pub struct TurnCandidate {
    pub file_path: String,
    pub project_path: String,
    pub turn_index: usize,
    pub ts_ms: Option<i64>,
    pub model: String,
    pub tokens: TokenUsage,
    pub user_prompt_preview: Option<String>,
}

/// Fetch a coarse top-K of turn candidates by total token count,
/// inclusive [from_ms, to_ms] on `ts_ms`. The token-sum ordering is
/// a proxy for cost — accurate enough to pre-trim the candidate set
/// before the caller does the model-aware re-rank in Rust. Caller
/// supplies a comfortable `pool_limit` (e.g. `final_n × 50`) so the
/// re-rank can overcome rate divergences across models without
/// scanning the whole table.
///
/// Open-ended bounds (`None`) translate to "no constraint on that
/// side"; passing both `None` is "all time."
pub fn fetch_turn_candidates(
    db: &Connection,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    pool_limit: usize,
) -> Result<Vec<TurnCandidate>, SessionIndexError> {
    // Build the WHERE clause dynamically. Each present bound adds one
    // condition + one bind value, in that order. SQLite ignores
    // missing optionals cleanly via this pattern.
    let (sql, has_from, has_to) = match (from_ms, to_ms) {
        (Some(_), Some(_)) => (SQL_TURN_CANDIDATES_BOTH, true, true),
        (Some(_), None) => (SQL_TURN_CANDIDATES_FROM, true, false),
        (None, Some(_)) => (SQL_TURN_CANDIDATES_TO, false, true),
        (None, None) => (SQL_TURN_CANDIDATES_OPEN, false, false),
    };
    let limit_i64 = i64::try_from(pool_limit).unwrap_or(i64::MAX);
    let mut stmt = db.prepare(sql)?;
    let map_row = |r: &rusqlite::Row| -> rusqlite::Result<TurnCandidate> {
        Ok(TurnCandidate {
            file_path: r.get("file_path")?,
            project_path: r.get("project_path")?,
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
    };
    let rows: Vec<TurnCandidate> = match (has_from, has_to) {
        (true, true) => stmt
            .query_map(
                params![from_ms.unwrap(), to_ms.unwrap(), limit_i64],
                map_row,
            )?
            .collect::<Result<_, _>>()?,
        (true, false) => stmt
            .query_map(params![from_ms.unwrap(), limit_i64], map_row)?
            .collect::<Result<_, _>>()?,
        (false, true) => stmt
            .query_map(params![to_ms.unwrap(), limit_i64], map_row)?
            .collect::<Result<_, _>>()?,
        (false, false) => stmt
            .query_map(params![limit_i64], map_row)?
            .collect::<Result<_, _>>()?,
    };
    Ok(rows)
}

/// Load every persisted turn for one file, ordered by `turn_index`.
/// Empty result is normal — sessions on disk before this table
/// existed have no turns until their next re-scan.
pub fn load_turns(db: &Connection, file_path: &str) -> Result<Vec<TurnRecord>, SessionIndexError> {
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

/// Token-sum ordering used as a coarse cost proxy. Same expression
/// is shared across the four bounded variants below to keep them in
/// lock-step — a future tweak (e.g. weighting cache_read lower)
/// only needs to land here once.
const TURN_RANK_EXPR: &str =
    "(tokens_input + tokens_output + tokens_cache_creation + tokens_cache_read)";

const SQL_TURN_CANDIDATES_BOTH: &str = r#"
SELECT t.file_path, s.project_path, t.turn_index, t.ts_ms, t.model,
       t.tokens_input, t.tokens_output, t.tokens_cache_creation, t.tokens_cache_read,
       t.user_prompt_preview
FROM session_turns t
JOIN sessions s ON s.file_path = t.file_path
WHERE t.ts_ms IS NOT NULL
  AND t.ts_ms >= ?1
  AND t.ts_ms <= ?2
ORDER BY (t.tokens_input + t.tokens_output + t.tokens_cache_creation + t.tokens_cache_read) DESC
LIMIT ?3
"#;

const SQL_TURN_CANDIDATES_FROM: &str = r#"
SELECT t.file_path, s.project_path, t.turn_index, t.ts_ms, t.model,
       t.tokens_input, t.tokens_output, t.tokens_cache_creation, t.tokens_cache_read,
       t.user_prompt_preview
FROM session_turns t
JOIN sessions s ON s.file_path = t.file_path
WHERE t.ts_ms IS NOT NULL
  AND t.ts_ms >= ?1
ORDER BY (t.tokens_input + t.tokens_output + t.tokens_cache_creation + t.tokens_cache_read) DESC
LIMIT ?2
"#;

const SQL_TURN_CANDIDATES_TO: &str = r#"
SELECT t.file_path, s.project_path, t.turn_index, t.ts_ms, t.model,
       t.tokens_input, t.tokens_output, t.tokens_cache_creation, t.tokens_cache_read,
       t.user_prompt_preview
FROM session_turns t
JOIN sessions s ON s.file_path = t.file_path
WHERE t.ts_ms IS NOT NULL
  AND t.ts_ms <= ?1
ORDER BY (t.tokens_input + t.tokens_output + t.tokens_cache_creation + t.tokens_cache_read) DESC
LIMIT ?2
"#;

const SQL_TURN_CANDIDATES_OPEN: &str = r#"
SELECT t.file_path, s.project_path, t.turn_index, t.ts_ms, t.model,
       t.tokens_input, t.tokens_output, t.tokens_cache_creation, t.tokens_cache_read,
       t.user_prompt_preview
FROM session_turns t
JOIN sessions s ON s.file_path = t.file_path
ORDER BY (t.tokens_input + t.tokens_output + t.tokens_cache_creation + t.tokens_cache_read) DESC
LIMIT ?1
"#;

// Silence the dead-code warning on the shared rank expr — kept as a
// const so the four SQL strings can be cross-referenced when the
// proxy formula needs tuning.
#[allow(dead_code)]
const _: &str = TURN_RANK_EXPR;

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
