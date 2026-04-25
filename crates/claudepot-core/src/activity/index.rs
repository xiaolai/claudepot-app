//! SQLite read/write surface for `activity_cards`. See design v2 §5.
//!
//! Owns its own `sessions.db` connection (separate from
//! `SessionIndex`). They share the file but not the handle: the
//! activity write path is already on the same WAL, and giving the
//! activity surface its own handle keeps the lock scope narrow and
//! makes the dependency graph linear (activity doesn't pull in the
//! whole `session_index` API).
//!
//! The DDL is idempotent — `CREATE TABLE IF NOT EXISTS` runs on
//! every open, so a DB created by an older binary that didn't know
//! about activity simply gains the table on first activity-aware
//! open. No migration script needed.
//!
//! No `body_json` column. The card carries `byte_offset` into the
//! source JSONL; rendering the body fetches it lazily. See design
//! v2 §1, call #3.

use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;

use super::card::{Card, CardKind, HelpRef, Severity, SourceRef};

/// Errors that escape the activity index.
#[derive(Debug, Error)]
pub enum ActivityIndexError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Handle to the `activity_cards` table. Wraps a single
/// `Mutex<Connection>` so it can cross `await` points in Tauri
/// command handlers, mirroring `SessionIndex`'s thread model.
pub struct ActivityIndex {
    db: Mutex<Connection>,
}

impl ActivityIndex {
    /// Open (or create) the activity index at `path`. Conventional
    /// production path is `~/.claudepot/sessions.db` — the same file
    /// `SessionIndex` uses, since SQLite's WAL mode lets multiple
    /// handles coexist on one DB.
    ///
    /// Sets WAL mode and 0600 perms on Unix. Idempotent on every
    /// detail; safe to call on every process start.
    ///
    /// If the DB file exists but is corrupt (`SQLITE_NOTADB` /
    /// `SQLITE_CORRUPT`), the bad file is moved aside as
    /// `<name>.db.corrupt-<epoch_ms>` and a fresh one is created.
    /// Mirrors `SessionIndex::open()` — the activity index is a pure
    /// derivation from JSONLs, so wipe-and-rebuild is always safe
    /// here. The next `activity reindex` repopulates from disk.
    pub fn open(path: &Path) -> Result<Self, ActivityIndexError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = match Self::init_connection(path) {
            Ok(c) => c,
            Err(ActivityIndexError::Sql(e)) if is_corrupt_error(&e) => {
                quarantine_corrupt_db(path)?;
                Self::init_connection(path)?
            }
            Err(e) => return Err(e),
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, mode.clone())?;
            for sidecar in [path.with_extension("db-wal"), path.with_extension("db-shm")] {
                if sidecar.exists() {
                    std::fs::set_permissions(&sidecar, mode.clone())?;
                }
            }
        }

        Ok(Self { db: Mutex::new(db) })
    }

    /// Open + WAL + busy_timeout + apply_schema + force-touch sidecars.
    /// Extracted so `open` can retry the whole sequence after
    /// quarantine — corruption can surface on PRAGMA or DDL, not just
    /// at `Connection::open`.
    fn init_connection(path: &Path) -> Result<Connection, ActivityIndexError> {
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        apply_schema(&db)?;
        // Force WAL/SHM sidecars to materialize NOW so the chmod
        // loop in open() can narrow their perms. Without this, the
        // sidecars don't exist yet and later writes create them with
        // the process umask (typically 0644) — leaking card titles
        // and stderr previews to other local users. Same trick as
        // `SessionIndex::init_connection`.
        db.execute_batch(
            "BEGIN IMMEDIATE; \
             CREATE TABLE IF NOT EXISTS _activity_touch (k TEXT PRIMARY KEY); \
             INSERT OR IGNORE INTO _activity_touch (k) VALUES ('_'); \
             DELETE FROM _activity_touch WHERE k = '_'; \
             COMMIT;",
        )?;
        Ok(db)
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Insert one card. Idempotent on `(session_path, event_uuid)` —
    /// re-feeding the same JSONL line yields zero new rows. Returns
    /// `Some(rowid)` when a new row was inserted, `None` when the
    /// `(session_path, event_uuid)` already existed (or `event_uuid`
    /// was None and the same `(session_path, byte_offset)` already
    /// existed — a fallback uniqueness constraint for lines without
    /// a uuid).
    pub fn insert(&self, card: &Card) -> Result<Option<i64>, ActivityIndexError> {
        let db = self.db();
        let help_id = card.help.as_ref().map(|h| h.template_id.as_str());
        let help_args = card
            .help
            .as_ref()
            .map(|h| serde_json::to_string(&h.args))
            .transpose()?;
        let source_ref_json = card
            .source_ref
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let session_path = card.session_path.to_string_lossy();
        let cwd = card.cwd.to_string_lossy();
        let ts_ms = card.ts.timestamp_millis();

        // INSERT OR IGNORE so re-feeding the same JSONL is a no-op.
        // The unique index covers both (session_path, event_uuid) and
        // (session_path, byte_offset) — see apply_schema.
        let rows = db.execute(
            "INSERT OR IGNORE INTO activity_cards
                (session_path, event_uuid, byte_offset, kind, severity,
                 ts_ms, title, subtitle, help_id, help_args_json,
                 source_ref_json, cwd, git_branch, plugin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                session_path.as_ref(),
                card.event_uuid,
                card.byte_offset as i64,
                card.kind.label(),
                card.severity.label(),
                ts_ms,
                card.title,
                card.subtitle,
                help_id,
                help_args,
                source_ref_json,
                cwd.as_ref(),
                card.git_branch,
                card.plugin,
            ],
        )?;
        if rows == 0 {
            return Ok(None);
        }
        Ok(Some(db.last_insert_rowid()))
    }

    /// Bulk insert + return per-card outcomes. `Some(rowid)` means
    /// inserted, `None` means deduped against an existing row. Vec
    /// is aligned with the input slice so callers can correlate
    /// classifier-emitted cards with their assigned ids — needed by
    /// the LiveRuntime bus emission path that wants to publish
    /// `CardEmitted { id, ... }` deltas with the canonical id.
    ///
    /// Slightly more expensive than `insert_many` because each row
    /// needs `last_insert_rowid()` lookup. Use `insert_many` when
    /// you only need counts (the backfill path).
    pub fn insert_many_returning_ids(
        &self,
        cards: &[Card],
    ) -> Result<Vec<Option<i64>>, ActivityIndexError> {
        if cards.is_empty() {
            return Ok(Vec::new());
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut out = Vec::with_capacity(cards.len());
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO activity_cards
                    (session_path, event_uuid, byte_offset, kind, severity,
                     ts_ms, title, subtitle, help_id, help_args_json,
                     source_ref_json, cwd, git_branch, plugin)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            for card in cards {
                let help_id = card.help.as_ref().map(|h| h.template_id.as_str());
                let help_args = card
                    .help
                    .as_ref()
                    .map(|h| serde_json::to_string(&h.args))
                    .transpose()?;
                let source_ref_json = card
                    .source_ref
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?;
                let session_path = card.session_path.to_string_lossy();
                let cwd = card.cwd.to_string_lossy();
                let n = stmt.execute(params![
                    session_path.as_ref(),
                    card.event_uuid,
                    card.byte_offset as i64,
                    card.kind.label(),
                    card.severity.label(),
                    card.ts.timestamp_millis(),
                    card.title,
                    card.subtitle,
                    help_id,
                    help_args,
                    source_ref_json,
                    cwd.as_ref(),
                    card.git_branch,
                    card.plugin,
                ])?;
                if n == 0 {
                    out.push(None);
                } else {
                    out.push(Some(tx.last_insert_rowid()));
                }
            }
        }
        tx.commit()?;
        Ok(out)
    }

    /// Bulk insert in a single transaction. Returns `(inserted,
    /// skipped_duplicates)`. The transaction wrapper is a 100×
    /// speedup over per-row autocommit when backfilling thousands
    /// of cards across thousands of sessions — measure once Phase 1
    /// lands; provisionally `IMMEDIATE` so reads don't block the
    /// long write.
    pub fn insert_many(&self, cards: &[Card]) -> Result<(usize, usize), ActivityIndexError> {
        if cards.is_empty() {
            return Ok((0, 0));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut inserted = 0usize;
        let mut skipped = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO activity_cards
                    (session_path, event_uuid, byte_offset, kind, severity,
                     ts_ms, title, subtitle, help_id, help_args_json,
                     source_ref_json, cwd, git_branch, plugin)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            for card in cards {
                let help_id = card.help.as_ref().map(|h| h.template_id.as_str());
                let help_args = card
                    .help
                    .as_ref()
                    .map(|h| serde_json::to_string(&h.args))
                    .transpose()?;
                let source_ref_json = card
                    .source_ref
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?;
                let session_path = card.session_path.to_string_lossy();
                let cwd = card.cwd.to_string_lossy();
                let n = stmt.execute(params![
                    session_path.as_ref(),
                    card.event_uuid,
                    card.byte_offset as i64,
                    card.kind.label(),
                    card.severity.label(),
                    card.ts.timestamp_millis(),
                    card.title,
                    card.subtitle,
                    help_id,
                    help_args,
                    source_ref_json,
                    cwd.as_ref(),
                    card.git_branch,
                    card.plugin,
                ])?;
                if n == 0 {
                    skipped += 1;
                } else {
                    inserted += 1;
                }
            }
        }
        tx.commit()?;
        Ok((inserted, skipped))
    }

    /// Delete every card sourced from `session_path`. Used on rebuild
    /// — the caller drops the rows for one transcript and replays
    /// the JSONL through the classifier. Returns the row count
    /// deleted.
    pub fn delete_for_session(&self, session_path: &Path) -> Result<usize, ActivityIndexError> {
        let db = self.db();
        let n = db.execute(
            "DELETE FROM activity_cards WHERE session_path = ?1",
            params![session_path.to_string_lossy().as_ref()],
        )?;
        Ok(n)
    }

    /// Total row count. Test + diagnostics hook.
    pub fn row_count(&self) -> Result<i64, ActivityIndexError> {
        let db = self.db();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM activity_cards", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Read the `last_seen_card_id` cursor — the highest `id` the
    /// user has acknowledged. Anything above this is "new since you
    /// were away."  Returns `None` when the cursor has never been
    /// set (a fresh install or a freshly cleared index).
    ///
    /// The cursor lives in the `activity_meta` table, alongside any
    /// future per-user index settings. Cheap key/value rows; no
    /// migration cost.
    pub fn last_seen(&self) -> Result<Option<i64>, ActivityIndexError> {
        let db = self.db();
        let v: Option<String> = db
            .query_row(
                "SELECT v FROM activity_meta WHERE k = 'last_seen_card_id'",
                [],
                |r| r.get(0),
            )
            .ok();
        Ok(v.and_then(|s| s.parse::<i64>().ok()))
    }

    /// Set the cursor. Idempotent UPSERT — re-setting to the same
    /// value is a no-op.
    pub fn set_last_seen(&self, id: i64) -> Result<(), ActivityIndexError> {
        let db = self.db();
        db.execute(
            "INSERT INTO activity_meta (k, v) VALUES ('last_seen_card_id', ?1) \
             ON CONFLICT (k) DO UPDATE SET v = excluded.v",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Count rows with `id > cursor` matching the given query
    /// filters. Cheap aggregate query — uses the existing indexes,
    /// no row read. Returns `i64` because SQLite's COUNT is `i64`
    /// natively; values realistically stay well below `i32::MAX` for
    /// any human's history.
    pub fn count_new_since(
        &self,
        cursor: Option<i64>,
        q: &RecentQuery,
    ) -> Result<i64, ActivityIndexError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(cursor) = cursor {
            where_parts.push(format!("id > ?{}", binds.len() + 1));
            binds.push(Box::new(cursor));
        }
        if let Some(since_ms) = q.since_ms {
            where_parts.push(format!("ts_ms >= ?{}", binds.len() + 1));
            binds.push(Box::new(since_ms));
        }
        if !q.kinds.is_empty() {
            let placeholders = q
                .kinds
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", binds.len() + 1 + i))
                .collect::<Vec<_>>()
                .join(",");
            where_parts.push(format!("kind IN ({placeholders})"));
            for k in &q.kinds {
                binds.push(Box::new(k.label().to_string()));
            }
        }
        if let Some(min_sev) = q.min_severity {
            where_parts.push(format!(
                "(CASE severity WHEN 'INFO' THEN 0 WHEN 'NOTICE' THEN 1 WHEN 'WARN' THEN 2 WHEN 'ERROR' THEN 3 ELSE -1 END) >= ?{}",
                binds.len() + 1
            ));
            binds.push(Box::new(severity_rank(min_sev)));
        }
        if let Some(project) = &q.project_path_prefix {
            let n = binds.len() + 1;
            where_parts.push(format!("substr(cwd, 1, length(?{n})) = ?{n}"));
            binds.push(Box::new(project.to_string_lossy().into_owned()));
        }
        if let Some(plugin) = &q.plugin {
            // Same predicate as recent() — keep the two paths in
            // lockstep so the "N new" badge agrees with the list.
            let n = binds.len() + 1;
            let m = binds.len() + 2;
            where_parts.push(format!("(plugin = ?{n} OR substr(plugin, 1, length(?{m})) = ?{m})"));
            binds.push(Box::new(plugin.clone()));
            binds.push(Box::new(format!("{plugin}@")));
        }

        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_parts.join(" AND "))
        };
        let sql = format!("SELECT COUNT(*) FROM activity_cards{where_clause}");

        let db = self.db();
        let mut stmt = db.prepare(&sql)?;
        let bind_refs: Vec<&dyn rusqlite::ToSql> =
            binds.iter().map(|b| b.as_ref()).collect();
        let n: i64 = stmt.query_row(rusqlite::params_from_iter(bind_refs), |r| r.get(0))?;
        Ok(n)
    }

    /// Return every distinct `session_path` in the index that is
    /// NOT in `live`. Used by `backfill::run` to identify orphaned
    /// rows whose source JSONL was deleted/moved/renamed since the
    /// last reindex. Caller drops these via `delete_for_session`.
    pub fn session_paths_not_in(
        &self,
        live: &std::collections::HashSet<PathBuf>,
    ) -> Result<Vec<PathBuf>, ActivityIndexError> {
        let db = self.db();
        let mut stmt = db.prepare("SELECT DISTINCT session_path FROM activity_cards")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            let s = r?;
            let p = PathBuf::from(s);
            if !live.contains(&p) {
                out.push(p);
            }
        }
        Ok(out)
    }

    /// Recent cards, newest first. The CLI's primary read path.
    pub fn recent(&self, q: &RecentQuery) -> Result<Vec<Card>, ActivityIndexError> {
        // Build the WHERE clause incrementally so optional filters
        // don't bind unused params. Each filter is a `(sql_fragment,
        // bind)` pair — we collect, join with " AND ", and bind in
        // order. Less elegant than a query builder, but rusqlite
        // doesn't ship one and the surface is tiny.
        let mut where_parts: Vec<String> = Vec::new();
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(since_ms) = q.since_ms {
            where_parts.push(format!("ts_ms >= ?{}", binds.len() + 1));
            binds.push(Box::new(since_ms));
        }
        if !q.kinds.is_empty() {
            let placeholders = q
                .kinds
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", binds.len() + 1 + i))
                .collect::<Vec<_>>()
                .join(",");
            where_parts.push(format!("kind IN ({placeholders})"));
            for k in &q.kinds {
                binds.push(Box::new(k.label().to_string()));
            }
        }
        if let Some(min_sev) = q.min_severity {
            // Severity is stored as text label. Translate to ordering
            // via a CASE since SQLite has no enum ordering. The
            // numeric ordering matches `Severity`'s `Ord` impl.
            where_parts.push(format!(
                "(CASE severity WHEN 'INFO' THEN 0 WHEN 'NOTICE' THEN 1 WHEN 'WARN' THEN 2 WHEN 'ERROR' THEN 3 ELSE -1 END) >= ?{}",
                binds.len() + 1
            ));
            binds.push(Box::new(severity_rank(min_sev)));
        }
        if let Some(project) = &q.project_path_prefix {
            // Plain prefix match. `LIKE` would treat `%` and `_` in
            // the user's path as wildcards, and SQLite's default
            // ASCII case-folding could surface cards from the wrong
            // directory. `substr(cwd, 1, length(?))` is byte-exact.
            let n = binds.len() + 1;
            where_parts.push(format!("substr(cwd, 1, length(?{n})) = ?{n}"));
            binds.push(Box::new(project.to_string_lossy().into_owned()));
        }
        if let Some(plugin) = &q.plugin {
            // Match either bare plugin name or `<name>@<owner>` form
            // — many cards have a name without owner attribution.
            let n = binds.len() + 1;
            let m = binds.len() + 2;
            where_parts.push(format!("(plugin = ?{n} OR substr(plugin, 1, length(?{m})) = ?{m})"));
            binds.push(Box::new(plugin.clone()));
            // Match `<name>@` so `name@anyowner` also matches when
            // the user filters by `name`.
            binds.push(Box::new(format!("{plugin}@")));
        }

        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_parts.join(" AND "))
        };
        let limit = q.limit.unwrap_or(200).min(10_000);
        let sql = format!(
            "SELECT id, session_path, event_uuid, byte_offset, kind, severity,
                    ts_ms, title, subtitle, help_id, help_args_json,
                    source_ref_json, cwd, git_branch, plugin
               FROM activity_cards
               {where_clause}
               ORDER BY ts_ms DESC
               LIMIT ?{n}",
            n = binds.len() + 1
        );
        binds.push(Box::new(limit as i64));

        let db = self.db();
        let mut stmt = db.prepare(&sql)?;
        let bind_refs: Vec<&dyn rusqlite::ToSql> =
            binds.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(bind_refs), row_to_card)?;
        let mut out = Vec::new();
        for r in rows {
            match r {
                Ok(card) => out.push(card),
                // Per-row decode error: log + skip, don't abort the
                // query. Lets a corrupt or forward-version row coexist
                // with healthy ones; users see the rest of their
                // activity. The next `claudepot activity reindex`
                // overwrites bad rows from the canonical JSONL.
                Err(e) if matches!(e, rusqlite::Error::FromSqlConversionFailure(..)) => {
                    tracing::warn!(
                        target = "activity::index",
                        error = %e,
                        "skipping undecodable activity_cards row"
                    );
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(out)
    }
}

/// Read-side query parameters. All filters are AND'd. `None` means
/// no constraint on that dimension.
#[derive(Debug, Clone, Default)]
pub struct RecentQuery {
    /// Only cards with `ts_ms >= since_ms`. Use `(now - duration)
    /// .timestamp_millis()` to emulate "last N hours."
    pub since_ms: Option<i64>,
    /// Empty = all kinds. Use `vec![CardKind::HookFailure]` for
    /// "show me only hook failures."
    pub kinds: Vec<CardKind>,
    /// Cards with `severity >= min_severity`. `None` = all.
    pub min_severity: Option<Severity>,
    /// Show only cards whose `cwd` starts with this path. Useful for
    /// per-project filtering. `None` = all projects.
    pub project_path_prefix: Option<PathBuf>,
    /// Show only cards attributed to this plugin (`<name>` or
    /// `<name>@<owner>`). `None` = all plugins, including unattributed
    /// (`plugin IS NULL`) cards.
    pub plugin: Option<String>,
    /// Default 200, max 10_000.
    pub limit: Option<usize>,
}

fn severity_rank(s: Severity) -> i64 {
    match s {
        Severity::Info => 0,
        Severity::Notice => 1,
        Severity::Warn => 2,
        Severity::Error => 3,
    }
}

/// Parse a severity label or return `None`. No silent fallback —
/// recent() turns `None` into a row-level decode error so corruption
/// or forward-version mismatches surface as a real error rather than
/// fake `Info` cards.
fn parse_severity_label(label: &str) -> Option<Severity> {
    Some(match label {
        "ERROR" => Severity::Error,
        "WARN" => Severity::Warn,
        "NOTICE" => Severity::Notice,
        "INFO" => Severity::Info,
        _ => return None,
    })
}

fn parse_kind_label(label: &str) -> Option<CardKind> {
    Some(match label {
        "hook" => CardKind::HookFailure,
        "hook-slow" => CardKind::HookSlow,
        "hook-info" => CardKind::HookGuidance,
        "agent" => CardKind::AgentReturn,
        "agent-stranded" => CardKind::AgentStranded,
        "tool-error" => CardKind::ToolError,
        "command" => CardKind::CommandFailure,
        "milestone" => CardKind::SessionMilestone,
        _ => return None,
    })
}

/// Decode a row into a `Card`, or return a sentinel rusqlite error
/// that callers in `recent()` translate into a logged + skipped row.
/// Decoding is strict: unknown enum labels, malformed JSON in the
/// help/source_ref columns, or out-of-range timestamps all reject
/// the row instead of fabricating defaults. Forward-version
/// compatibility is provided by skip-with-log, not silent coercion.
fn row_to_card(r: &rusqlite::Row) -> rusqlite::Result<Card> {
    let id: i64 = r.get(0)?;
    let session_path: String = r.get(1)?;
    let event_uuid: Option<String> = r.get(2)?;
    let byte_offset: i64 = r.get(3)?;
    let kind_label: String = r.get(4)?;
    let severity_label: String = r.get(5)?;
    let ts_ms: i64 = r.get(6)?;
    let title: String = r.get(7)?;
    let subtitle: Option<String> = r.get(8)?;
    let help_id: Option<String> = r.get(9)?;
    let help_args_json: Option<String> = r.get(10)?;
    let source_ref_json: Option<String> = r.get(11)?;
    let cwd: String = r.get(12)?;
    let git_branch: Option<String> = r.get(13)?;
    let plugin: Option<String> = r.get(14)?;

    let kind = parse_kind_label(&kind_label).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(DecodeError::new(format!("unknown kind {kind_label:?}"))),
        )
    })?;
    let severity = parse_severity_label(&severity_label).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(DecodeError::new(format!(
                "unknown severity {severity_label:?}"
            ))),
        )
    })?;

    let help = match (help_id, help_args_json) {
        (Some(id), args_json) => {
            let args = match args_json.as_deref() {
                None => Default::default(),
                Some(s) => serde_json::from_str(s).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(DecodeError::new(format!("help_args_json invalid: {e}"))),
                    )
                })?,
            };
            Some(HelpRef {
                template_id: id,
                args,
            })
        }
        (None, _) => None,
    };
    let source_ref = match source_ref_json.as_deref() {
        None => None,
        Some(s) => Some(serde_json::from_str::<SourceRef>(s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Text,
                Box::new(DecodeError::new(format!("source_ref_json invalid: {e}"))),
            )
        })?),
    };

    let ts = chrono::DateTime::from_timestamp_millis(ts_ms).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Integer,
            Box::new(DecodeError::new(format!("ts_ms out of range: {ts_ms}"))),
        )
    })?;

    Ok(Card {
        id: Some(id),
        session_path: PathBuf::from(session_path),
        event_uuid,
        byte_offset: byte_offset as u64,
        kind,
        ts,
        severity,
        title,
        subtitle,
        help,
        source_ref,
        cwd: PathBuf::from(cwd),
        git_branch,
        plugin,
    })
}

/// Boxed payload for `rusqlite::Error::FromSqlConversionFailure`.
/// rusqlite expects `Box<dyn std::error::Error + Send + Sync>` so
/// the decode-error reason can travel with the variant.
#[derive(Debug)]
struct DecodeError(String);
impl DecodeError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DecodeError {}

/// Detect the two rusqlite error codes that mean "the file isn't a
/// usable SQLite database." Anything else (I/O, locking, etc.)
/// propagates — we don't quarantine a DB just because the disk is
/// full.
fn is_corrupt_error(err: &rusqlite::Error) -> bool {
    if let rusqlite::Error::SqliteFailure(info, _) = err {
        matches!(
            info.code,
            rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
        )
    } else {
        false
    }
}

/// Move a corrupt DB (plus any WAL/SHM sidecars) aside so `open()`
/// can create a fresh file. The activity index is a pure derivation,
/// so "recovered" just means "rebuild via `claudepot activity
/// reindex`". The quarantined file is preserved for inspection rather
/// than deleted.
fn quarantine_corrupt_db(path: &Path) -> Result<(), ActivityIndexError> {
    let stamp = chrono::Utc::now().timestamp_millis();
    let corrupt_path = path.with_extension(format!("db.corrupt-{stamp}"));
    tracing::warn!(
        from = %path.display(),
        to = %corrupt_path.display(),
        "activity index: quarantining corrupt DB and rebuilding"
    );
    std::fs::rename(path, &corrupt_path)?;
    for sidecar_ext in ["db-wal", "db-shm"] {
        let side = path.with_extension(sidecar_ext);
        if side.exists() {
            let _ = std::fs::remove_file(side);
        }
    }
    Ok(())
}

/// Add `plugin` column to `activity_cards` on DBs that pre-date its
/// introduction. Idempotent: checks `PRAGMA table_info` first, only
/// runs `ALTER TABLE` when missing. Same shape as `account.rs`'s
/// additive-ALTER pattern.
fn ensure_plugin_column(db: &Connection) -> Result<(), ActivityIndexError> {
    let mut stmt = db.prepare("PRAGMA table_info(activity_cards)")?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == "plugin") {
        db.execute("ALTER TABLE activity_cards ADD COLUMN plugin TEXT", [])?;
    }
    Ok(())
}

fn apply_schema(db: &Connection) -> Result<(), ActivityIndexError> {
    db.execute_batch(
        r#"
        -- Per-DB key/value scratch space for the activity index.
        -- Currently holds `last_seen_card_id` (the cursor for the
        -- "N new since you were away" badge); future additions
        -- (per-template mute, per-project last-acknowledged) live
        -- here too.
        CREATE TABLE IF NOT EXISTS activity_meta (
            k TEXT PRIMARY KEY,
            v TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS activity_cards (
            id              INTEGER PRIMARY KEY,
            session_path    TEXT NOT NULL,
            event_uuid      TEXT,
            byte_offset     INTEGER NOT NULL,
            kind            TEXT NOT NULL,
            severity        TEXT NOT NULL,
            ts_ms           INTEGER NOT NULL,
            title           TEXT NOT NULL,
            subtitle        TEXT,
            help_id         TEXT,
            help_args_json  TEXT,
            source_ref_json TEXT,
            cwd             TEXT NOT NULL,
            git_branch      TEXT,
            plugin          TEXT
        );

        -- Idempotency: a re-fed JSONL line with the same uuid is a
        -- no-op insert. Lines without a uuid (rare; pre-2.1 envelope
        -- shapes) fall back to (session_path, byte_offset) which is
        -- guaranteed unique within one file.
        CREATE UNIQUE INDEX IF NOT EXISTS uniq_activity_cards_uuid
            ON activity_cards(session_path, event_uuid)
            WHERE event_uuid IS NOT NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS uniq_activity_cards_offset
            ON activity_cards(session_path, byte_offset)
            WHERE event_uuid IS NULL;

        CREATE INDEX IF NOT EXISTS idx_activity_cards_ts
            ON activity_cards(ts_ms DESC);
        CREATE INDEX IF NOT EXISTS idx_activity_cards_kind_sev_ts
            ON activity_cards(kind, severity, ts_ms DESC);
        CREATE INDEX IF NOT EXISTS idx_activity_cards_cwd_ts
            ON activity_cards(cwd, ts_ms DESC);
        "#,
    )?;
    ensure_plugin_column(db)?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_activity_cards_plugin_ts \
         ON activity_cards(plugin, ts_ms DESC) WHERE plugin IS NOT NULL;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::card::CardKind;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn sample_card(uuid: &str, byte: u64, title: &str) -> Card {
        let mut args = BTreeMap::new();
        args.insert("plugin".to_string(), "mermaid-preview@xiaolai".to_string());
        Card {
            id: None,
            session_path: PathBuf::from("/tmp/x.jsonl"),
            event_uuid: Some(uuid.into()),
            byte_offset: byte,
            kind: CardKind::HookFailure,
            ts: Utc::now(),
            severity: Severity::Warn,
            title: title.into(),
            subtitle: Some("bash failed".into()),
            help: Some(HelpRef {
                template_id: "hook.plugin_missing".into(),
                args,
            }),
            source_ref: None,
            cwd: PathBuf::from("/Users/x/proj"),
            git_branch: Some("main".into()),
            plugin: None,
        }
    }

    #[test]
    fn open_creates_table_and_starts_empty() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        assert_eq!(idx.row_count().unwrap(), 0);
    }

    /// Regression for Codex audit MEDIUM #6 / LOW #12: `open()` must
    /// quarantine a corrupt DB and rebuild from scratch, mirroring
    /// `SessionIndex`. The activity index is a derivation, so wipe-
    /// and-rebuild is always safe — we don't want a corrupted file
    /// to brick `claudepot activity` commands.
    #[test]
    fn open_quarantines_corrupt_db_and_rebuilds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.db");
        // Plant a non-SQLite file at the target path. SQLite will
        // refuse it with NotADatabase; the open() retry path must
        // rename it aside and create a fresh DB.
        std::fs::write(&path, b"not a sqlite database, just bytes").unwrap();
        let idx = ActivityIndex::open(&path).unwrap();
        assert_eq!(idx.row_count().unwrap(), 0, "fresh DB must start empty");

        let sibs: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            sibs.iter().any(|n| n.contains(".db.corrupt-")),
            "quarantined file must remain on disk: {sibs:?}"
        );
    }

    /// Regression for Codex audit LOW #12 (sidecar perms): the
    /// touch-write trick in `init_connection` must materialize the
    /// WAL/SHM sidecars on first open so the chmod loop can narrow
    /// their perms. Without it, the first write to the DB would
    /// create them with the process umask (typically 0644).
    #[cfg(unix)]
    #[test]
    fn open_narrows_wal_shm_sidecar_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.db");
        let _idx = ActivityIndex::open(&path).unwrap();
        for ext in ["db-wal", "db-shm"] {
            let side = path.with_extension(ext);
            assert!(side.exists(), "{ext} sidecar must exist after open");
            let mode = std::fs::metadata(&side).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{ext} perms should be 0o600, got {mode:o}");
        }
    }

    /// Regression for Codex audit MEDIUM #7: row decode must reject
    /// rows with unknown enum labels rather than fabricate `Info`/
    /// `ToolError` defaults. The bad row is logged and skipped; the
    /// healthy rows still come back.
    #[test]
    fn recent_skips_rows_with_unknown_kind_label() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        idx.insert(&sample_card("u1", 0, "good")).unwrap();

        // Inject a row with a bogus kind label — bypass the public
        // insert path so we can simulate a forward-version row.
        {
            let db = idx.db();
            db.execute(
                "INSERT INTO activity_cards (session_path, event_uuid, byte_offset, kind, severity, ts_ms, title, cwd) \
                 VALUES ('/tmp/bad.jsonl', 'u-bad', 0, 'kind-from-future-version', 'WARN', 0, 'bad', '/x')",
                [],
            )
            .unwrap();
        }

        let cards = idx.recent(&RecentQuery::default()).unwrap();
        assert_eq!(cards.len(), 1, "bad row skipped, good row returned");
        assert_eq!(cards[0].title, "good");
    }

    /// Phase 2: `last_seen_card_id` cursor round-trips. Fresh DB →
    /// `None`. After `set_last_seen(N)` → `Some(N)`. Re-setting is
    /// an UPSERT, not an error.
    #[test]
    fn last_seen_cursor_round_trips() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        assert_eq!(idx.last_seen().unwrap(), None);

        idx.set_last_seen(42).unwrap();
        assert_eq!(idx.last_seen().unwrap(), Some(42));

        // UPSERT — re-setting overwrites without error.
        idx.set_last_seen(43).unwrap();
        assert_eq!(idx.last_seen().unwrap(), Some(43));
    }

    /// Phase 2: `count_new_since` returns rows above the cursor that
    /// also match the filter set. Drives the "N new since you were
    /// away" badge.
    #[test]
    fn count_new_since_respects_cursor_and_filters() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let mut warn = sample_card("u1", 0, "warn-card");
        warn.severity = Severity::Warn;
        let mut info = sample_card("u2", 1, "info-card");
        info.kind = CardKind::ToolError;
        info.severity = Severity::Info;
        let id_warn = idx.insert(&warn).unwrap().unwrap();
        let id_info = idx.insert(&info).unwrap().unwrap();
        assert!(id_warn < id_info, "rowid is monotonic");

        // No cursor: every row counts.
        let total = idx
            .count_new_since(None, &RecentQuery::default())
            .unwrap();
        assert_eq!(total, 2);

        // Cursor at id_warn: only the info row is "new."
        let after_warn = idx
            .count_new_since(Some(id_warn), &RecentQuery::default())
            .unwrap();
        assert_eq!(after_warn, 1);

        // Cursor + severity filter: cursor excludes warn, filter
        // would exclude info, so net zero.
        let new_warns_after = idx
            .count_new_since(
                Some(id_warn),
                &RecentQuery {
                    min_severity: Some(Severity::Warn),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(new_warns_after, 0);

        // Plugin filter parity with recent(): inserting a card with
        // a plugin attribution and counting "new since 0" with that
        // filter must agree with recent() (1, not 0).
        let mut tagged = sample_card("u3", 2, "tagged");
        tagged.plugin = Some("mermaid-preview@xiaolai".to_string());
        let id_tagged = idx.insert(&tagged).unwrap().unwrap();
        let only_plugin = RecentQuery {
            plugin: Some("mermaid-preview".to_string()),
            ..Default::default()
        };
        assert_eq!(idx.recent(&only_plugin).unwrap().len(), 1);
        assert_eq!(
            idx.count_new_since(Some(id_tagged - 1), &only_plugin).unwrap(),
            1,
            "count_new_since must honor plugin filter — parity with recent()"
        );
    }

    /// Regression for Codex audit LOW #3: project-prefix filtering
    /// must NOT treat `%` and `_` in the user's path as SQL wildcards.
    /// A path containing `%` should match exactly, not as a glob.
    #[test]
    fn project_filter_does_not_treat_percent_as_wildcard() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let mut a = sample_card("u-a", 0, "a");
        a.cwd = PathBuf::from("/Users/x/proj");
        let mut b = sample_card("u-b", 0, "b");
        b.session_path = PathBuf::from("/tmp/y.jsonl");
        b.cwd = PathBuf::from("/Users/x/other");
        idx.insert(&a).unwrap();
        idx.insert(&b).unwrap();

        // Filtering by "/Users/x/p%" used to match BOTH rows under
        // a LIKE-based filter (because % is a wildcard there); the
        // substr-based filter must match neither.
        let q = RecentQuery {
            project_path_prefix: Some(PathBuf::from("/Users/x/p%")),
            ..Default::default()
        };
        let cards = idx.recent(&q).unwrap();
        assert!(
            cards.is_empty(),
            "literal % in prefix must not glob: got {:?}",
            cards.iter().map(|c| c.title.clone()).collect::<Vec<_>>()
        );

        // Sanity: a real prefix still filters correctly.
        let q2 = RecentQuery {
            project_path_prefix: Some(PathBuf::from("/Users/x/proj")),
            ..Default::default()
        };
        let only_a = idx.recent(&q2).unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].title, "a");
    }

    #[test]
    fn insert_and_recent_round_trip() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let id = idx.insert(&sample_card("u1", 0, "Hook failed: PostToolUse:Edit")).unwrap();
        assert!(id.is_some());
        assert_eq!(idx.row_count().unwrap(), 1);
        let cards = idx.recent(&RecentQuery::default()).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].title, "Hook failed: PostToolUse:Edit");
        assert_eq!(cards[0].help.as_ref().unwrap().template_id, "hook.plugin_missing");
    }

    /// The idempotency invariant — re-inserting a card with the same
    /// (session_path, event_uuid) is a no-op. Re-running backfill on
    /// the same JSONL must NOT duplicate rows.
    #[test]
    fn re_insert_same_uuid_is_a_no_op() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let c = sample_card("u1", 0, "first");
        let _ = idx.insert(&c).unwrap();
        let second = idx.insert(&c).unwrap();
        assert!(second.is_none(), "duplicate insert returns None");
        assert_eq!(idx.row_count().unwrap(), 1, "no duplicate row");
    }

    /// Lines without a uuid fall back to (session_path, byte_offset)
    /// uniqueness — same offset twice is a no-op, different offset
    /// is a new row.
    #[test]
    fn null_uuid_dedupes_on_offset() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let mut c = sample_card("ignored", 100, "no-uuid card");
        c.event_uuid = None;
        let first = idx.insert(&c).unwrap();
        let second = idx.insert(&c).unwrap();
        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(idx.row_count().unwrap(), 1);

        let mut c2 = c.clone();
        c2.byte_offset = 200;
        let third = idx.insert(&c2).unwrap();
        assert!(third.is_some(), "different offset → new row");
        assert_eq!(idx.row_count().unwrap(), 2);
    }

    #[test]
    fn delete_for_session_clears_only_that_session() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let mut c1 = sample_card("u1", 0, "session1");
        c1.session_path = PathBuf::from("/tmp/s1.jsonl");
        let mut c2 = sample_card("u2", 0, "session2");
        c2.session_path = PathBuf::from("/tmp/s2.jsonl");
        idx.insert(&c1).unwrap();
        idx.insert(&c2).unwrap();
        let n = idx.delete_for_session(Path::new("/tmp/s1.jsonl")).unwrap();
        assert_eq!(n, 1);
        assert_eq!(idx.row_count().unwrap(), 1);
        let remaining = idx.recent(&RecentQuery::default()).unwrap();
        assert_eq!(remaining[0].title, "session2");
    }

    #[test]
    fn recent_filters_by_kind_and_severity() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let mut warn_card = sample_card("u1", 0, "warn");
        warn_card.severity = Severity::Warn;
        let mut error_card = sample_card("u2", 1, "error");
        error_card.kind = CardKind::ToolError;
        error_card.severity = Severity::Error;
        idx.insert(&warn_card).unwrap();
        idx.insert(&error_card).unwrap();

        let only_errors = idx
            .recent(&RecentQuery {
                min_severity: Some(Severity::Error),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(only_errors.len(), 1);
        assert_eq!(only_errors[0].title, "error");

        let only_hooks = idx
            .recent(&RecentQuery {
                kinds: vec![CardKind::HookFailure],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(only_hooks.len(), 1);
        assert_eq!(only_hooks[0].title, "warn");
    }

    /// Bulk insert keeps the same idempotency contract — re-running
    /// the backfill yields zero new rows on the second pass. This is
    /// the rebuild safety net.
    #[test]
    fn insert_many_is_idempotent() {
        let dir = tempdir().unwrap();
        let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let cards = vec![
            sample_card("u1", 0, "a"),
            sample_card("u2", 1, "b"),
            sample_card("u3", 2, "c"),
        ];
        let (ins, skipped) = idx.insert_many(&cards).unwrap();
        assert_eq!((ins, skipped), (3, 0));
        let (ins2, skipped2) = idx.insert_many(&cards).unwrap();
        assert_eq!((ins2, skipped2), (0, 3));
        assert_eq!(idx.row_count().unwrap(), 3);
    }
}
