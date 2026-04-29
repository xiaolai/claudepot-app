//! SQL ↔ `SessionRow` encoding. Kept out of `mod.rs` so the refresh
//! orchestration stays readable.
//!
//! Timestamp conventions:
//!   - `file_mtime_ns` — nanoseconds since `UNIX_EPOCH`, i64 (safe
//!     through 2262 AD). 0 means "unknown / pre-epoch / fs error".
//!   - `first_ts_ms` / `last_ts_ms` — milliseconds since epoch, i64,
//!     NULL-able (transcripts can have zero timestamped events).
//!   - `indexed_at_ms` — wall-clock ms when the row was written.

use crate::session::{SessionRow, TokenUsage};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::diff::IndexTuple;
use super::SessionIndexError;

/// Result of walking the filesystem: everything that could be stat'd,
/// plus a list of files that were visible but wouldn't stat. Callers
/// surface the latter through `RefreshStats.failed` so transient
/// ENOENT / EACCES don't turn into silent "gone from cache" outcomes.
pub(super) struct WalkOutcome {
    pub entries: Vec<FsEntry>,
    pub stat_failed: Vec<(PathBuf, String)>,
}

/// Walk `<config_dir>/projects/*/*.jsonl` and return `(slug, absolute_path,
/// IndexTuple)` triples. The triple layout lets the refresh path feed the
/// pure diff fn without re-walking to recover the slug later.
pub(super) fn walk_fs(config_dir: &Path) -> Result<WalkOutcome, SessionIndexError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(WalkOutcome {
            entries: Vec::new(),
            stat_failed: Vec::new(),
        });
    }
    let mut entries = Vec::new();
    let mut stat_failed: Vec<(PathBuf, String)> = Vec::new();
    for slug_entry in fs::read_dir(&projects_dir)? {
        let slug_entry = slug_entry?;
        if !slug_entry.file_type()?.is_dir() {
            continue;
        }
        let slug = slug_entry.file_name().to_string_lossy().into_owned();
        for session_entry in fs::read_dir(slug_entry.path())? {
            let session_entry = session_entry?;
            let name = session_entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".jsonl") {
                continue;
            }
            let path = session_entry.path();
            // stat() errors: record them rather than silently dropping
            // so callers can distinguish "transcript genuinely gone"
            // from "transient ENOENT/EACCES during walk".
            let meta = match fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    stat_failed.push((path, e.to_string()));
                    continue;
                }
            };
            let size = meta.len();
            let mtime_ns = mtime_ns_of(&meta);
            let inode = inode_of(&meta);
            let file_path = path.to_string_lossy().into_owned();
            entries.push(FsEntry {
                slug: slug.clone(),
                path,
                tuple: IndexTuple {
                    file_path,
                    size,
                    mtime_ns,
                    inode,
                },
            });
        }
    }
    Ok(WalkOutcome {
        entries,
        stat_failed,
    })
}

/// One discovered transcript on disk, carrying everything the refresh
/// pipeline needs to either verify-and-skip (via tuple match) or
/// re-scan-and-upsert (via slug + path).
pub(super) struct FsEntry {
    pub slug: String,
    pub path: PathBuf,
    pub tuple: IndexTuple,
}

/// Read every `(file_path, size, mtime_ns, inode)` tuple from the
/// cache. Ordering is not meaningful here — the diff fn rebuilds
/// hashmaps.
pub(super) fn load_db_tuples(db: &Connection) -> Result<Vec<IndexTuple>, SessionIndexError> {
    let mut stmt =
        db.prepare("SELECT file_path, file_size_bytes, file_mtime_ns, file_inode FROM sessions")?;
    let rows = stmt.query_map([], |r| {
        Ok(IndexTuple {
            file_path: r.get::<_, String>(0)?,
            size: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
            mtime_ns: r.get::<_, i64>(2)?,
            inode: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Full UPSERT. `indexed_at_ms` is passed in so a single refresh pass
/// stamps all rows with the same wall-clock value — useful for
/// diagnostics ("which pass wrote this row").
///
/// Redacts any `sk-ant-*` token that happens to appear in
/// `first_user_prompt` before persisting. The on-disk JSONL already
/// contains that data and redaction there is out of scope, but
/// copying it verbatim into a secondary cache that a user might
/// include in a bug report / backup would compound the exposure.
pub(super) fn upsert_row(
    db: &Connection,
    row: &SessionRow,
    indexed_at_ms: i64,
) -> Result<(), SessionIndexError> {
    let file_path = row.file_path.to_string_lossy().into_owned();
    let mtime_ns = row.last_modified.map(mtime_ns_of_systemtime).unwrap_or(0);
    // Inode is read from the file on disk at UPSERT time (cheap stat)
    // so callers don't need to thread it through SessionRow, which is
    // the public session API type. Missing → 0, which matches the
    // non-Unix degraded-mode contract.
    let inode: i64 = std::fs::metadata(&row.file_path)
        .ok()
        .map(|m| i64::try_from(inode_of(&m)).unwrap_or(0))
        .unwrap_or(0);
    let models_json = serde_json::to_string(&row.models)?;
    let first_user_prompt = row.first_user_prompt.as_deref().map(redact_secrets);

    db.execute(
        SQL_UPSERT,
        params![
            file_path,
            row.slug,
            row.session_id,
            i64::try_from(row.file_size_bytes).unwrap_or(i64::MAX),
            mtime_ns,
            inode,
            row.project_path,
            row.project_from_transcript as i64,
            row.first_ts.map(|t| t.timestamp_millis()),
            row.last_ts.map(|t| t.timestamp_millis()),
            row.event_count as i64,
            row.message_count as i64,
            row.user_message_count as i64,
            row.assistant_message_count as i64,
            first_user_prompt,
            models_json,
            i64::try_from(row.tokens.input).unwrap_or(i64::MAX),
            i64::try_from(row.tokens.output).unwrap_or(i64::MAX),
            i64::try_from(row.tokens.cache_creation).unwrap_or(i64::MAX),
            i64::try_from(row.tokens.cache_read).unwrap_or(i64::MAX),
            row.git_branch,
            row.cc_version,
            row.display_slug,
            row.has_error as i64,
            row.is_sidechain as i64,
            indexed_at_ms,
        ],
    )?;
    Ok(())
}

#[cfg(unix)]
fn inode_of(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn inode_of(_meta: &fs::Metadata) -> u64 {
    0
}

/// Replace any `sk-ant-*` token run with `sk-ant-****` so pasted
/// credentials don't survive into `sessions.db`. A token is
/// `sk-ant-` followed by one or more `[A-Za-z0-9_-]`.
fn redact_secrets(input: &str) -> String {
    const NEEDLE: &str = "sk-ant-";
    const REPLACEMENT: &str = "sk-ant-****";
    if !input.contains(NEEDLE) {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find(NEEDLE) {
        out.push_str(&rest[..idx]);
        out.push_str(REPLACEMENT);
        let after = &rest[idx + NEEDLE.len()..];
        // Skip the token body (continuation chars allowed in sk-ant-*).
        let tail_start = after
            .char_indices()
            .find(|(_, c)| !is_token_char(*c))
            .map(|(i, _)| i)
            .unwrap_or(after.len());
        rest = &after[tail_start..];
    }
    out.push_str(rest);
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Remove one row by path. Used for files that vanished from disk.
pub(super) fn delete_row(db: &Connection, file_path: &str) -> Result<(), SessionIndexError> {
    db.execute(
        "DELETE FROM sessions WHERE file_path = ?1",
        params![file_path],
    )?;
    Ok(())
}

/// Load every cached row back as `SessionRow`, ordered newest-first
/// (by `last_ts_ms`, falling back to `file_mtime_ns`). Matches the
/// sort contract of the pre-cache `list_all_sessions`.
pub(super) fn load_all_rows(db: &Connection) -> Result<Vec<SessionRow>, SessionIndexError> {
    let mut stmt = db.prepare(SQL_SELECT_ALL_SORTED)?;
    let rows = stmt.query_map([], row_from_sql)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Look up a single cached row by path. Handy for diagnostics and
/// tests that want to verify an UPSERT landed.
#[cfg(test)]
pub(super) fn get_row_by_path(
    db: &Connection,
    file_path: &str,
) -> Result<Option<SessionRow>, SessionIndexError> {
    use rusqlite::OptionalExtension;
    let mut stmt = db.prepare(&format!("{SQL_SELECT_ALL} WHERE file_path = ?1 LIMIT 1"))?;
    let row = stmt
        .query_row(params![file_path], row_from_sql)
        .optional()?;
    Ok(row)
}

fn row_from_sql(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    let file_path: String = r.get("file_path")?;
    let models_json: String = r.get("models_json")?;
    let models: Vec<String> = serde_json::from_str(&models_json).unwrap_or_default();
    let first_ts_ms: Option<i64> = r.get("first_ts_ms")?;
    let last_ts_ms: Option<i64> = r.get("last_ts_ms")?;
    let mtime_ns: i64 = r.get("file_mtime_ns")?;

    Ok(SessionRow {
        session_id: r.get("session_id")?,
        slug: r.get("slug")?,
        file_path: PathBuf::from(&file_path),
        file_size_bytes: u64::try_from(r.get::<_, i64>("file_size_bytes")?).unwrap_or(0),
        last_modified: systemtime_from_mtime_ns(mtime_ns),
        project_path: r.get("project_path")?,
        project_from_transcript: r.get::<_, i64>("project_from_transcript")? != 0,
        first_ts: first_ts_ms.and_then(ms_to_dt),
        last_ts: last_ts_ms.and_then(ms_to_dt),
        event_count: usize::try_from(r.get::<_, i64>("event_count")?).unwrap_or(0),
        message_count: usize::try_from(r.get::<_, i64>("message_count")?).unwrap_or(0),
        user_message_count: usize::try_from(r.get::<_, i64>("user_message_count")?).unwrap_or(0),
        assistant_message_count: usize::try_from(r.get::<_, i64>("assistant_message_count")?)
            .unwrap_or(0),
        first_user_prompt: r.get("first_user_prompt")?,
        models,
        tokens: TokenUsage {
            input: u64::try_from(r.get::<_, i64>("tokens_input")?).unwrap_or(0),
            output: u64::try_from(r.get::<_, i64>("tokens_output")?).unwrap_or(0),
            cache_creation: u64::try_from(r.get::<_, i64>("tokens_cache_creation")?).unwrap_or(0),
            cache_read: u64::try_from(r.get::<_, i64>("tokens_cache_read")?).unwrap_or(0),
        },
        git_branch: r.get("git_branch")?,
        cc_version: r.get("cc_version")?,
        display_slug: r.get("display_slug")?,
        has_error: r.get::<_, i64>("has_error")? != 0,
        is_sidechain: r.get::<_, i64>("is_sidechain")? != 0,
    })
}

fn mtime_ns_of(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .map(mtime_ns_of_systemtime)
        .unwrap_or(0)
}

fn mtime_ns_of_systemtime(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

fn systemtime_from_mtime_ns(ns: i64) -> Option<SystemTime> {
    if ns <= 0 {
        return None;
    }
    Some(UNIX_EPOCH + std::time::Duration::from_nanos(ns as u64))
}

fn ms_to_dt(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_millis(ms)
}

const SQL_UPSERT: &str = r#"
INSERT INTO sessions (
    file_path, slug, session_id, file_size_bytes, file_mtime_ns, file_inode,
    project_path, project_from_transcript, first_ts_ms, last_ts_ms,
    event_count, message_count, user_message_count, assistant_message_count,
    first_user_prompt, models_json,
    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
    git_branch, cc_version, display_slug, has_error, is_sidechain,
    indexed_at_ms
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
    ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
)
ON CONFLICT(file_path) DO UPDATE SET
    slug                     = excluded.slug,
    session_id               = excluded.session_id,
    file_size_bytes          = excluded.file_size_bytes,
    file_mtime_ns            = excluded.file_mtime_ns,
    file_inode               = excluded.file_inode,
    project_path             = excluded.project_path,
    project_from_transcript  = excluded.project_from_transcript,
    first_ts_ms              = excluded.first_ts_ms,
    last_ts_ms               = excluded.last_ts_ms,
    event_count              = excluded.event_count,
    message_count            = excluded.message_count,
    user_message_count       = excluded.user_message_count,
    assistant_message_count  = excluded.assistant_message_count,
    first_user_prompt        = excluded.first_user_prompt,
    models_json              = excluded.models_json,
    tokens_input             = excluded.tokens_input,
    tokens_output            = excluded.tokens_output,
    tokens_cache_creation    = excluded.tokens_cache_creation,
    tokens_cache_read        = excluded.tokens_cache_read,
    git_branch               = excluded.git_branch,
    cc_version               = excluded.cc_version,
    display_slug             = excluded.display_slug,
    has_error                = excluded.has_error,
    is_sidechain             = excluded.is_sidechain,
    indexed_at_ms            = excluded.indexed_at_ms
"#;

#[cfg(test)]
const SQL_SELECT_ALL: &str = r#"
SELECT
    file_path, slug, session_id, file_size_bytes, file_mtime_ns,
    project_path, project_from_transcript, first_ts_ms, last_ts_ms,
    event_count, message_count, user_message_count, assistant_message_count,
    first_user_prompt, models_json,
    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
    git_branch, cc_version, display_slug, has_error, is_sidechain,
    indexed_at_ms
FROM sessions
"#;

const SQL_SELECT_ALL_SORTED: &str = r#"
SELECT
    file_path, slug, session_id, file_size_bytes, file_mtime_ns,
    project_path, project_from_transcript, first_ts_ms, last_ts_ms,
    event_count, message_count, user_message_count, assistant_message_count,
    first_user_prompt, models_json,
    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
    git_branch, cc_version, display_slug, has_error, is_sidechain,
    indexed_at_ms
FROM sessions
ORDER BY
    COALESCE(last_ts_ms, file_mtime_ns / 1000000) DESC
"#;

#[cfg(test)]
mod redact_tests {
    use super::redact_secrets;

    #[test]
    fn passthrough_when_no_token() {
        assert_eq!(redact_secrets("debug this"), "debug this");
    }

    #[test]
    fn redacts_a_bare_token() {
        let out = redact_secrets("here is sk-ant-oat01-AbC123_-xYz end");
        assert_eq!(out, "here is sk-ant-**** end");
    }

    #[test]
    fn redacts_multiple_tokens() {
        let out = redact_secrets("sk-ant-one sk-ant-two_-MixEd");
        assert_eq!(out, "sk-ant-**** sk-ant-****");
    }

    #[test]
    fn redacts_at_string_boundaries() {
        assert_eq!(redact_secrets("sk-ant-Abc"), "sk-ant-****");
        assert_eq!(redact_secrets("sk-ant-Abc\n"), "sk-ant-****\n");
    }

    #[test]
    fn leaves_the_bare_prefix_alone_when_not_a_token() {
        // An `sk-ant-` with no continuation chars becomes the redacted
        // form anyway (still matches the prefix), which is fine —
        // better to over-redact than leak.
        assert_eq!(redact_secrets("sk-ant- space"), "sk-ant-**** space");
    }
}
