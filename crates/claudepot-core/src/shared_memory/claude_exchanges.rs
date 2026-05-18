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
//! * Stable exchange id: `<session_id>:<turn_index>` where
//!   `session_id` is the file stem (matches the existing
//!   `sessions.session_id` derivation) and `turn_index` is the
//!   0-based ordinal of the user prompt in the file.
//!
//! * Stable tool-call id: `<exchange_id>\u{001f}<tool_use_id>`,
//!   matching the L3 unit-separator convention used by the Codex
//!   indexer (so a `tool_use_id` containing `:` doesn't collide
//!   with the exchange-id separator).
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
/// `exchanges` + `tool_calls` for any Claude `sessions` row whose
/// `(file_size_bytes, file_mtime_ns, file_inode)` triple has moved
/// since the last exchange-write, OR has zero `exchanges` rows.
///
/// `claude_config_dir` is typically `~/.claude` (or the
/// `CLAUDE_CONFIG_DIR` override). The function appends `projects/`
/// itself so callers can pass the literal config dir.
///
/// Doesn't touch the `sessions` row staleness tuple — that's owned
/// by `session_index::refresh`. Instead, this module tracks its
/// own "last exchanged" state per `file_path` via the existing
/// row's tuple. If the tuple in `sessions` has moved since the
/// exchanges were written, we re-write. The signal "exchanges
/// already populated for this file" is: a non-zero count in
/// `exchanges WHERE file_path = ?`.
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

    // 2. Load the existing exchange-tuple state for every Claude
    //    row in the cache: (file_path, size, mtime, inode,
    //    exchanges_row_count).
    let existing = load_claude_exchange_state(&tx)?;

    for entry in &discovered {
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

#[cfg(unix)]
fn inode_of(meta: &fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino() as i64
}

#[cfg(windows)]
fn inode_of(meta: &fs::Metadata) -> i64 {
    // Windows has no inode in the POSIX sense. `MetadataExt::file_index()`
    // returns the NTFS file id (the closest analog) but is gated behind
    // `#![feature(windows_by_handle)]` — using it on stable rustc breaks
    // the Windows release build (E0658, rust-lang/rust#63010). What this
    // value is used for: equality inside the (size, mtime_ns, inode)
    // staleness triple in `sessions.db` — purely "is this the same file
    // I indexed before?" `creation_time()` (stable since 1.1) is a
    // strictly safer proxy: it changes when a file is created or
    // replaced, stays constant across in-place modifications. Cast to
    // i64 (matches the column type); signedness doesn't matter because
    // the consumer only ever uses `==`. v0.1.36 was the first release
    // to hit this path and the release-build failed on Windows; this
    // is the fix.
    use std::os::windows::fs::MetadataExt;
    meta.creation_time() as i64
}

#[cfg(not(any(unix, windows)))]
fn inode_of(_meta: &fs::Metadata) -> i64 {
    0
}

/// Per-file state we use to decide skip-vs-reindex. The triple is
/// the same one the `sessions` row already stores; we treat it as
/// authoritative. If a previously-indexed Claude file has zero
/// `exchanges` rows AND a matching tuple, we re-index regardless
/// (catches the first-ever run of this module against a populated
/// v4 DB).
fn load_claude_exchange_state(
    tx: &rusqlite::Transaction<'_>,
) -> Result<std::collections::HashMap<String, (i64, i64, i64, i64)>, rusqlite::Error> {
    let mut stmt = tx.prepare(
        "SELECT s.file_path, s.file_size_bytes, s.file_mtime_ns, s.file_inode, \
                COUNT(e.id) \
         FROM sessions s \
         LEFT JOIN exchanges e ON e.file_path = s.file_path \
         WHERE s.source_kind = 'claude_code' \
         GROUP BY s.file_path",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        let s: i64 = row.get(1)?;
        let m: i64 = row.get(2)?;
        let i: i64 = row.get(3)?;
        let c: i64 = row.get(4)?;
        out.insert(path, (s, m, i, c));
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
    if let Some((size, mtime, inode, ex_count)) = existing {
        // Skip when (a) the staleness tuple matches AND (b) the
        // exchanges table already has rows for this file. The
        // second condition catches the first-ever run of this
        // module against an existing v4 DB where `sessions` was
        // populated but `exchanges` wasn't.
        if *size == entry.size && *mtime == entry.mtime_ns && *inode == entry.inode && *ex_count > 0
        {
            return Ok(Outcome::Skipped);
        }
    } else {
        // File isn't in `sessions` yet — `session_index::refresh`
        // hasn't seen it. Skip for this run; the user should run
        // their normal refresh first. We avoid the failure path
        // because this isn't a Claude exchange bug, it's a setup
        // ordering issue.
        return Ok(Outcome::Skipped);
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

    // Pair events into exchanges.
    let exchanges = pair_events_into_exchanges(&session_id, &events);

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

        for tc in &ex.tool_calls {
            let tc_id = format!("{}\u{001f}{}", ex.id, tc.tool_use_id);
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
