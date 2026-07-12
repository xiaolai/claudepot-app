//! Codex backfill + incremental indexer (WI-003).
//!
//! Walks `$CODEX_HOME/sessions/**/*.jsonl`, parses each rollout via
//! `crate::codex_session::parse_codex_rollout_jsonl`, and upserts
//! into `sessions` (source_kind='codex'), `exchanges`, and
//! `tool_calls`. The `exchange_fts` virtual table is maintained
//! transparently by the AFTER INSERT/DELETE/UPDATE triggers on
//! `exchanges`.
//!
//! Incremental semantics mirror the existing Claude path:
//! `(file_size_bytes, file_mtime_ns, file_inode)` is the re-parse
//! guard. Rows whose tuple still matches the on-disk file are
//! skipped without parsing.
//!
//! Claude-side exchange population is deferred to a follow-up
//! commit — the existing `session_index::refresh` already writes
//! `sessions` rows for Claude, but does not yet emit `exchanges`.
//! WI-004 (search) initially queries Codex-only rows; the Claude
//! follow-up lights up the unified-search story.
//!
//! All raw text columns are written unredacted at rest per R9 in
//! the plan — `~/.claudepot/sessions.db` is the at-rest trust
//! boundary, mirroring the source `.jsonl` permissions. Snippet
//! generation for emission surfaces is the caller's responsibility
//! (WI-004 / WI-008).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::params;

use crate::codex_session::{parse_codex_rollout_jsonl, CodexConversation};
use crate::session_index::SessionIndex;

/// Result of `forget_shared_memory` — row counts removed.
#[derive(Debug, Default, Clone)]
pub struct ForgetReport {
    pub exchanges: i64,
    pub tool_calls: i64,
    pub exchange_fts: i64,
    pub memories: i64,
    pub decisions: i64,
    pub evidence_records: i64,
    pub memory_links: i64,
}

/// Set the `_pending_rescan` marker. The next `SessionIndex::open`
/// clears the transcript-derived cache atomically inside the
/// migration transaction; `refresh()` repopulates from disk.
///
/// Durable rows (memories, decisions, evidence, links) are NOT
/// affected. Use `forget_shared_memory` to clear those.
///
/// The marker is checked by `apply_schema`; this function just
/// writes it. Opening the DB through `SessionIndex::open` would
/// also trigger an immediate rescan in the current process, which
/// callers usually don't want — they typically follow this with
/// `claudepot codex index` to drive the rescan deliberately.
pub fn mark_pending_rescan(db_path: &Path) -> Result<(), rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    crate::db_pragmas::apply_standard_pragmas(&conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (k, v) VALUES ('_pending_rescan', '1')",
        [],
    )?;
    Ok(())
}

/// Count rows in the Shared Memory tables — both transcript-
/// derived and durable. Used by the CLI `forget` verb to print a
/// dry-run summary before the destructive path.
pub fn count_shared_memory_rows(db_path: &Path) -> Result<ForgetReport, rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    crate::db_pragmas::apply_standard_pragmas(&conn)?;
    let count = |t: &str| -> Result<i64, rusqlite::Error> {
        conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
    };
    Ok(ForgetReport {
        exchanges: count("exchanges")?,
        tool_calls: count("tool_calls")?,
        exchange_fts: count("exchange_fts")?,
        memories: count("memories")?,
        decisions: count("decisions")?,
        evidence_records: count("evidence_records")?,
        memory_links: count("memory_links")?,
    })
}

/// Wipe all Shared Memory rows in a single transaction. Drops:
/// - Transcript-derived: `exchanges`, `tool_calls` (the FTS table
///   is rebuilt via FTS5's `'rebuild'` command after the cascade).
/// - Durable: `memories`, `decisions`, `evidence_records`,
///   `memory_links`.
///
/// The v4 schema stays in place — no DROP TABLE. After this runs,
/// the `_pending_rescan` marker is set so the next index run
/// repopulates the transcript-derived rows from disk. Durable
/// rows are gone for good.
///
/// `PRAGMA foreign_keys=ON` is set on the local connection so the
/// `DELETE FROM exchanges` cascade fires correctly even if the
/// caller's connection has FKs off.
pub fn forget_shared_memory(db_path: &Path) -> Result<ForgetReport, rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    crate::db_pragmas::apply_standard_pragmas(&conn)?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    let counts = count_shared_memory_rows_impl(&conn)?;

    let tx = conn.unchecked_transaction()?;
    // Transcript-derived. FK cascade from exchanges drops tool_calls
    // rows and the FTS trigger drops corresponding exchange_fts rows.
    tx.execute("DELETE FROM exchanges", [])?;
    // Defensive: explicitly clear tool_calls in case FKs were
    // somehow not in effect.
    tx.execute("DELETE FROM tool_calls", [])?;
    // Force-rebuild the FTS index from scratch as a belt-and-
    // suspenders cleanup in case any orphaned FTS rows slipped past.
    tx.execute_batch("INSERT INTO exchange_fts(exchange_fts) VALUES('rebuild');")?;
    // Durable rows.
    tx.execute("DELETE FROM memory_links", [])?;
    tx.execute("DELETE FROM evidence_records", [])?;
    tx.execute("DELETE FROM decisions", [])?;
    tx.execute("DELETE FROM memories", [])?;
    // Mark for rescan on next open.
    tx.execute(
        "INSERT OR REPLACE INTO meta (k, v) VALUES ('_pending_rescan', '1')",
        [],
    )?;
    tx.commit()?;
    Ok(counts)
}

fn count_shared_memory_rows_impl(
    conn: &rusqlite::Connection,
) -> Result<ForgetReport, rusqlite::Error> {
    let count = |t: &str| -> Result<i64, rusqlite::Error> {
        conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
    };
    Ok(ForgetReport {
        exchanges: count("exchanges")?,
        tool_calls: count("tool_calls")?,
        exchange_fts: count("exchange_fts")?,
        memories: count("memories")?,
        decisions: count("decisions")?,
        evidence_records: count("evidence_records")?,
        memory_links: count("memory_links")?,
    })
}

/// Tally of what one `backfill_codex` run did.
#[derive(Debug, Default, Clone)]
pub struct CodexIndexerStats {
    /// Files discovered under `$CODEX_HOME/sessions`.
    pub discovered: usize,
    /// Files re-parsed because their staleness triple changed.
    pub indexed: usize,
    /// Files skipped because the triple still matched the cache.
    pub skipped_unchanged: usize,
    /// Files dropped from the cache because they disappeared on disk.
    pub deleted: usize,
    /// Per-file failures: `(path, error_string)`.
    pub failed: Vec<(PathBuf, String)>,
}

/// Walk `$CODEX_HOME/sessions/**/*.jsonl` and synchronize the
/// `sessions` cache with what's on disk. Pure synchronous I/O —
/// callers running under tokio can wrap in `spawn_blocking`.
///
/// `codex_sessions_root` is the absolute path to the
/// `<CODEX_HOME>/sessions` directory. The function does not
/// resolve `CODEX_HOME` itself; that policy belongs to the caller
/// (CLI / Tauri command) which has access to env vars.
pub fn backfill_codex(
    idx: &SessionIndex,
    codex_sessions_root: &Path,
) -> Result<CodexIndexerStats, rusqlite::Error> {
    let mut stats = CodexIndexerStats::default();

    let discovered = walk_codex_sessions(codex_sessions_root, &mut stats);
    stats.discovered = discovered.len();

    let db = idx.db();
    // Single outer transaction for atomic apply. Each per-file
    // write is wrapped in a SAVEPOINT (M15) so that a per-file
    // failure (e.g. PRIMARY KEY collision in tool_calls) only
    // rolls back that file's writes, not the entire batch.
    let tx = db.unchecked_transaction()?;

    // Load existing Codex cache state.
    let existing: std::collections::HashMap<String, (i64, i64, i64)> =
        load_codex_cache_tuples(&tx)?;

    // Index pass.
    for entry in &discovered {
        let previously_indexed = existing.contains_key(&entry.file_path);
        match upsert_codex_session_in_savepoint(&tx, entry, existing.get(&entry.file_path)) {
            Ok(IndexOutcome::Indexed) => stats.indexed += 1,
            Ok(IndexOutcome::Skipped) => stats.skipped_unchanged += 1,
            Err(e) => {
                tracing::warn!(
                    path = %entry.file_path,
                    error = %e,
                    previously_indexed,
                    "shared_memory: codex backfill error (savepoint rolled back)"
                );
                stats.failed.push((PathBuf::from(&entry.file_path), e));
                // H6 — stale-row cleanup: if a previously-indexed
                // file fails to re-parse / re-write now, the old
                // `sessions` row + cascade rows would otherwise
                // keep pointing at content that no longer matches
                // disk. Force-delete here so search results
                // reflect on-disk truth.
                //
                // The DELETE runs in the outer transaction (not
                // the rolled-back savepoint), so it persists at
                // outer commit.
                if previously_indexed {
                    if let Err(e2) = tx.execute(
                        "DELETE FROM sessions WHERE file_path = ?1 AND source_kind = 'codex'",
                        [&entry.file_path],
                    ) {
                        tracing::warn!(
                            path = %entry.file_path,
                            error = %e2,
                            "shared_memory: failed to clear stale cache row after parse failure"
                        );
                    }
                }
            }
        }
    }

    // Reap cache rows whose file vanished from disk. FK cascade
    // drops the corresponding `exchanges` / `tool_calls` /
    // `exchange_fts` rows automatically (PRAGMA foreign_keys=ON
    // since v4 → enforces).
    let on_disk: std::collections::HashSet<&str> =
        discovered.iter().map(|e| e.file_path.as_str()).collect();
    for path in existing.keys() {
        if !on_disk.contains(path.as_str()) {
            tx.execute(
                "DELETE FROM sessions WHERE file_path = ?1 AND source_kind = 'codex'",
                [path],
            )?;
            stats.deleted += 1;
        }
    }

    tx.commit()?;
    Ok(stats)
}

/// Run `upsert_codex_session` inside a SAVEPOINT. On success the
/// savepoint is released (merged into the outer txn); on failure
/// it's rolled back so the per-file error doesn't poison the
/// batch.
///
/// SQLite SAVEPOINT names must be ASCII identifiers; we use a
/// fixed name (`codex_upsert`) since SAVEPOINTs nest by stack order
/// and we never have two nested codex upserts at the same depth.
fn upsert_codex_session_in_savepoint(
    tx: &rusqlite::Transaction<'_>,
    entry: &CodexDiscovery,
    existing: Option<&(i64, i64, i64)>,
) -> Result<IndexOutcome, String> {
    tx.execute_batch("SAVEPOINT codex_upsert")
        .map_err(|e| format!("savepoint: {e}"))?;
    match upsert_codex_session(tx, entry, existing) {
        Ok(outcome) => {
            tx.execute_batch("RELEASE codex_upsert")
                .map_err(|e| format!("release: {e}"))?;
            Ok(outcome)
        }
        Err(e) => {
            // Best-effort rollback. If this also fails, we propagate
            // the original parse/write error to the caller; the
            // outer transaction will fail on next write, which is
            // acceptable degradation for what should be a rare path.
            let _ = tx.execute_batch("ROLLBACK TO codex_upsert; RELEASE codex_upsert");
            Err(e)
        }
    }
}

// ─── walk ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CodexDiscovery {
    file_path: String,
    size: i64,
    mtime_ns: i64,
    inode: i64,
}

fn walk_codex_sessions(root: &Path, stats: &mut CodexIndexerStats) -> Vec<CodexDiscovery> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    walk_dir_recursive(root, &mut out, stats, 0);
    out
}

fn walk_dir_recursive(
    dir: &Path,
    out: &mut Vec<CodexDiscovery>,
    stats: &mut CodexIndexerStats,
    depth: usize,
) {
    // Cap depth so a runaway symlink doesn't recurse forever.
    // Codex's layout is sessions/YYYY/MM/DD/file.jsonl → depth 4.
    if depth > 8 {
        // L7 — surface the depth cap as a warn log so a future
        // Codex layout that pushes past the cap doesn't silently
        // drop files.
        tracing::warn!(
            depth,
            dir = %dir.display(),
            "codex_session indexer: depth cap reached; subdirectories skipped"
        );
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
        // M4 — symlink containment. `DirEntry::metadata()` does
        // NOT follow symlinks (it's effectively `symlink_metadata`
        // on every supported platform — see std::fs::DirEntry docs).
        // So calling `.file_type().is_symlink()` on its result
        // detects symlink entries and lets us skip them. Reasoning:
        // a symlink under $CODEX_HOME/sessions/ pointing at e.g.
        // /etc/passwd or ~/.config/Claude/.credentials would
        // otherwise be indexed → readable via
        // `claudepot_read_conversation` because the cache row
        // "exists" → arbitrary file disclosure within the MCP user's
        // own privileges. Symlinks under the sessions root are not
        // a known Codex pattern, so refusing them is safe.
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            tracing::warn!(
                path = %path.display(),
                "codex_session indexer: skipping symlink (M4 containment)"
            );
            continue;
        }
        if meta.is_dir() {
            walk_dir_recursive(&path, out, stats, depth + 1);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        // Codex rollouts are `.jsonl`. Skip anything else.
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
        out.push(CodexDiscovery {
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

// ─── load cache ───────────────────────────────────────────────

fn load_codex_cache_tuples(
    tx: &rusqlite::Transaction<'_>,
) -> Result<std::collections::HashMap<String, (i64, i64, i64)>, rusqlite::Error> {
    let mut stmt = tx.prepare(
        "SELECT file_path, file_size_bytes, file_mtime_ns, file_inode \
         FROM sessions WHERE source_kind = 'codex'",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        let s: i64 = row.get(1)?;
        let m: i64 = row.get(2)?;
        let i: i64 = row.get(3)?;
        out.insert(path, (s, m, i));
    }
    Ok(out)
}

// ─── upsert ───────────────────────────────────────────────────

enum IndexOutcome {
    Indexed,
    Skipped,
}

fn upsert_codex_session(
    tx: &rusqlite::Transaction<'_>,
    entry: &CodexDiscovery,
    existing: Option<&(i64, i64, i64)>,
) -> Result<IndexOutcome, String> {
    if let Some((size, mtime, inode)) = existing {
        if *size == entry.size && *mtime == entry.mtime_ns && *inode == entry.inode {
            return Ok(IndexOutcome::Skipped);
        }
    }

    // Parse outside the SQL portion. Errors propagate as strings
    // so the indexer's `failed` list can carry them.
    let conv = parse_codex_rollout_jsonl(Path::new(&entry.file_path))
        .map_err(|e| format!("parse: {e}"))?;

    // M14 — TOCTOU mitigation. The staleness triple we want to
    // stamp must reflect the file's state AFTER parsing, not
    // before. If the file grew between the walk's stat and the
    // parser's read (a live Codex session appended bytes), the
    // walk's triple no longer matches disk; stamping it would mean
    // the next backfill mistakenly skips the file. Re-stat now and
    // use the post-parse triple. Net effect: we converge to truth
    // on the next backfill if any drift was observed.
    let post_parse_entry = match restat_after_parse(entry) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                path = %entry.file_path,
                error = %e,
                "shared_memory: failed to re-stat after parse; using pre-parse tuple"
            );
            entry.clone()
        }
    };

    // H6 — partial-parse stickiness mitigation. If the parser
    // saw a mid-stream I/O error, the conversation we have is
    // incomplete. Refuse to stamp the staleness triple so the
    // next backfill retries; without this, the incomplete row
    // would persist indefinitely because `(size, mtime, inode)`
    // matches and the file is skipped.
    if conv.diagnostics.truncated_by_io {
        return Err(format!(
            "parse truncated by I/O error (malformed={}, oversize={}); not stamping cache",
            conv.diagnostics.malformed_lines, conv.diagnostics.oversize_lines
        ));
    }

    write_codex_conversation(tx, &post_parse_entry, &conv).map_err(|e| format!("write: {e}"))?;
    Ok(IndexOutcome::Indexed)
}

fn restat_after_parse(entry: &CodexDiscovery) -> Result<CodexDiscovery, std::io::Error> {
    let meta = fs::metadata(&entry.file_path)?;
    let size = meta.len() as i64;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let inode = inode_of(&meta);
    Ok(CodexDiscovery {
        file_path: entry.file_path.clone(),
        size,
        mtime_ns,
        inode,
    })
}

fn write_codex_conversation(
    tx: &rusqlite::Transaction<'_>,
    entry: &CodexDiscovery,
    conv: &CodexConversation,
) -> Result<(), rusqlite::Error> {
    let session_id = conv.head.session_id.clone();
    let project_path = conv
        .head
        .cwd
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug = derive_slug(&entry.file_path);
    let first_ts = conv
        .exchanges
        .iter()
        .filter_map(|e| e.timestamp)
        .map(|t| t.timestamp_millis())
        .min();
    let last_ts = conv
        .exchanges
        .iter()
        .filter_map(|e| e.timestamp)
        .map(|t| t.timestamp_millis())
        .max();
    let event_count = (conv
        .exchanges
        .iter()
        .map(|e| 1 + e.tool_calls.len())
        .sum::<usize>()) as i64;
    let message_count = conv.exchanges.len() as i64 * 2; // user + assistant per turn
    let user_message_count = conv.exchanges.len() as i64;
    let assistant_message_count = conv.exchanges.len() as i64;
    let indexed_at = chrono::Utc::now().timestamp_millis();

    // Upsert the sessions row. Caller has guaranteed PRAGMA
    // foreign_keys=ON, so DELETE-on-replace cascades to exchanges
    // before the new INSERT lands.
    //
    // ON CONFLICT(file_path) DO UPDATE: a code path that wants
    // forward migration of an existing Claude-shape row would
    // be unusual but should be safe — the source_kind column
    // change makes the row drift visible.
    tx.execute(
        "INSERT INTO sessions (
            file_path, slug, session_id,
            file_size_bytes, file_mtime_ns, file_inode,
            project_path, project_from_transcript,
            first_ts_ms, last_ts_ms,
            event_count, message_count, user_message_count, assistant_message_count,
            first_user_prompt, models_json,
            tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
            git_branch, cc_version, display_slug, has_error, is_sidechain,
            indexed_at_ms, source_kind
        ) VALUES (
            ?1, ?2, ?3,
            ?4, ?5, ?6,
            ?7, 1,
            ?8, ?9,
            ?10, ?11, ?12, ?13,
            ?14, '[]',
            0, 0, 0, 0,
            NULL, ?15, NULL, 0, 0,
            ?16, 'codex'
        )
        ON CONFLICT(file_path) DO UPDATE SET
            slug = excluded.slug,
            session_id = excluded.session_id,
            file_size_bytes = excluded.file_size_bytes,
            file_mtime_ns = excluded.file_mtime_ns,
            file_inode = excluded.file_inode,
            project_path = excluded.project_path,
            project_from_transcript = excluded.project_from_transcript,
            first_ts_ms = excluded.first_ts_ms,
            last_ts_ms = excluded.last_ts_ms,
            event_count = excluded.event_count,
            message_count = excluded.message_count,
            user_message_count = excluded.user_message_count,
            assistant_message_count = excluded.assistant_message_count,
            first_user_prompt = excluded.first_user_prompt,
            cc_version = excluded.cc_version,
            indexed_at_ms = excluded.indexed_at_ms,
            source_kind = excluded.source_kind",
        params![
            entry.file_path,
            slug,
            session_id,
            entry.size,
            entry.mtime_ns,
            entry.inode,
            project_path,
            first_ts,
            last_ts,
            event_count,
            message_count,
            user_message_count,
            assistant_message_count,
            conv.exchanges.first().map(|e| e.user_text.as_str()),
            conv.head.cli_version,
            indexed_at,
        ],
    )?;

    // Wipe + reinsert the per-file exchanges. FK cascade plus FTS
    // AFTER DELETE trigger keep `exchange_fts` and `tool_calls`
    // in sync without explicit work here.
    tx.execute(
        "DELETE FROM exchanges WHERE file_path = ?1",
        [&entry.file_path],
    )?;

    for ex in &conv.exchanges {
        let ts_ms = ex.timestamp.map(|t| t.timestamp_millis());
        let snippet = build_snippet(&ex.user_text, &ex.assistant_text);
        tx.execute(
            "INSERT INTO exchanges (
                id, file_path, source_kind, turn_index, role_pair,
                timestamp_ms, user_text, assistant_text,
                line_start, line_end, is_sidechain, parent_id, snippet_text
            ) VALUES (
                ?1, ?2, 'codex', ?3, 'user_assistant',
                ?4, ?5, ?6,
                ?7, ?8, 0, NULL, ?9
            )",
            params![
                ex.id,
                entry.file_path,
                ex.turn_index,
                ts_ms,
                ex.user_text,
                ex.assistant_text,
                ex.line_start,
                ex.line_end,
                snippet,
            ],
        )?;

        for tc in &ex.tool_calls {
            let tc_ts = tc.timestamp.map(|t| t.timestamp_millis());
            // Tool call id stable across reparse. L3 — use ASCII
            // unit-separator (U+001F) instead of `:` so the
            // composite key is unambiguous even if Codex emits a
            // call_id containing `:`. Unit-separator is in the
            // legacy ASCII control range and won't appear in any
            // Codex-generated identifier (which is hex / printable).
            let tc_id = format!("{}\u{001f}{}", ex.id, tc.call_id);
            tx.execute(
                "INSERT INTO tool_calls (
                    id, exchange_id, tool_name, tool_input_json,
                    tool_result_text, is_error, timestamp_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    tc_id,
                    ex.id,
                    tc.name,
                    tc.arguments,
                    tc.output,
                    tc.is_error as i64,
                    tc_ts,
                ],
            )?;
        }
    }

    Ok(())
}

/// Produce a stable, distinct slug for `file_path`. The common
/// case — a clean UTF-8 file stem — returns the stem verbatim.
///
/// L9 — `walk_dir_recursive` resolves paths via
/// `Path::to_string_lossy()`, which replaces invalid UTF-8 bytes
/// with U+FFFD. Two distinct paths with non-UTF-8 bytes at
/// different positions both lossily render to the same U+FFFD
/// sequence — pre-fix, their slugs collided into the literal
/// "codex-session", making them indistinguishable in the UI.
/// Post-fix: any stem containing U+FFFD (or absent / empty)
/// falls back to a hex digest of the full path bytes so distinct
/// inputs always get distinct slugs.
///
/// The slug is stable across reparses (deterministic hash) and
/// short enough for a UI column (16 hex chars = 64 bits of
/// entropy; collisions are not a concern).
fn derive_slug(file_path: &str) -> String {
    if let Some(stem) = Path::new(file_path).file_stem().and_then(|s| s.to_str()) {
        if !stem.is_empty() && !stem.contains('\u{fffd}') {
            return stem.to_string();
        }
    }
    // Fallback path: hash the full path so distinct non-UTF-8 or
    // empty-stem names get distinct slugs.
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(file_path.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("codex-session-{hex}")
}

/// Build the pre-emission snippet column. v1: first 240 chars of
/// `user_text` joined to the first 240 chars of `assistant_text`,
/// then run through `redaction::apply` so the stored column is
/// already redacted at rest. The schema comment promises a
/// "pre-redacted preview"; this function makes that promise true.
///
/// Later read paths (search, MCP) still pass the column through
/// `redaction::apply` again as defense in depth — a stricter
/// emission policy may catch what the at-rest policy let through.
/// But a future caller that reads `exchanges.snippet_text`
/// directly (CLI dump, debug surface, backup tool) gets the
/// redacted form, not raw tokens.
fn build_snippet(user: &str, assistant: &str) -> String {
    const CAP: usize = 240;
    let head = truncate_chars(user, CAP);
    let tail = truncate_chars(assistant, CAP);
    let combined = if head.is_empty() {
        tail
    } else if tail.is_empty() {
        head
    } else {
        format!("{head}\n→ {tail}")
    };
    // Run the at-rest redaction policy. Default masks sk-ant-*
    // tokens; future tightening of `RedactionPolicy::default()`
    // (e.g. opt-in env-assignment masking) would propagate here
    // automatically.
    crate::redaction::apply(&combined, &crate::redaction::RedactionPolicy::default())
}

/// Truncate `s` to at most `cap` graphemes (extended grapheme
/// clusters per UAX#29), appending `…` if truncation occurred.
///
/// L10 — uses `unicode_segmentation::UnicodeSegmentation::graphemes`
/// rather than `chars()`. The `chars()` approach splits multi-
/// codepoint graphemes (emoji + skin-tone modifier, ZWJ
/// sequences like 👨‍👩‍👧, regional indicators like 🇨🇳) at codepoint
/// boundaries, producing broken visual output at the snippet cap.
/// Grapheme segmentation respects user-perceived characters.
fn truncate_chars(s: &str, cap: usize) -> String {
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

    fn open_idx(dir: &TempDir) -> SessionIndex {
        SessionIndex::open(&dir.path().join("sessions.db")).expect("open")
    }

    fn write_rollout(root: &Path, day_dir: &str, name: &str, body: &str) -> PathBuf {
        let dir = root.join(day_dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    fn sample_rollout_body(session_id: &str, prompt: &str, answer: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{{"id":"{session_id}","timestamp":"2026-05-15T11:30:00.000Z","cwd":"/Users/jane/proj","originator":"codex_cli","cli_version":"0.44.0"}}}}
{{"timestamp":"2026-05-15T11:30:00.100Z","type":"turn_context","payload":{{"cwd":"/Users/jane/proj","approval_policy":"on-request","sandbox_policy":{{"mode":"workspace-write"}}}}}}
{{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{prompt}"}}]}}}}
{{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{answer}"}}]}}}}
"#,
        )
    }

    fn open_raw(path: &Path) -> Connection {
        let c = Connection::open(path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        c
    }

    #[test]
    fn backfill_fresh_corpus_indexes_codex_rollouts() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout-a.jsonl",
            &sample_rollout_body("01-a", "first prompt", "first answer"),
        );
        write_rollout(
            &codex_root,
            "2026/05/16",
            "rollout-b.jsonl",
            &sample_rollout_body("01-b", "second prompt", "second answer"),
        );

        let stats = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(stats.discovered, 2);
        assert_eq!(stats.indexed, 2);
        assert_eq!(stats.skipped_unchanged, 0);
        assert_eq!(stats.deleted, 0);
        assert!(stats.failed.is_empty());

        let db = open_raw(&tmp.path().join("sessions.db"));
        let sessions: i64 = db
            .query_row(
                "SELECT count(*) FROM sessions WHERE source_kind = 'codex'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 2);
        let exchanges: i64 = db
            .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(exchanges, 2);
        let fts: i64 = db
            .query_row("SELECT count(*) FROM exchange_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts, 2, "FTS trigger should populate one row per exchange");
    }

    #[test]
    fn second_backfill_skips_unchanged_files() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout.jsonl",
            &sample_rollout_body("01", "hello", "hi"),
        );

        let s1 = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(s1.indexed, 1);

        let s2 = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(s2.discovered, 1);
        assert_eq!(s2.indexed, 0);
        assert_eq!(s2.skipped_unchanged, 1);
    }

    #[test]
    fn rewritten_file_is_reindexed_with_no_duplicates() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        let p = write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout.jsonl",
            &sample_rollout_body("01", "v1 prompt", "v1 answer"),
        );

        backfill_codex(&idx, &codex_root).unwrap();
        // Touch + rewrite with a different body so size/mtime
        // diverge from the cache tuple.
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&p, sample_rollout_body("01", "v2 prompt", "v2 answer")).unwrap();
        let stats = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(stats.indexed, 1, "rewritten file should be re-parsed");

        let db = open_raw(&tmp.path().join("sessions.db"));
        let exchanges: i64 = db
            .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(exchanges, 1, "no duplicate exchange rows after re-index");
        let user_text: String = db
            .query_row("SELECT user_text FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(user_text, "v2 prompt", "exchange should reflect v2 content");
    }

    #[test]
    fn deleted_files_drop_from_cache_and_cascade() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        let p = write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout.jsonl",
            &sample_rollout_body("01", "x", "y"),
        );

        backfill_codex(&idx, &codex_root).unwrap();
        let db = open_raw(&tmp.path().join("sessions.db"));
        let before: i64 = db
            .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, 1);

        fs::remove_file(&p).unwrap();
        let stats = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(stats.discovered, 0);
        assert_eq!(stats.deleted, 1);

        // FK cascade should have dropped the exchanges row, and the
        // AFTER DELETE trigger should have dropped the FTS row.
        let after_ex: i64 = db
            .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after_ex, 0);
        let after_fts: i64 = db
            .query_row("SELECT count(*) FROM exchange_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after_fts, 0);
    }

    #[test]
    fn tool_calls_persisted_with_stable_ids() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        let body = r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"01-tools","cwd":"/x","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"run shell"}]}}
{"timestamp":"2026-05-15T11:30:01.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"ls\"}","call_id":"call-a"}}
{"timestamp":"2026-05-15T11:30:01.500Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call-a","output":"{\"output\":\"ok\",\"metadata\":{\"exit_code\":0}}"}}
{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}
"#;
        write_rollout(&codex_root, "2026/05/15", "rollout.jsonl", body);
        backfill_codex(&idx, &codex_root).unwrap();

        let db = open_raw(&tmp.path().join("sessions.db"));
        let tc_id: String = db
            .query_row("SELECT id FROM tool_calls", [], |r| r.get(0))
            .unwrap();
        // L3 — composite key uses unit-separator (U+001F), not `:`.
        // Format: <source_kind>:<session_id>:<turn_index>\u{001f}<call_id>.
        // (source_kind prefix added in v5 to avoid cross-harness
        // session_id collisions.)
        assert_eq!(tc_id, "codex:01-tools:0\u{001f}call-a");
        let tc_name: String = db
            .query_row("SELECT tool_name FROM tool_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tc_name, "shell");
    }

    #[test]
    fn parse_failure_clears_stale_cache_row() {
        // H6 — when a previously-indexed file becomes unparseable,
        // the indexer must remove the stale `sessions` row so
        // search results don't keep pointing at the old content.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        let p = write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout.jsonl",
            &sample_rollout_body("01", "first prompt", "first answer"),
        );

        // First backfill: clean index.
        backfill_codex(&idx, &codex_root).unwrap();
        {
            let db = open_raw(&tmp.path().join("sessions.db"));
            let n: i64 = db
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1);
        }

        // Corrupt the file — replace contents with non-JSONL.
        // Bump mtime so the staleness guard triggers a re-parse.
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&p, "not json at all\nstill not json\n").unwrap();

        let stats = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(
            stats.failed.len(),
            1,
            "corrupted file should be in failed list"
        );

        // Stale row must be gone — H6 cleanup.
        let db = open_raw(&tmp.path().join("sessions.db"));
        let n: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            n, 0,
            "stale sessions row must be removed after parse failure"
        );
    }

    #[test]
    fn savepoint_isolates_per_file_failures() {
        // M15 — a single bad file must not abort the whole tick's
        // transaction. Stage one good file + one bad file and
        // verify the good one persists.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        write_rollout(
            &codex_root,
            "2026/05/15",
            "good.jsonl",
            &sample_rollout_body("01-good", "good prompt", "good answer"),
        );
        // Bad: looks like a Codex rollout but session_meta lacks
        // payload.id, triggering MissingSessionMeta after parse_head.
        write_rollout(
            &codex_root,
            "2026/05/15",
            "bad.jsonl",
            r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"cwd":"/x"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"a"}]}}
"#,
        );

        let stats = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(stats.discovered, 2);
        assert_eq!(stats.indexed, 1, "one file should index cleanly");
        assert_eq!(stats.failed.len(), 1, "one file should fail");

        let db = open_raw(&tmp.path().join("sessions.db"));
        let session_ids: Vec<String> = db
            .prepare("SELECT session_id FROM sessions ORDER BY session_id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            session_ids,
            vec!["01-good".to_string()],
            "good file should persist despite bad file's savepoint rollback"
        );
    }

    #[test]
    fn mark_pending_rescan_sets_marker_for_next_open() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout.jsonl",
            &sample_rollout_body("01", "hello", "hi"),
        );
        backfill_codex(&idx, &codex_root).unwrap();
        // Sanity: row present in exchanges.
        {
            let db = open_raw(&tmp.path().join("sessions.db"));
            let n: i64 = db
                .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1);
        }
        // Drop the SessionIndex so the rusqlite connection lock
        // releases; mark_pending_rescan opens its own.
        drop(idx);

        let db_path = tmp.path().join("sessions.db");
        mark_pending_rescan(&db_path).unwrap();

        // The marker is set; the next SessionIndex::open triggers
        // a rescan inside apply_schema, clearing sessions +
        // session_turns. After the rescan branch runs,
        // refresh() would repopulate from disk — but we don't
        // run refresh here, just verify the cache was cleared.
        let _idx2 = SessionIndex::open(&db_path).expect("reopen");
        let db = open_raw(&db_path);
        let n: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "_pending_rescan marker should clear sessions on open");
        // Marker itself was unset by apply_schema.
        let marker: Option<String> = db
            .query_row("SELECT v FROM meta WHERE k = '_pending_rescan'", [], |r| {
                r.get(0)
            })
            .ok();
        assert!(marker.is_none(), "marker should be cleared after handling");
    }

    #[test]
    fn forget_shared_memory_wipes_all_rows_and_sets_marker() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        write_rollout(
            &codex_root,
            "2026/05/15",
            "rollout.jsonl",
            &sample_rollout_body("01", "user said", "assistant said"),
        );
        backfill_codex(&idx, &codex_root).unwrap();

        // Seed durable rows so forget has something to clear.
        {
            let db = open_raw(&tmp.path().join("sessions.db"));
            db.execute(
                "INSERT INTO memories (id, scope, project_path, kind, content,
                    created_by_kind, created_by, confidence,
                    created_at_ms, updated_at_ms, archived_at_ms)
                 VALUES ('m1','global',NULL,'fact','x','user','u',NULL,1,1,NULL)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO decisions (id, project_path, topic, decision, rationale,
                    status, created_by_kind, created_by, created_at_ms, supersedes_id)
                 VALUES ('d1',NULL,'t','dec',NULL,'active','user','u',1,NULL)",
                [],
            )
            .unwrap();
        }

        drop(idx);
        let db_path = tmp.path().join("sessions.db");

        // Pre-counts.
        let pre = count_shared_memory_rows(&db_path).unwrap();
        assert!(pre.exchanges > 0);
        assert_eq!(pre.memories, 1);
        assert_eq!(pre.decisions, 1);

        let report = forget_shared_memory(&db_path).unwrap();
        // Report returns the counts AS THEY WERE before deletion
        // (so callers can show "X rows removed").
        assert_eq!(report.exchanges, pre.exchanges);
        assert_eq!(report.memories, 1);
        assert_eq!(report.decisions, 1);

        // Post-state: all rows gone.
        let post = count_shared_memory_rows(&db_path).unwrap();
        assert_eq!(post.exchanges, 0);
        assert_eq!(post.tool_calls, 0);
        assert_eq!(post.exchange_fts, 0);
        assert_eq!(post.memories, 0);
        assert_eq!(post.decisions, 0);
        assert_eq!(post.evidence_records, 0);
        assert_eq!(post.memory_links, 0);

        // Marker set for next open.
        let db = open_raw(&db_path);
        let marker: String = db
            .query_row("SELECT v FROM meta WHERE k = '_pending_rescan'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(marker, "1");

        // `sessions` rows are preserved — they're the cache that
        // points back to source files on disk. Only the v4
        // Shared Memory tables (transcript-derived + durable)
        // were cleared.
        let session_count: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(session_count, 1, "sessions table is preserved by forget");
    }

    #[test]
    fn snippet_text_is_redacted_at_rest() {
        // M3 — the schema comment promises a pre-redacted snippet
        // column. Backfill a fixture containing a secret token,
        // then query `exchanges.snippet_text` and assert the
        // literal token is absent.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("codex").join("sessions");
        fs::create_dir_all(&codex_root).unwrap();
        let body = r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"sec","cwd":"/x","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"my key sk-ant-oat01-VeryLongSecretValueHere is the prompt"}]}}
{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}}
"#;
        write_rollout(&codex_root, "2026/05/15", "sec.jsonl", body);
        backfill_codex(&idx, &codex_root).unwrap();

        let db = open_raw(&tmp.path().join("sessions.db"));
        let snippet: String = db
            .query_row("SELECT snippet_text FROM exchanges", [], |r| r.get(0))
            .unwrap();
        assert!(
            !snippet.contains("sk-ant-oat01-VeryLongSecretValueHere"),
            "snippet at rest must not contain raw secret, got: {snippet}"
        );
    }

    #[test]
    fn derive_slug_returns_stem_for_utf8_paths() {
        // Common case: file_stem is UTF-8, slug equals the stem
        // verbatim.
        assert_eq!(derive_slug("/x/rollout-2026.jsonl"), "rollout-2026");
        assert_eq!(derive_slug("/x/foo.bar.jsonl"), "foo.bar");
    }

    #[test]
    fn derive_slug_distinct_hashes_for_distinct_lossy_paths() {
        // L9 — distinct non-UTF-8 paths render to the same U+FFFD
        // sequence via to_string_lossy() and would collide on a
        // simple stem-only slug. Post-fix they're disambiguated
        // via a hex digest of the full path bytes.
        //
        // Simulate the lossy output directly: two paths whose
        // stems contain U+FFFD at different positions but happen
        // to render to similar-looking strings.
        let a = derive_slug("/x/\u{fffd}abc.jsonl");
        let b = derive_slug("/y/\u{fffd}abc.jsonl");
        assert_ne!(a, b, "two lossy stems must produce distinct slugs");
        assert!(a.starts_with("codex-session-"));
        assert!(b.starts_with("codex-session-"));
        // Same path → same slug (stability across reparses).
        assert_eq!(a, derive_slug("/x/\u{fffd}abc.jsonl"));

        // Clean UTF-8 stems with U+FFFD inside also trigger the
        // fallback (no false negative).
        let c = derive_slug("/x/foo\u{fffd}bar.jsonl");
        assert!(c.starts_with("codex-session-"));
    }

    #[test]
    fn truncate_chars_respects_grapheme_boundaries() {
        // L10 — ZWJ family emoji and regional-indicator flag emoji
        // are multi-codepoint graphemes. The pre-fix chars()
        // approach would split them at the cap.
        // 👨‍👩‍👧 is 5 codepoints (man, ZWJ, woman, ZWJ, girl) = 1 grapheme.
        let family = "👨\u{200d}👩\u{200d}👧";
        // With cap=1, we should get either the full family OR
        // (if cap < grapheme count) the ellipsis. The previous
        // chars() impl with cap=1 would have produced "👨…",
        // splitting mid-grapheme.
        let result = truncate_chars(family, 1);
        // Either the whole grapheme survives or only the ellipsis
        // is added. Either way, the man-emoji should NOT appear
        // detached from its family.
        assert!(
            result == family || !result.contains('👨') || result.contains("👨\u{200d}👩\u{200d}👧"),
            "ZWJ family must not be split: got {result:?}"
        );

        // A simple "a b c" string with cap=2 truncates to "ab…".
        assert_eq!(truncate_chars("abc", 2), "ab…");
    }

    #[test]
    fn missing_codex_dir_is_a_clean_zero() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let codex_root = tmp.path().join("nope").join("sessions");
        // Don't create it.
        let stats = backfill_codex(&idx, &codex_root).expect("ok");
        assert_eq!(stats.discovered, 0);
        assert_eq!(stats.indexed, 0);
    }

    #[test]
    fn test_indexer_utility_conns_wait_for_concurrent_writer() {
        // The path-based utility functions open their own raw
        // connections. Without the standard pragmas (busy_timeout)
        // they returned SQLITE_BUSY immediately whenever another
        // connection — e.g. the GUI's long-lived SessionIndex handle
        // — held the writer lock. With `apply_standard_pragmas` they
        // wait out the writer like every other store.
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("sessions.db");
        drop(open_idx(&tmp));

        let writer_path = db_path.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let writer = std::thread::spawn(move || {
            let conn = Connection::open(&writer_path).unwrap();
            crate::db_pragmas::apply_standard_pragmas(&conn).unwrap();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO meta (k, v) VALUES ('_writer_probe', '1')",
                [],
            )
            .unwrap();
            started_tx.send(()).unwrap();
            // Hold the write lock long enough that the utility write
            // below is reliably contending, then release — well
            // within the 5 s busy_timeout.
            std::thread::sleep(std::time::Duration::from_millis(400));
            conn.execute_batch("COMMIT").unwrap();
        });

        started_rx.recv().unwrap();
        // Pre-fix: immediate SQLITE_BUSY. Post-fix: waits for COMMIT.
        mark_pending_rescan(&db_path)
            .expect("mark_pending_rescan must wait out the writer, not fail with SQLITE_BUSY");
        writer.join().unwrap();
        count_shared_memory_rows(&db_path).expect("count must succeed once the writer is idle");
    }
}
