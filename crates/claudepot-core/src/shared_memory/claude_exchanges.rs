//! Claude-side `exchanges` + `tool_calls` population.
//!
//! Mirrors `shared_memory::indexer::backfill_codex` but for the
//! Claude side. The existing `session_index::refresh` writes
//! one `sessions` row per Claude JSONL but does NOT emit
//! `exchanges` / `tool_calls`; until this module runs,
//! `claudepot_search_memory` returns Codex hits only for Claude
//! files. This closes that gap.
//!
//! Implementation notes:
//!
//! * Reuses `session::parse_events_public` for JSONL decoding so
//!   the Claude event grammar lives in exactly one place. This
//!   module only handles event *pairing* into exchanges and
//!   *writing* into the v4 tables.
//!
//! * Pairing rule: every `UserText` event opens a new exchange.
//!   Any `AssistantText` / `AssistantToolUse` / `UserToolResult`
//!   events that follow are folded into the current exchange
//!   until the next `UserText`. `summary` / `system` / `attachment`
//!   events are ignored (they're not turn content).
//!
//! * Stable exchange id: `claude_code:<slug>/<stem>:<turn_index>`, where
//!   `<slug>/<stem>` identifies the FILE and `turn_index` is the 0-based
//!   ordinal of the user prompt within it. The slug is load-bearing: a
//!   session id is only unique within a project, and CC leaves the same
//!   transcript uuid in two project dirs after a move/adopt. Keying on
//!   the stem alone collided on the `exchanges.id` primary key and
//!   silently dropped the second copy from the index.
//!
//! * Stable tool-call id: `<exchange_id>\u{001f}<ordinal>\u{001f}<tool_use_id>`,
//!   using the L3 unit-separator convention from the Codex indexer (so a
//!   `tool_use_id` containing `:` doesn't collide with the exchange-id
//!   separator). The ordinal is required: `tool_use_id` is not reliably
//!   unique within one exchange, and without it a repeated id collided on
//!   the `tool_calls.id` primary key and rolled the whole file back.
//!
//! * Source kind: `'claude_code'`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::exchange_rows::{self, ExchangeRow, ReconcileWrites, ToolCallRow};
use crate::redaction::{apply as redact_apply, RedactionPolicy};
use crate::session::{parse_events_public, SessionEvent};
use crate::session_index::SessionIndex;

/// Tally of what one `backfill_claude_exchanges` run did.
#[derive(Debug, Default, Clone)]
pub struct ClaudeExchangeStats {
    pub discovered: usize,
    pub indexed: usize,
    pub skipped_unchanged: usize,
    pub failed: Vec<(PathBuf, String)>,
    /// Files left for the next pass because another writer indexed them,
    /// or removed their `sessions` row, between this pass's snapshot and
    /// its write. Not a failure — but not indexed at this file's current
    /// content either, which a caller that needs a specific file indexed
    /// (redaction) must not mistake for success.
    pub conflicted: Vec<PathBuf>,
    /// Rows actually inserted, updated or deleted across `exchanges` and
    /// `tool_calls`. An indexed file whose parse reproduced its stored
    /// rows contributes nothing here.
    pub rows_written: usize,
}

/// A file's `exchange_state` tuple: `(size, mtime_ns, inode)`.
type FileTuple = (i64, i64, i64);

/// Walk `<claude_config_dir>/projects/**/*.jsonl` and populate
/// `exchanges` + `tool_calls` for every Claude `sessions` row whose
/// `(size, mtime_ns, inode)` differs from the tuple recorded in
/// `exchange_state` — i.e. never indexed, or changed since it was.
///
/// `claude_config_dir` is typically `~/.claude` (or the
/// `CLAUDE_CONFIG_DIR` override). The function appends `projects/`
/// itself so callers can pass the literal config dir.
///
/// Staleness is tracked in this module's own `exchange_state` table, NOT
/// against the `sessions` tuple. `session_index::refresh` owns that tuple
/// and keeps it equal to disk, so a backfill running after a refresh —
/// which is exactly the startup order — compared disk against an
/// already-current tuple, concluded "unchanged", and skipped the file.
/// Appended turns were therefore never indexed: a transcript could grow
/// all session long while its new content never reached `exchanges` or
/// the FTS index.
///
/// The guard is the tuple and only the tuple: an edit that preserves a
/// file's size, mtime and inode is not seen. Every rewrite Claudepot
/// itself makes (`session redact`, `session slim`) replaces the file by
/// rename, which moves the inode.
///
/// ## Locking
///
/// Each changed file is parsed with no lock held, then written in its own
/// `BEGIN IMMEDIATE` transaction under the index mutex. This used to run
/// as ONE transaction holding the `SessionIndex` mutex for the whole pass,
/// parse included, so a live 263 MB transcript kept both the mutex and the
/// database write lock for up to a minute of every two, and every other
/// writer of `sessions.db` failed with "database is locked".
///
/// A file's transaction first re-reads its `exchange_state` row and
/// abandons the write when it no longer matches the snapshot this pass
/// planned against — another process (the CLI's backfill, `session
/// redact`'s re-index) got there first, possibly with newer content. The
/// file is reported in `conflicted` and the next pass re-evaluates it.
pub fn backfill_claude_exchanges(
    idx: &SessionIndex,
    claude_config_dir: &Path,
) -> Result<ClaudeExchangeStats, rusqlite::Error> {
    let mut stats = ClaudeExchangeStats::default();
    let projects_root = claude_config_dir.join("projects");
    if !projects_root.is_dir() {
        return Ok(stats);
    }

    // 1. Walk projects/ and collect (file_path, size, mtime, inode)
    //    for every .jsonl.
    let discovered = walk_claude_projects(&projects_root, &mut stats);
    stats.discovered = discovered.len();

    // 2. Snapshot, under a short lock:
    //    a. which transcripts `sessions` knows about — `exchanges` has an
    //       FK onto `sessions.file_path`, so a file the index hasn't seen
    //       yet cannot be written; it waits for the next refresh;
    //    b. what THIS module last indexed, and at which file tuple.
    let (known, existing) = {
        let db = idx.db();
        (
            load_known_claude_sessions(&db)?,
            load_claude_exchange_state(&db)?,
        )
    };

    for entry in &discovered {
        if !known.contains(&entry.file_path) {
            stats.skipped_unchanged += 1;
            continue;
        }
        let snapshot = existing.get(&entry.file_path).copied();
        if snapshot == Some(entry.tuple()) {
            stats.skipped_unchanged += 1;
            continue;
        }
        let rows = match parse_claude_rows(entry) {
            Ok(rows) => rows,
            Err(e) => {
                record_failure(&mut stats, entry, e);
                continue;
            }
        };
        match write_claude_file(idx, entry, snapshot, &rows) {
            Ok(Some(writes)) => {
                stats.indexed += 1;
                stats.rows_written += writes.total();
            }
            Ok(None) => stats.conflicted.push(PathBuf::from(&entry.file_path)),
            Err(e) => record_failure(&mut stats, entry, e),
        }
    }

    Ok(stats)
}

fn record_failure(stats: &mut ClaudeExchangeStats, entry: &ClaudeFile, error: String) {
    tracing::warn!(
        path = %entry.file_path,
        error = %error,
        "shared_memory: claude exchange backfill error"
    );
    stats.failed.push((PathBuf::from(&entry.file_path), error));
}

/// `file_path` of every Claude transcript the session index knows about.
fn load_known_claude_sessions(
    db: &Connection,
) -> Result<std::collections::HashSet<String>, rusqlite::Error> {
    let mut stmt =
        db.prepare("SELECT file_path FROM sessions WHERE source_kind = 'claude_code'")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = std::collections::HashSet::new();
    for r in rows {
        out.insert(r?);
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct ClaudeFile {
    file_path: String,
    size: i64,
    mtime_ns: i64,
    inode: i64,
}

impl ClaudeFile {
    fn tuple(&self) -> FileTuple {
        (self.size, self.mtime_ns, self.inode)
    }
}

fn walk_claude_projects(root: &Path, stats: &mut ClaudeExchangeStats) -> Vec<ClaudeFile> {
    let mut out = Vec::new();
    walk(root, &mut out, stats, 0);
    out
}

fn walk(dir: &Path, out: &mut Vec<ClaudeFile>, stats: &mut ClaudeExchangeStats, depth: usize) {
    if depth > 8 {
        tracing::warn!(depth, dir = %dir.display(), "claude exchanges: depth cap reached");
        return;
    }
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            stats
                .failed
                .push((dir.to_path_buf(), format!("read_dir: {e}")));
            return;
        }
    };
    for entry in read.flatten() {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            // Same containment posture as the Codex indexer.
            continue;
        }
        if meta.is_dir() {
            walk(&path, out, stats, depth + 1);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let (size, mtime_ns, inode) = tuple_of(&meta);
        out.push(ClaudeFile {
            file_path: path.to_string_lossy().into_owned(),
            size,
            mtime_ns,
            inode,
        });
    }
}

/// The `(size, mtime_ns, inode)` staleness tuple of a file, exactly as the
/// walk records it — the one definition both the walk and
/// [`reindex_file_verified`] compare against.
fn tuple_of(meta: &fs::Metadata) -> FileTuple {
    let size = meta.len() as i64;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let inode = crate::fs_utils::file_identity(meta) as i64;
    (size, mtime_ns, inode)
}

/// Why [`reindex_file_verified`] could not confirm a file re-indexed.
#[derive(Debug, thiserror::Error)]
pub enum ReindexError {
    #[error("refresh the session index: {0}")]
    Refresh(#[from] crate::session_index::SessionIndexError),
    #[error("read the exchange index: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("stat {}: {source}", path.display())]
    Stat {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{} is still indexed at its previous content after {attempts} attempts",
        path.display()
    )]
    Stale { path: PathBuf, attempts: u32 },
}

/// How many refresh-and-backfill rounds [`reindex_file_verified`] runs
/// before giving up. A round only fails to land when another writer
/// indexed the file between its snapshot and its write; that writer is
/// either done or has itself indexed the new content by the next round.
const REINDEX_ATTEMPTS: u32 = 3;

/// Re-index the transcript at `file` and confirm its exchange rows now
/// come from its current bytes.
///
/// For a caller about to tell the user that content is gone — `session
/// redact` — "the backfill ran" is not enough: a pass may leave a file in
/// `conflicted` for the next one, or skip it because the session index has
/// not seen it yet, and either way the old text stays searchable. This
/// runs refresh + backfill until the file's `exchange_state` matches its
/// on-disk tuple, and fails with [`ReindexError::Stale`] if it never does.
///
/// A file with no `exchange_state` row at all was never indexed, so there
/// is nothing stale to evict, and that is success.
pub fn reindex_file_verified(
    idx: &SessionIndex,
    claude_config_dir: &Path,
    file: &Path,
) -> Result<(), ReindexError> {
    for _ in 0..REINDEX_ATTEMPTS {
        idx.refresh(claude_config_dir)?;
        backfill_claude_exchanges(idx, claude_config_dir)?;
        let meta = fs::metadata(file).map_err(|source| ReindexError::Stat {
            path: file.to_path_buf(),
            source,
        })?;
        match indexed_tuple(idx, file)? {
            None => return Ok(()),
            Some(t) if t == tuple_of(&meta) => return Ok(()),
            Some(_) => {}
        }
    }
    Err(ReindexError::Stale {
        path: file.to_path_buf(),
        attempts: REINDEX_ATTEMPTS,
    })
}

/// The `exchange_state` tuple recorded for `file`. The row is keyed by the
/// walk's spelling of the path (`<config>/projects/<slug>/<name>.jsonl`),
/// which a caller-supplied path need not match byte for byte, so rows
/// ending in the same file name are compared by canonical path when the
/// exact spelling finds nothing.
fn indexed_tuple(idx: &SessionIndex, file: &Path) -> Result<Option<FileTuple>, rusqlite::Error> {
    let db = idx.db();
    let exact = db
        .query_row(
            "SELECT size, mtime_ns, inode FROM exchange_state WHERE file_path = ?1",
            [file.to_string_lossy()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if exact.is_some() {
        return Ok(exact);
    }
    let (Some(name), Ok(canonical)) = (
        file.file_name(),
        crate::path_utils::canonicalize_simplified(file),
    ) else {
        return Ok(None);
    };
    let mut stmt = db.prepare(
        "SELECT file_path, size, mtime_ns, inode FROM exchange_state \
         WHERE file_path LIKE '%' || ?1",
    )?;
    let rows = stmt.query_map([name.to_string_lossy()], |r| {
        Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
    })?;
    for row in rows {
        let (path, tuple) = row?;
        if crate::path_utils::canonicalize_simplified(Path::new(&path)).ok()
            == Some(canonical.clone())
        {
            return Ok(Some(tuple));
        }
    }
    Ok(None)
}

/// Per-file skip-vs-reindex state: the `(size, mtime_ns, inode)` of each
/// transcript as of the last time THIS module wrote its exchanges. A file
/// with no entry has never been indexed and will be.
fn load_claude_exchange_state(
    db: &Connection,
) -> Result<std::collections::HashMap<String, FileTuple>, rusqlite::Error> {
    // Read the marker THIS module wrote (`exchange_state`), not the
    // `sessions` tuple — `session_index::refresh` owns that one and keeps
    // it equal to disk, so comparing against it made every changed file
    // look unchanged and skipped its re-index. A file `sessions` knows
    // about but that has never been through here has no marker, so it is
    // absent from this map and gets indexed.
    let mut stmt = db.prepare(
        "SELECT es.file_path, es.size, es.mtime_ns, es.inode \
         FROM exchange_state es \
         JOIN sessions s ON s.file_path = es.file_path \
         WHERE s.source_kind = 'claude_code'",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        out.insert(path, (row.get(1)?, row.get(2)?, row.get(3)?));
    }
    Ok(out)
}

/// Parse one transcript into the rows its exchanges should have. No lock
/// is held: on a 263 MB transcript this is the slow part.
fn parse_claude_rows(entry: &ClaudeFile) -> Result<Vec<ExchangeRow>, String> {
    let events =
        parse_events_public(Path::new(&entry.file_path)).map_err(|e| format!("parse: {e}"))?;

    // Derive session_id from the file_path stem (matches what
    // session::scan_session does so the sessions.session_id and
    // exchanges.id namespace agree).
    let session_id = Path::new(&entry.file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("claude-session")
        .to_string();

    // The exchange id must be unique per FILE, not per session id.
    //
    // A session id is only unique within a project: CC leaves the same
    // transcript uuid in two project dirs after a move/adopt (the codebase
    // already knows this — `session_read_path` exists precisely because
    // "two rows can legitimately share a session_id"). Keying the exchange
    // id on the file stem alone therefore collided on `exchanges.id`, the
    // insert rolled the whole file back, and that transcript silently
    // never reached the index. Three transcripts on a real corpus were in
    // exactly this state, invisible because the backfill's `stats.failed`
    // was not surfaced.
    //
    // The slug (the project dir) discriminates them: `sessions.file_path`
    // is `projects/<slug>/<stem>.jsonl`, so `(slug, stem)` is unique.
    let slug = Path::new(&entry.file_path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown-project");

    let exchanges = pair_events_into_exchanges(&format!("{slug}/{session_id}"), &events);
    Ok(exchanges.into_iter().map(to_row).collect())
}

fn to_row(ex: ClaudeExchange) -> ExchangeRow {
    let snippet_text = build_snippet(&ex.user_text, &ex.assistant_text);
    let tool_calls = ex
        .tool_calls
        .into_iter()
        .enumerate()
        .map(|(ordinal, tc)| ToolCallRow {
            // `<exchange_id>\u{1f}<ordinal>\u{1f}<tool_use_id>`.
            //
            // The ordinal is load-bearing. `tool_use_id` is NOT reliably
            // unique within one exchange: a resumed or rescued transcript
            // can replay the same `tool_use` id twice in a turn, and the
            // id used to be `<exchange_id>\u{1f}<tool_use_id>` — which
            // then collided on the `tool_calls.id` primary key. The whole
            // file's write rolled back with
            // "UNIQUE constraint failed: tool_calls.id", so that
            // transcript stayed permanently absent from the exchange
            // index. Observed on a real corpus, where it also kept
            // search's un-indexed-remainder probe permanently non-empty.
            id: format!("{}\u{001f}{}\u{001f}{}", ex.id, ordinal, tc.tool_use_id),
            tool_name: tc.tool_name,
            tool_input_json: Some(tc.tool_input_json),
            tool_result_text: tc.tool_result_text,
            is_error: tc.is_error,
            timestamp_ms: tc.timestamp_ms,
        })
        .collect();
    ExchangeRow {
        id: ex.id,
        turn_index: i64::from(ex.turn_index),
        timestamp_ms: ex.timestamp_ms,
        user_text: ex.user_text,
        assistant_text: ex.assistant_text,
        line_start: None,
        line_end: None,
        snippet_text,
        tool_calls,
    }
}

/// Write one file's rows, if nobody else has since the snapshot.
///
/// `Ok(None)` is the conflict case: the file's `exchange_state` moved away
/// from `snapshot`, or its `sessions` row is gone. Nothing is written.
fn write_claude_file(
    idx: &SessionIndex,
    entry: &ClaudeFile,
    snapshot: Option<FileTuple>,
    rows: &[ExchangeRow],
) -> Result<Option<ReconcileWrites>, String> {
    let mut db = idx.db();
    // IMMEDIATE, not the DEFERRED default: the snapshot re-check below is
    // a read, and a DEFERRED transaction that reads and then writes can
    // lose the upgrade to another connection's commit and fail with
    // SQLITE_BUSY_SNAPSHOT — which the busy handler does not retry.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("begin: {e}"))?;

    let current: Option<FileTuple> = tx
        .query_row(
            "SELECT size, mtime_ns, inode FROM exchange_state WHERE file_path = ?1",
            [&entry.file_path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("read exchange_state: {e}"))?;
    let still_known: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions \
             WHERE file_path = ?1 AND source_kind = 'claude_code')",
            [&entry.file_path],
            |r| r.get(0),
        )
        .map_err(|e| format!("read sessions: {e}"))?;
    if current != snapshot || !still_known {
        // Dropping `tx` rolls back; nothing was written.
        return Ok(None);
    }

    let writes = exchange_rows::reconcile(&tx, &entry.file_path, "claude_code", rows)
        .map_err(|e| format!("write exchanges: {e}"))?;

    // Record the tuple we just indexed AT, in the same transaction as the
    // rows, so a file that fails partway leaves no marker and is retried
    // next pass rather than being mistaken for done.
    tx.execute(
        "INSERT INTO exchange_state (file_path, size, mtime_ns, inode) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(file_path) DO UPDATE SET \
            size = excluded.size, \
            mtime_ns = excluded.mtime_ns, \
            inode = excluded.inode",
        params![entry.file_path, entry.size, entry.mtime_ns, entry.inode],
    )
    .map_err(|e| format!("upsert exchange_state: {e}"))?;

    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(Some(writes))
}

// ─── pairing ─────────────────────────────────────────────────

#[derive(Debug, Default)]
pub(crate) struct ClaudeExchange {
    pub(crate) id: String,
    pub(crate) turn_index: u32,
    pub(crate) user_text: String,
    pub(crate) assistant_text: String,
    pub(crate) timestamp_ms: Option<i64>,
    pub(crate) tool_calls: Vec<ClaudeToolCall>,
}

#[derive(Debug, Default)]
pub(crate) struct ClaudeToolCall {
    pub(crate) tool_use_id: String,
    pub(crate) tool_name: String,
    pub(crate) tool_input_json: String,
    pub(crate) tool_result_text: Option<String>,
    pub(crate) is_error: bool,
    pub(crate) timestamp_ms: Option<i64>,
}

pub(crate) fn pair_events_into_exchanges(
    session_id: &str,
    events: &[SessionEvent],
) -> Vec<ClaudeExchange> {
    let mut out: Vec<ClaudeExchange> = Vec::new();
    let mut current: Option<ClaudeExchange> = None;

    for event in events {
        match event {
            SessionEvent::UserText { ts, text, .. } => {
                if let Some(ex) = current.take() {
                    out.push(ex);
                }
                let turn_index = out.len() as u32;
                current = Some(ClaudeExchange {
                    id: format!("claude_code:{session_id}:{turn_index}"),
                    turn_index,
                    user_text: text.clone(),
                    assistant_text: String::new(),
                    timestamp_ms: ts.map(|t| t.timestamp_millis()),
                    tool_calls: Vec::new(),
                });
            }
            SessionEvent::AssistantText { text, ts, .. } => {
                if let Some(ref mut ex) = current {
                    if !ex.assistant_text.is_empty() {
                        ex.assistant_text.push('\n');
                    }
                    ex.assistant_text.push_str(text);
                    if ex.timestamp_ms.is_none() {
                        ex.timestamp_ms = ts.map(|t| t.timestamp_millis());
                    }
                }
            }
            SessionEvent::AssistantToolUse {
                tool_use_id,
                tool_name,
                input_full,
                ts,
                ..
            } => {
                if let Some(ref mut ex) = current {
                    ex.tool_calls.push(ClaudeToolCall {
                        tool_use_id: tool_use_id.clone(),
                        tool_name: tool_name.clone(),
                        tool_input_json: input_full.clone(),
                        tool_result_text: None,
                        is_error: false,
                        timestamp_ms: ts.map(|t| t.timestamp_millis()),
                    });
                }
            }
            SessionEvent::UserToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                if let Some(ref mut ex) = current {
                    // Pair with the most recent matching tool_use
                    // by id (Claude can emit tool_use → tool_result
                    // back-to-back, multiple in one turn).
                    if let Some(tc) = ex
                        .tool_calls
                        .iter_mut()
                        .rev()
                        .find(|t| t.tool_use_id == *tool_use_id && t.tool_result_text.is_none())
                    {
                        tc.tool_result_text = Some(content.clone());
                        tc.is_error = *is_error;
                    }
                }
            }
            // Summary / system / attachment / thinking / task-summary
            // are not turn content. Skip.
            _ => {}
        }
    }

    if let Some(ex) = current.take() {
        out.push(ex);
    }
    out
}

/// Same shape as `shared_memory::indexer::build_snippet`. Pre-
/// redacts at rest per R9. The 240-char cap matches Codex.
fn build_snippet(user: &str, assistant: &str) -> String {
    const CAP: usize = 240;
    let head = truncate_graphemes(user, CAP);
    let tail = truncate_graphemes(assistant, CAP);
    let combined = if head.is_empty() {
        tail
    } else if tail.is_empty() {
        head
    } else {
        format!("{head}\n→ {tail}")
    };
    redact_apply(&combined, &RedactionPolicy::default())
}

fn truncate_graphemes(s: &str, cap: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out = String::new();
    for (i, g) in s.graphemes(true).enumerate() {
        if i >= cap {
            out.push('…');
            break;
        }
        out.push_str(g);
    }
    out
}

// ─── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;
    use tempfile::TempDir;

    fn open_idx(tmp: &TempDir) -> SessionIndex {
        SessionIndex::open(&tmp.path().join("sessions.db")).unwrap()
    }

    fn open_raw(path: &Path) -> Connection {
        let c = Connection::open(path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        c
    }

    /// Stage a Claude-shape projects/<slug>/<session>.jsonl with a
    /// minimal user/assistant turn + a tool_use/tool_result pair.
    /// Returns the path written.
    fn stage_claude_session(config_dir: &Path, slug: &str, session_id: &str) -> std::path::PathBuf {
        let dir = config_dir.join("projects").join(slug);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{session_id}.jsonl"));
        let body = format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"please refactor the auth flow"}}]}},"timestamp":"2026-05-15T11:30:00.000Z","sessionId":"{session_id}","cwd":"/proj"}}
{{"type":"assistant","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"I'll start by reading the file."}},{{"type":"tool_use","id":"tu_1","name":"Read","input":{{"file_path":"/proj/auth.rs"}}}}]}},"timestamp":"2026-05-15T11:30:01.000Z","sessionId":"{session_id}","cwd":"/proj"}}
{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_1","content":"file contents...","is_error":false}}]}},"timestamp":"2026-05-15T11:30:02.000Z","sessionId":"{session_id}","cwd":"/proj"}}
{{"type":"assistant","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"Got it — here is the refactor."}}]}},"timestamp":"2026-05-15T11:30:03.000Z","sessionId":"{session_id}","cwd":"/proj"}}
"#,
        );
        fs::write(&path, body).unwrap();
        path
    }

    /// `session_index::refresh` needs to see the file first so its
    /// staleness tuple lands in `sessions`. Call refresh manually.
    fn refresh_sessions(idx: &SessionIndex, claude_config: &Path) {
        let stats = idx.refresh(claude_config).expect("refresh");
        assert!(stats.scanned > 0, "refresh should pick up the staged file");
    }

    #[test]
    fn backfill_writes_exchanges_for_claude_files() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);

        let stats = backfill_claude_exchanges(&idx, &claude_config).expect("ok");
        assert_eq!(stats.discovered, 1);
        assert_eq!(stats.indexed, 1);
        assert!(stats.failed.is_empty());

        let db = open_raw(&tmp.path().join("sessions.db"));
        let ex_count: i64 = db
            .query_row(
                "SELECT count(*) FROM exchanges WHERE source_kind = 'claude_code'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // One UserText → one exchange (the second `user` line is a
        // tool_result, which folds into the open exchange).
        assert_eq!(ex_count, 1);

        let user_text: String = db
            .query_row("SELECT user_text FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(user_text, "please refactor the auth flow");

        let assistant_text: String = db
            .query_row("SELECT assistant_text FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert!(assistant_text.contains("I'll start by reading"));
        assert!(assistant_text.contains("Got it — here is the refactor"));

        let tc_count: i64 = db
            .query_row("SELECT count(*) FROM tool_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tc_count, 1);
        let (tool_name, tool_result, is_error): (String, Option<String>, i64) = db
            .query_row(
                "SELECT tool_name, tool_result_text, is_error FROM tool_calls",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(tool_name, "Read");
        assert_eq!(tool_result.as_deref(), Some("file contents..."));
        assert_eq!(is_error, 0);

        // FTS row was populated via trigger.
        let fts_count: i64 = db
            .query_row("SELECT count(*) FROM exchange_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn the_same_session_id_in_two_projects_both_index() {
        // CC leaves the same transcript uuid in two project dirs after a
        // move/adopt — the codebase already knows this (`session_read_path`
        // exists because "two rows can legitimately share a session_id").
        // The exchange id used to be keyed on the file stem alone, so the
        // second file collided on `exchanges.id`, its insert rolled back,
        // and that transcript was silently absent from search. Three
        // transcripts on a real corpus were in exactly this state.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        stage_claude_session(&claude_config, "-proj-a", "dupe");
        stage_claude_session(&claude_config, "-proj-b", "dupe");
        refresh_sessions(&idx, &claude_config);

        let stats = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(stats.indexed, 2, "both copies must index");
        assert!(
            stats.failed.is_empty(),
            "no PK collision expected, got {:?}",
            stats.failed
        );

        let db = open_raw(&tmp.path().join("sessions.db"));
        let files: i64 = db
            .query_row("SELECT COUNT(DISTINCT file_path) FROM exchanges", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(files, 2, "both transcripts must reach the exchange index");
    }

    #[test]
    fn appended_turns_are_reindexed_even_after_a_refresh() {
        // The bug this guards: the backfill used to compare the file on
        // disk against the `sessions` tuple — which `session_index::refresh`
        // owns and keeps equal to disk. Refresh-then-backfill (the startup
        // order) therefore always concluded "unchanged" and skipped the
        // file, so a transcript that grew during a session never got its
        // new turns into `exchanges` or the FTS index. The content the
        // user most wants to find — what they just said — was the content
        // search could never see.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        let path = stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);
        assert_eq!(
            backfill_claude_exchanges(&idx, &claude_config)
                .unwrap()
                .indexed,
            1
        );

        // The session continues: a new turn is appended.
        let mut body = fs::read_to_string(&path).unwrap();
        body.push_str(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"and now the zebrafish question"}]},"timestamp":"2026-05-15T11:31:00.000Z","sessionId":"sid","cwd":"/proj"}
"#,
        );
        // mtime granularity: make sure the tuple genuinely moves.
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, body).unwrap();

        // Refresh first — this is what makes the old guard blind.
        refresh_sessions(&idx, &claude_config);
        let stats = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(
            stats.indexed, 1,
            "a grown transcript must be re-indexed, not skipped"
        );

        let db = open_raw(&tmp.path().join("sessions.db"));
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM exchanges WHERE user_text LIKE '%zebrafish%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the appended turn must reach the exchange index");
    }

    #[test]
    fn second_backfill_skips_unchanged_files() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);

        let s1 = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(s1.indexed, 1);

        let s2 = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(s2.discovered, 1);
        assert_eq!(s2.indexed, 0);
        assert_eq!(s2.skipped_unchanged, 1);
    }

    #[test]
    fn missing_claude_config_is_a_clean_zero() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("nope");
        let stats = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(stats.discovered, 0);
        assert_eq!(stats.indexed, 0);
    }

    #[test]
    fn file_without_sessions_row_is_skipped() {
        // The Claude backfill leans on `session_index::refresh` to
        // have written the staleness tuple first. A file present
        // on disk but absent from `sessions` is skipped (not
        // indexed and not failed) so we don't introduce a parallel
        // staleness model.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        stage_claude_session(&claude_config, "-proj", "sid");
        // Don't refresh.

        let stats = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(stats.discovered, 1);
        assert_eq!(stats.indexed, 0);
        assert_eq!(stats.skipped_unchanged, 1);
    }

    #[test]
    fn assistant_messages_concatenate_within_a_turn() {
        // Claude often emits multiple AssistantText events in one
        // turn (one per content block + tool_use interleavings).
        // The backfill should concatenate them with newlines.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        let dir = claude_config.join("projects").join("-proj");
        fs::create_dir_all(&dir).unwrap();
        let body = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]},"timestamp":"2026-05-15T11:30:00.000Z","sessionId":"multi","cwd":"/p"}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"first chunk"}]},"timestamp":"2026-05-15T11:30:01.000Z","sessionId":"multi","cwd":"/p"}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"second chunk"}]},"timestamp":"2026-05-15T11:30:02.000Z","sessionId":"multi","cwd":"/p"}
"#;
        fs::write(dir.join("multi.jsonl"), body).unwrap();
        refresh_sessions(&idx, &claude_config);

        backfill_claude_exchanges(&idx, &claude_config).unwrap();

        let db = open_raw(&tmp.path().join("sessions.db"));
        let assistant: String = db
            .query_row("SELECT assistant_text FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(assistant, "first chunk\nsecond chunk");
    }

    // ─── incremental writes ──────────────────────────────────────

    /// One transcript exercising the pairing edge cases: a message id
    /// repeated across lines (usage charged once, text kept), a tool_use id
    /// repeated inside one turn, an empty result, a line carrying a result
    /// for the previous turn AND the next prompt, a line carrying two
    /// prompts, a CRLF line, and a result with no matching call.
    fn edge_case_transcript() -> String {
        let ts = |s: u32| format!("2026-05-15T11:30:{s:02}.000Z");
        let meta = |s: u32| {
            format!(
                "\"timestamp\":\"{}\",\"sessionId\":\"sid\",\"cwd\":\"/proj\"",
                ts(s)
            )
        };
        [
            format!(r#"{{"type":"user","message":{{"role":"user","content":"refactor the auth flow"}},{}}}"#, meta(0)),
            format!(r#"{{"type":"assistant","message":{{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"reading the file"}},{{"type":"tool_use","id":"tu_1","name":"Read","input":{{"file_path":"/proj/auth.rs"}}}}],"usage":{{"input_tokens":10,"output_tokens":5}}}},{}}}"#, meta(1)),
            format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_1","content":"fn login() {{}}","is_error":false}}]}},{}}}"#, meta(2)),
            format!(r#"{{"type":"assistant","message":{{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"zebrafish pattern found"}}],"usage":{{"input_tokens":10,"output_tokens":5}}}},{}}}"#, meta(3)),
            format!(r#"{{"type":"assistant","message":{{"id":"m2","role":"assistant","model":"claude-opus-4-7","content":[{{"type":"tool_use","id":"tu_2","name":"Bash","input":{{"command":"cargo test"}}}},{{"type":"tool_use","id":"tu_2","name":"Bash","input":{{"command":"cargo test --retry"}}}}]}},{}}}"#, meta(4)),
            format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_2","content":"","is_error":false}},{{"type":"text","text":"and the second question"}}]}},{}}}"#, meta(5)),
            format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"first ask"}},{{"type":"text","text":"second ask"}}]}},{}}}"#, meta(6)),
            format!(r#"{{"type":"assistant","message":{{"id":"m3","role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"done"}}]}},{}}}{}"#, meta(7), "\r"),
            format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_9","content":"orphan","is_error":true}}]}},{}}}"#, meta(8)),
        ]
        .iter()
        .map(|l| format!("{l}\n"))
        .collect()
    }

    const EDGE_TERMS: &[&str] = &["reading", "zebrafish", "second", "ask", "done", "auth"];

    /// Refresh + backfill `body` as a brand-new index; the reference a
    /// history of incremental passes must equal.
    fn fresh_index_of(body: &str) -> (TempDir, SessionIndex, String) {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let cfg = tmp.path().join("claude");
        let dir = cfg.join("projects").join("-proj");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sid.jsonl");
        fs::write(&path, body).unwrap();
        idx.refresh(&cfg).unwrap();
        let stats = backfill_claude_exchanges(&idx, &cfg).unwrap();
        assert!(stats.failed.is_empty(), "{:?}", stats.failed);
        (tmp, idx, path.to_string_lossy().into_owned())
    }

    #[test]
    fn backfill_after_every_append_matches_one_backfill_of_the_whole_file() {
        use super::super::exchange_rows::test_support::snapshot;
        use std::io::Write;

        let body = edge_case_transcript();
        // Every line boundary, and the middle of every line: the index is
        // re-read while Claude Code is half-way through writing a line.
        let mut cuts: Vec<usize> = Vec::new();
        let mut start = 0;
        for (i, b) in body.bytes().enumerate() {
            if b == b'\n' {
                cuts.push(start + (i - start) / 2);
                cuts.push(i + 1);
                start = i + 1;
            }
        }

        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let cfg = tmp.path().join("claude");
        let dir = cfg.join("projects").join("-proj");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sid.jsonl");
        fs::write(&path, "").unwrap();
        let live = path.to_string_lossy().into_owned();

        let mut written = 0;
        for cut in cuts {
            // Append, never rewrite: the file keeps its inode, as a live
            // transcript does.
            let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&body.as_bytes()[written..cut]).unwrap();
            drop(f);
            written = cut;

            idx.refresh(&cfg).unwrap();
            let stats = backfill_claude_exchanges(&idx, &cfg).unwrap();
            assert!(stats.failed.is_empty(), "cut {cut}: {:?}", stats.failed);
            assert!(
                stats.conflicted.is_empty(),
                "cut {cut}: {:?}",
                stats.conflicted
            );

            let (_ref_dir, reference, ref_path) = fresh_index_of(&body[..cut]);
            let norm = |s: super::super::exchange_rows::test_support::Snapshot, p: &str| {
                let strip = |v: Vec<String>| -> Vec<String> {
                    v.into_iter().map(|r| r.replace(p, "<file>")).collect()
                };
                (strip(s.0), strip(s.1), s.2)
            };
            assert_eq!(
                norm(snapshot(&idx.db(), EDGE_TERMS), &live),
                norm(snapshot(&reference.db(), EDGE_TERMS), &ref_path),
                "after {cut} of {} bytes the incrementally maintained rows differ \
                 from a fresh index of the same bytes",
                body.len()
            );
        }
    }

    #[test]
    fn a_growing_transcript_writes_only_what_it_appended() {
        // The measured failure: every pass deleted and re-inserted every
        // row of every transcript that had grown, so a live 263 MB one was
        // rewritten in full every two minutes.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        let path = stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);
        let first = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(first.rows_written, 2, "one exchange + one tool call");

        let mut body = fs::read_to_string(&path).unwrap();
        body.push_str(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"and now the tests"}]},"timestamp":"2026-05-15T11:31:00.000Z","sessionId":"sid","cwd":"/proj"}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu_2","name":"Bash","input":{"command":"cargo test"}}]},"timestamp":"2026-05-15T11:31:01.000Z","sessionId":"sid","cwd":"/proj"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_2","content":"ok","is_error":false}]},"timestamp":"2026-05-15T11:31:02.000Z","sessionId":"sid","cwd":"/proj"}
"#,
        );
        fs::write(&path, body).unwrap();
        refresh_sessions(&idx, &claude_config);
        let second = backfill_claude_exchanges(&idx, &claude_config).unwrap();
        assert_eq!(second.indexed, 1);
        assert_eq!(
            second.rows_written, 2,
            "the new exchange and its tool call — the first exchange is untouched"
        );
    }

    #[test]
    fn a_file_another_writer_indexed_meanwhile_is_left_for_the_next_pass() {
        // Another process (the CLI's backfill, `session redact`'s re-index)
        // wrote this file's rows after this pass took its snapshot. This
        // pass's parse may be OLDER than what it would overwrite — for a
        // redaction, older means the text that was just removed.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        let path = stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);
        backfill_claude_exchanges(&idx, &claude_config).unwrap();

        let meta = fs::metadata(&path).unwrap();
        let (size, mtime_ns, inode) = tuple_of(&meta);
        let entry = ClaudeFile {
            file_path: path.to_string_lossy().into_owned(),
            size,
            mtime_ns,
            inode,
        };
        // Planned as if the file had never been indexed.
        let stale_rows = vec![ExchangeRow {
            id: "claude_code:-proj/sid:0".into(),
            turn_index: 0,
            timestamp_ms: None,
            user_text: "stale plan".into(),
            assistant_text: String::new(),
            line_start: None,
            line_end: None,
            snippet_text: String::new(),
            tool_calls: vec![],
        }];
        assert_eq!(write_claude_file(&idx, &entry, None, &stale_rows), Ok(None));

        let db = open_raw(&tmp.path().join("sessions.db"));
        let user: String = db
            .query_row("SELECT user_text FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            user, "please refactor the auth flow",
            "nothing may be written"
        );
    }

    #[test]
    fn reindex_file_verified_evicts_rewritten_text() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        let path = stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);
        backfill_claude_exchanges(&idx, &claude_config).unwrap();

        // `session redact`'s shape: rewrite to a sibling, rename over.
        let redacted = fs::read_to_string(&path)
            .unwrap()
            .replace("refactor the auth flow", "refactor the [REDACTED] flow");
        let tmp_file = path.with_extension("jsonl.tmp");
        fs::write(&tmp_file, redacted).unwrap();
        fs::rename(&tmp_file, &path).unwrap();

        reindex_file_verified(&idx, &claude_config, &path).unwrap();

        let db = open_raw(&tmp.path().join("sessions.db"));
        let hits: i64 = db
            .query_row(
                "SELECT count(*) FROM exchange_fts WHERE exchange_fts MATCH 'auth'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            hits, 0,
            "the redacted word must be gone from the text index"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reindex_file_verified_fails_when_the_file_cannot_be_reindexed() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let claude_config = tmp.path().join("claude");
        let path = stage_claude_session(&claude_config, "-proj", "sid");
        refresh_sessions(&idx, &claude_config);
        backfill_claude_exchanges(&idx, &claude_config).unwrap();

        // New content that cannot be read, so no pass can index it.
        let mut body = fs::read_to_string(&path).unwrap();
        body.push('\n');
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            // Running as root: permissions do not stop the read, so this
            // setup cannot produce an unindexable file.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            return;
        }
        let err = reindex_file_verified(&idx, &claude_config, &path).unwrap_err();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            matches!(
                err,
                ReindexError::Stale {
                    attempts: REINDEX_ATTEMPTS,
                    ..
                }
            ),
            "got {err}"
        );
    }
}
