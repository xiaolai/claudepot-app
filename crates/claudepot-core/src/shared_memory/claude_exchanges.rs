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

use rusqlite::params;

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
}

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

    let db = idx.db();
    let tx = db.unchecked_transaction()?;

    // 2a. Which transcripts does `sessions` know about? `exchanges` has a
    //     FK onto `sessions.file_path`, so a file the index hasn't seen
    //     yet cannot be written — it waits for the next refresh.
    let known = load_known_claude_sessions(&tx)?;
    // 2b. What did THIS module last index, and at which file tuple?
    let existing = load_claude_exchange_state(&tx)?;

    for entry in &discovered {
        if !known.contains(&entry.file_path) {
            stats.skipped_unchanged += 1;
            continue;
        }
        match upsert_claude_exchanges_in_savepoint(&tx, entry, existing.get(&entry.file_path)) {
            Ok(Outcome::Indexed) => stats.indexed += 1,
            Ok(Outcome::Skipped) => stats.skipped_unchanged += 1,
            Err(e) => {
                tracing::warn!(
                    path = %entry.file_path,
                    error = %e,
                    "shared_memory: claude exchange backfill error"
                );
                stats.failed.push((PathBuf::from(&entry.file_path), e));
            }
        }
    }

    tx.commit()?;
    Ok(stats)
}

/// `file_path` of every Claude transcript the session index knows about.
fn load_known_claude_sessions(
    tx: &rusqlite::Transaction<'_>,
) -> Result<std::collections::HashSet<String>, rusqlite::Error> {
    let mut stmt =
        tx.prepare("SELECT file_path FROM sessions WHERE source_kind = 'claude_code'")?;
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
        let size = meta.len() as i64;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let inode = inode_of(&meta);
        out.push(ClaudeFile {
            file_path: path.to_string_lossy().into_owned(),
            size,
            mtime_ns,
            inode,
        });
    }
}

fn inode_of(meta: &fs::Metadata) -> i64 {
    crate::fs_utils::file_identity(meta) as i64
}

/// Per-file skip-vs-reindex state: the `(size, mtime_ns, inode)` of each
/// transcript as of the last time THIS module wrote its exchanges. A file
/// with no entry has never been indexed and will be.
fn load_claude_exchange_state(
    tx: &rusqlite::Transaction<'_>,
) -> Result<std::collections::HashMap<String, (i64, i64, i64, i64)>, rusqlite::Error> {
    // Read the marker THIS module wrote (`exchange_state`), not the
    // `sessions` tuple — `session_index::refresh` owns that one and keeps
    // it equal to disk, so comparing against it made every changed file
    // look unchanged and skipped its re-index. A file `sessions` knows
    // about but that has never been through here has no marker, so it is
    // absent from this map and gets indexed.
    //
    // The trailing `1` keeps the tuple shape stable for callers; the
    // exchange count is no longer part of the decision (a transcript with
    // no user turns legitimately yields zero exchanges, and re-indexing
    // it on every pass just to rediscover that was wasted work).
    let mut stmt = tx.prepare(
        "SELECT es.file_path, es.size, es.mtime_ns, es.inode \
         FROM exchange_state es \
         JOIN sessions s ON s.file_path = es.file_path \
         WHERE s.source_kind = 'claude_code'",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        let s: i64 = row.get(1)?;
        let m: i64 = row.get(2)?;
        let i: i64 = row.get(3)?;
        out.insert(path, (s, m, i, 1));
    }
    Ok(out)
}

enum Outcome {
    Indexed,
    Skipped,
}

fn upsert_claude_exchanges_in_savepoint(
    tx: &rusqlite::Transaction<'_>,
    entry: &ClaudeFile,
    existing: Option<&(i64, i64, i64, i64)>,
) -> Result<Outcome, String> {
    tx.execute_batch("SAVEPOINT claude_exchanges")
        .map_err(|e| format!("savepoint: {e}"))?;
    let outcome = upsert_claude_exchanges(tx, entry, existing);
    match outcome {
        Ok(o) => {
            tx.execute_batch("RELEASE claude_exchanges")
                .map_err(|e| format!("release: {e}"))?;
            Ok(o)
        }
        Err(e) => {
            let _ = tx.execute_batch("ROLLBACK TO claude_exchanges; RELEASE claude_exchanges");
            Err(e)
        }
    }
}

fn upsert_claude_exchanges(
    tx: &rusqlite::Transaction<'_>,
    entry: &ClaudeFile,
    existing: Option<&(i64, i64, i64, i64)>,
) -> Result<Outcome, String> {
    // The caller has already established the file is in `sessions`. The
    // only question here is whether THIS module has indexed it at its
    // current on-disk tuple. No marker (never indexed) or a moved tuple
    // (the transcript grew) means re-index. Comparing against the
    // `sessions` tuple instead — as this used to — always said "unchanged"
    // once a refresh had run, so appended turns never got indexed.
    if let Some((size, mtime, inode, _)) = existing {
        if *size == entry.size && *mtime == entry.mtime_ns && *inode == entry.inode {
            return Ok(Outcome::Skipped);
        }
    }

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

    // Pair events into exchanges.
    let exchanges = pair_events_into_exchanges(&format!("{slug}/{session_id}"), &events);

    // Wipe + reinsert the per-file exchanges. FK cascade + FTS
    // trigger keep tool_calls and exchange_fts in sync.
    tx.execute(
        "DELETE FROM exchanges WHERE file_path = ?1",
        [&entry.file_path],
    )
    .map_err(|e| format!("delete exchanges: {e}"))?;

    for ex in &exchanges {
        let snippet = build_snippet(&ex.user_text, &ex.assistant_text);
        tx.execute(
            "INSERT INTO exchanges (
                id, file_path, source_kind, turn_index, role_pair,
                timestamp_ms, user_text, assistant_text,
                line_start, line_end, is_sidechain, parent_id, snippet_text
            ) VALUES (
                ?1, ?2, 'claude_code', ?3, 'user_assistant',
                ?4, ?5, ?6,
                NULL, NULL, 0, NULL, ?7
            )",
            params![
                ex.id,
                entry.file_path,
                ex.turn_index,
                ex.timestamp_ms,
                ex.user_text,
                ex.assistant_text,
                snippet,
            ],
        )
        .map_err(|e| format!("insert exchange: {e}"))?;

        for (ordinal, tc) in ex.tool_calls.iter().enumerate() {
            // `<exchange_id>\u{1f}<ordinal>\u{1f}<tool_use_id>`.
            //
            // The ordinal is load-bearing. `tool_use_id` is NOT reliably
            // unique within one exchange: a resumed or rescued transcript
            // can replay the same `tool_use` id twice in a turn, and the
            // id used to be `<exchange_id>\u{1f}<tool_use_id>` — which
            // then collided on the `tool_calls.id` primary key. The whole
            // file's savepoint rolled back with
            // "UNIQUE constraint failed: tool_calls.id", so that
            // transcript stayed permanently absent from the exchange
            // index. Observed on a real corpus, where it also kept
            // search's un-indexed-remainder probe permanently non-empty.
            let tc_id = format!("{}\u{001f}{}\u{001f}{}", ex.id, ordinal, tc.tool_use_id);
            tx.execute(
                "INSERT INTO tool_calls (
                    id, exchange_id, tool_name, tool_input_json,
                    tool_result_text, is_error, timestamp_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    tc_id,
                    ex.id,
                    tc.tool_name,
                    tc.tool_input_json,
                    tc.tool_result_text,
                    tc.is_error as i64,
                    tc.timestamp_ms,
                ],
            )
            .map_err(|e| format!("insert tool_call: {e}"))?;
        }
    }

    // Record the tuple we just indexed AT. Inside the same savepoint as
    // the writes above, so a file that fails partway leaves no marker and
    // is retried next pass rather than being mistaken for done.
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

    Ok(Outcome::Indexed)
}

// ─── pairing ─────────────────────────────────────────────────

#[derive(Debug, Default)]
struct ClaudeExchange {
    id: String,
    turn_index: u32,
    user_text: String,
    assistant_text: String,
    timestamp_ms: Option<i64>,
    tool_calls: Vec<ClaudeToolCall>,
}

#[derive(Debug, Default)]
struct ClaudeToolCall {
    tool_use_id: String,
    tool_name: String,
    tool_input_json: String,
    tool_result_text: Option<String>,
    is_error: bool,
    timestamp_ms: Option<i64>,
}

fn pair_events_into_exchanges(session_id: &str, events: &[SessionEvent]) -> Vec<ClaudeExchange> {
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
}
