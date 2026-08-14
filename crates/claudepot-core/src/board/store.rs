//! SQLite boundary for boards — `~/.claudepot/boards.db`.
//!
//! # Multi-process rules
//!
//! This store is opened **directly** by every writer: the GUI, the CLI,
//! the MCP server subprocess, and any script. There is no coordinating
//! channel (see [`super`]), so the concurrency rules are load-bearing
//! rather than advisory:
//!
//! - WAL plus a 5s busy timeout, via
//!   [`crate::db_pragmas::apply_standard_pragmas`].
//! - Every write runs in a **short** `BEGIN IMMEDIATE` transaction.
//!   Immediate rather than deferred so writer contention is resolved at
//!   `BEGIN` (where `busy_timeout` applies) instead of at first write
//!   (where a deferred transaction would have to roll back).
//! - **Readers must never hold a transaction across a render or a
//!   subscription lifetime.** Every read method here opens and closes
//!   within the call for exactly that reason. A long-held read
//!   transaction blocks WAL checkpointing and grows `boards.db-wal`
//!   without bound — the 2026-05-24 `sessions.db-wal` incident is what
//!   this rule exists to avoid repeating.
//!
//! # Migrations preserve rows
//!
//! `boards.db` is user data, not a cache. A migration that drops and
//! rebuilds is a data-loss bug, not a fast path — a board's contents
//! exist nowhere else once the writing session ends.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use super::error::BoardError;
use super::ingest::{self, IngestCaps, PushOutcome, PushRequest};
use super::series::{check_name, Provenance, Row, SeriesDef, Value, WriterId, WriterKind};
use super::spec::BoardSpec;
use crate::db_pragmas::apply_standard_pragmas;

/// Current on-disk schema version, tracked in `PRAGMA user_version`.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS boards (
    board_id      TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    spec_json     TEXT NOT NULL,
    spec_revision INTEGER NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    -- Set only on an imported board: the id it carried in its
    -- envelope. Import never reuses an id (see board::export).
    source_board_id TEXT
);

CREATE TABLE IF NOT EXISTS board_series (
    board_id     TEXT NOT NULL,
    name         TEXT NOT NULL,
    columns_json TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (board_id, name)
);

CREATE TABLE IF NOT EXISTS board_rows (
    row_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    board_id     TEXT NOT NULL,
    series       TEXT NOT NULL,
    writer_key   TEXT NOT NULL,
    writer_kind  TEXT NOT NULL,
    writer_label TEXT NOT NULL,
    writer_seq   INTEGER NOT NULL,
    values_json  TEXT NOT NULL,
    pushed_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS board_rows_lookup
    ON board_rows (board_id, series, row_id);

CREATE INDEX IF NOT EXISTS board_rows_writer_seq
    ON board_rows (board_id, series, writer_key, writer_seq);

CREATE TABLE IF NOT EXISTS board_pushes (
    board_id   TEXT NOT NULL,
    idem_key   TEXT NOT NULL,
    rows_added INTEGER NOT NULL,
    applied_at TEXT NOT NULL,
    PRIMARY KEY (board_id, idem_key)
);
"#;

/// One series inside a [`BoardDetailSnapshot`].
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesSnapshot {
    pub def: SeriesDef,
    pub row_count: usize,
    /// The tail, oldest-first within the window.
    pub rows: Vec<Row>,
}

/// Everything a detail view needs, consistent as of one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardDetailSnapshot {
    pub board: Board,
    pub source_board_id: Option<String>,
    pub series: Vec<SeriesSnapshot>,
}

/// A board plus the aggregate facts a list view needs, gathered
/// without a per-board query storm. See [`BoardStore::list_summaries`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSummary {
    pub board: Board,
    pub series: Vec<String>,
    pub total_rows: usize,
    /// Latest writer's self-declared label. A claim, never verified.
    pub reported_writer: Option<String>,
}

/// A board's identity and current spec revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub board_id: String,
    /// Mutable display label. **Not** an identity and not unique —
    /// renaming a board must not break a scheduled agent writing to it.
    pub name: String,
    pub spec: BoardSpec,
    pub spec_revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct BoardStore {
    db: Mutex<Connection>,
    caps: IngestCaps,
}

impl BoardStore {
    /// Open (creating if absent) with standard pragmas, user-only
    /// permissions, and migrations applied.
    pub fn open(path: &Path) -> Result<Self, BoardError> {
        Self::open_with_caps(path, IngestCaps::default())
    }

    pub fn open_with_caps(path: &Path, caps: IngestCaps) -> Result<Self, BoardError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Pre-create with user-only perms BEFORE rusqlite opens it at
        // umask defaults; the WAL/SHM sidecars inherit the main file's
        // mode. Same ordering as `env_vault::store` and
        // `session_index::SessionIndex::open`.
        crate::secure_perms::precreate_user_only(path);

        let db = Connection::open(path)?;
        apply_standard_pragmas(&db)?;
        migrate(&db)?;

        crate::secure_perms::harden_user_only(path)?;
        crate::secure_perms::harden_user_only(&path.with_extension("db-wal"))?;
        crate::secure_perms::harden_user_only(&path.with_extension("db-shm"))?;

        Ok(Self {
            db: Mutex::new(db),
            caps,
        })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        crate::sync::recover_lock(&self.db, "boards")
    }

    pub fn caps(&self) -> &IngestCaps {
        &self.caps
    }

    /// Create a board with its series definitions and an initial spec.
    ///
    /// Series are fixed at creation because their column types are the
    /// contract every later push is checked against.
    pub fn create_board(
        &self,
        name: &str,
        spec: &BoardSpec,
        series: &[SeriesDef],
    ) -> Result<Board, BoardError> {
        self.create_board_inner(name, spec, series, None)
    }

    /// Create a board that came from an export envelope, recording the
    /// id it carried. The new board always gets a fresh `board_id`.
    pub fn create_board_imported(
        &self,
        name: &str,
        spec: &BoardSpec,
        series: &[SeriesDef],
        source_board_id: &str,
    ) -> Result<Board, BoardError> {
        self.create_board_inner(name, spec, series, Some(source_board_id))
    }

    fn create_board_inner(
        &self,
        name: &str,
        spec: &BoardSpec,
        series: &[SeriesDef],
        source_board_id: Option<&str>,
    ) -> Result<Board, BoardError> {
        check_name(name)?;
        if series.len() > self.caps.max_series_per_board {
            return Err(BoardError::CapExceeded {
                what: "series per board",
                limit: self.caps.max_series_per_board,
                actual: series.len(),
            });
        }
        for def in series {
            def.validate(self.caps.max_columns_per_series)?;
        }
        let mut names: Vec<&str> = series.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        if names.len() != before {
            return Err(BoardError::InvalidSpec(
                "duplicate series names on one board".to_string(),
            ));
        }
        spec.validate(series)?;

        let board_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let spec_json = serde_json::to_string(spec)?;

        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO boards \
             (board_id, name, spec_json, spec_revision, created_at, updated_at, source_board_id) \
             VALUES (?1, ?2, ?3, 1, ?4, ?4, ?5)",
            params![board_id, name, spec_json, now.to_rfc3339(), source_board_id],
        )?;
        for def in series {
            tx.execute(
                "INSERT INTO board_series (board_id, name, columns_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    board_id,
                    def.name,
                    serde_json::to_string(&def.columns)?,
                    now.to_rfc3339()
                ],
            )?;
        }
        tx.commit()?;

        Ok(Board {
            board_id,
            name: name.to_string(),
            spec: spec.clone(),
            spec_revision: 1,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn get_board(&self, board_id: &str) -> Result<Board, BoardError> {
        let db = self.db();
        row_to_board(&db, board_id)
    }

    /// Every board, most recently updated first.
    pub fn list_boards(&self) -> Result<Vec<Board>, BoardError> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT board_id, name, spec_json, spec_revision, created_at, updated_at \
             FROM boards ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(build_board(r?)?);
        }
        Ok(out)
    }

    /// Revision-checked spec replacement.
    ///
    /// A stale `base_rev` is a [`BoardError::RevisionConflict`], never a
    /// silent overwrite (plan F8). Two agents patching one board must
    /// both observe the other's change.
    pub fn update_spec(
        &self,
        board_id: &str,
        spec: &BoardSpec,
        base_rev: i64,
    ) -> Result<Board, BoardError> {
        let series = self.series_defs(board_id)?;
        spec.validate(&series)?;
        let spec_json = serde_json::to_string(spec)?;
        let now = Utc::now();

        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Read the revision *inside* the transaction. Deciding from a
        // snapshot taken outside it is a race by construction.
        let current: i64 = tx
            .query_row(
                "SELECT spec_revision FROM boards WHERE board_id = ?1",
                params![board_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                BoardError::BoardNotFound(crate::board::error::redact_identifier(board_id))
            })?;
        if current != base_rev {
            return Err(BoardError::RevisionConflict {
                board: crate::board::error::redact_identifier(board_id),
                base: base_rev,
                current,
            });
        }
        tx.execute(
            "UPDATE boards SET spec_json = ?1, spec_revision = ?2, updated_at = ?3 \
             WHERE board_id = ?4",
            params![spec_json, current + 1, now.to_rfc3339(), board_id],
        )?;
        tx.commit()?;

        let mut board = row_to_board(&db, board_id)?;
        board.spec = spec.clone();
        Ok(board)
    }

    /// Delete a board and everything under it. Explicit only — there is
    /// no automatic pruning anywhere in this module (plan §12).
    pub fn delete_board(&self, board_id: &str) -> Result<(), BoardError> {
        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existed: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM boards WHERE board_id = ?1",
                params![board_id],
                |r| r.get(0),
            )
            .optional()?;
        if existed.is_none() {
            return Err(BoardError::BoardNotFound(
                crate::board::error::redact_identifier(board_id),
            ));
        }
        tx.execute(
            "DELETE FROM board_rows WHERE board_id = ?1",
            params![board_id],
        )?;
        tx.execute(
            "DELETE FROM board_series WHERE board_id = ?1",
            params![board_id],
        )?;
        tx.execute(
            "DELETE FROM board_pushes WHERE board_id = ?1",
            params![board_id],
        )?;
        tx.execute("DELETE FROM boards WHERE board_id = ?1", params![board_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn series_defs(&self, board_id: &str) -> Result<Vec<SeriesDef>, BoardError> {
        let db = self.db();
        let exists: Option<i64> = db
            .query_row(
                "SELECT 1 FROM boards WHERE board_id = ?1",
                params![board_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(BoardError::BoardNotFound(
                crate::board::error::redact_identifier(board_id),
            ));
        }
        let mut stmt = db.prepare(
            "SELECT name, columns_json FROM board_series WHERE board_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![board_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (name, columns_json) = r?;
            out.push(SeriesDef {
                name,
                columns: serde_json::from_str(&columns_json)?,
            });
        }
        Ok(out)
    }

    fn series_def(&self, board_id: &str, series: &str) -> Result<SeriesDef, BoardError> {
        self.series_defs(board_id)?
            .into_iter()
            .find(|d| d.name == series)
            .ok_or_else(|| BoardError::SeriesNotFound {
                board: crate::board::error::redact_identifier(board_id),
                series: crate::board::error::redact_identifier(series),
            })
    }

    /// Append or replace rows on a series.
    ///
    /// The whole push is one `BEGIN IMMEDIATE` transaction: a partially
    /// applied push would leave a series that no writer can reconcile,
    /// since the per-writer sequence would have advanced past rows that
    /// were never stored.
    pub fn push(&self, req: &PushRequest) -> Result<PushOutcome, BoardError> {
        let def = self.series_def(&req.board_id, &req.series)?;
        let rows = ingest::validate_push(req, &def, &self.caps)?;

        let now = Utc::now();
        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        // Re-check existence INSIDE the transaction.
        //
        // The lookup above ran before `BEGIN IMMEDIATE`, and there are
        // no foreign keys on this schema. Another *process* — the whole
        // point of this store — can run `delete_board` in that window,
        // leaving this push to insert rows nothing references and to
        // update zero board rows. Multi-process is the design, so the
        // check has to be inside the lock that makes it meaningful.
        require_series_in_tx(&tx, &req.board_id, &req.series)?;

        // Idempotency is checked inside the transaction so two
        // concurrent retries of the same push cannot both pass the
        // check and both append.
        if let Some(key) = &req.idem_key {
            let prior: Option<i64> = tx
                .query_row(
                    "SELECT rows_added FROM board_pushes WHERE board_id = ?1 AND idem_key = ?2",
                    params![req.board_id, key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(rows_added) = prior {
                return Ok(PushOutcome {
                    rows_added: rows_added as usize,
                    deduplicated: true,
                    sequence_gap: None,
                });
            }
        }

        if req.mode == super::ingest::PushMode::Replace {
            tx.execute(
                "DELETE FROM board_rows WHERE board_id = ?1 AND series = ?2",
                params![req.board_id, req.series],
            )?;
        }

        let writer_key = req.writer.key();
        let last_seq: Option<i64> = tx
            .query_row(
                "SELECT MAX(writer_seq) FROM board_rows \
                 WHERE board_id = ?1 AND series = ?2 AND writer_key = ?3",
                params![req.board_id, req.series, writer_key],
                |r| r.get(0),
            )
            .optional()?
            .flatten();

        let base_seq = req.writer_seq.unwrap_or_else(|| last_seq.unwrap_or(0) + 1);
        check_seq_range(base_seq, rows.len())?;
        let sequence_gap = ingest::detect_gap(last_seq, base_seq);

        // A writer that re-sends a sequence it already used, with a
        // different payload, is a bug in the writer — not a retry.
        // Surface it rather than interleaving two versions of history.
        //
        // This applies **regardless of `idem_key`**. The key lookup
        // above already returned for a genuine retry, so reaching here
        // with `base_seq <= prev` means a *different* push is reusing a
        // sequence — and the earlier `idem_key.is_none()` condition let
        // exactly that through whenever the caller varied the key.
        if let Some(prev) = last_seq {
            if base_seq <= prev {
                return Err(BoardError::SequenceReplay {
                    writer: crate::board::error::redact_identifier(&req.writer.label),
                    series: crate::board::error::redact_identifier(&req.series),
                    seq: base_seq,
                });
            }
        }

        let total_after: i64 = tx.query_row(
            "SELECT COUNT(*) FROM board_rows WHERE board_id = ?1 AND series = ?2",
            params![req.board_id, req.series],
            |r| r.get(0),
        )?;
        let projected = total_after as usize + rows.len();
        if projected > self.caps.max_rows_per_series {
            return Err(BoardError::CapExceeded {
                what: "rows per series",
                limit: self.caps.max_rows_per_series,
                actual: projected,
            });
        }

        let pushed_at = now.to_rfc3339();
        let to_insert: Vec<InsertRow<'_>> = rows
            .iter()
            .enumerate()
            .map(|(offset, values)| InsertRow {
                values,
                writer: &req.writer,
                writer_seq: base_seq + offset as i64,
                pushed_at: &pushed_at,
            })
            .collect();
        insert_rows_tx(&tx, &req.board_id, &req.series, &to_insert)?;

        if let Some(key) = &req.idem_key {
            tx.execute(
                "INSERT INTO board_pushes (board_id, idem_key, rows_added, applied_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![req.board_id, key, rows.len() as i64, now.to_rfc3339()],
            )?;
        }

        tx.execute(
            "UPDATE boards SET updated_at = ?1 WHERE board_id = ?2",
            params![now.to_rfc3339(), req.board_id],
        )?;
        tx.commit()?;

        Ok(PushOutcome {
            rows_added: rows.len(),
            deduplicated: false,
            sequence_gap,
        })
    }

    /// Drop every row on a series, keeping its definition.
    pub fn clear_series(&self, board_id: &str, series: &str) -> Result<usize, BoardError> {
        self.series_def(board_id, series)?;
        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_series_in_tx(&tx, board_id, series)?;
        let n = tx.execute(
            "DELETE FROM board_rows WHERE board_id = ?1 AND series = ?2",
            params![board_id, series],
        )?;
        // Clearing is a destructive mutation, so it moves `updated_at`
        // like any other. Without this, the list sorts a just-emptied
        // board as untouched and its freshness column reads as a lie.
        tx.execute(
            "UPDATE boards SET updated_at = ?1 WHERE board_id = ?2",
            params![Utc::now().to_rfc3339(), board_id],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// Read rows oldest-first, capped at `limit`.
    ///
    /// Opens and closes its read within the call — never hand a
    /// transaction to a renderer (see the module docs).
    pub fn read_rows(
        &self,
        board_id: &str,
        series: &str,
        limit: usize,
    ) -> Result<Vec<Row>, BoardError> {
        self.read_rows_paged(board_id, series, 0, limit)
    }

    /// One bounded page of rows, oldest-first.
    ///
    /// The paging unit for export. Ordered by `row_id`, which is
    /// insertion order and stable — ordering by a timestamp column
    /// would let a writer's clock reshuffle history.
    pub fn read_rows_paged(
        &self,
        board_id: &str,
        series: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Row>, BoardError> {
        self.read_rows_query(
            board_id,
            series,
            "ORDER BY row_id LIMIT ?3 OFFSET ?4",
            limit as i64,
            offset as i64,
        )
    }

    /// A board, its series definitions, row counts, and the tail of each
    /// series — read inside **one** transaction.
    ///
    /// Assembling this from separate calls let a concurrent writer land
    /// between them, so a caller could report `row_count = 200` beside
    /// 201 rows, or an `updated_at` older than the rows it shipped. A
    /// deferred read transaction gives a consistent snapshot; in WAL
    /// mode it does not block writers.
    ///
    /// Bounded by construction, so it does not violate the
    /// never-hold-a-transaction-across-a-render rule in this module's
    /// header: it opens and closes within the call.
    pub fn detail_snapshot(
        &self,
        board_id: &str,
        tail: usize,
    ) -> Result<BoardDetailSnapshot, BoardError> {
        let mut db = self.db();
        let tx = db.transaction()?;

        let raw = tx
            .query_row(
                "SELECT board_id, name, spec_json, spec_revision, created_at, updated_at \
                 FROM boards WHERE board_id = ?1",
                params![board_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| {
                BoardError::BoardNotFound(crate::board::error::redact_identifier(board_id))
            })?;
        let board = build_board(raw)?;

        let source_board_id: Option<String> = tx.query_row(
            "SELECT source_board_id FROM boards WHERE board_id = ?1",
            params![board_id],
            |r| r.get(0),
        )?;

        let mut defs: Vec<SeriesDef> = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT name, columns_json FROM board_series WHERE board_id = ?1 ORDER BY name",
            )?;
            let rows = stmt.query_map(params![board_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (name, columns_json) = row?;
                defs.push(SeriesDef {
                    name,
                    columns: serde_json::from_str(&columns_json)?,
                });
            }
        }

        let mut series = Vec::with_capacity(defs.len());
        for def in &defs {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_rows WHERE board_id = ?1 AND series = ?2",
                params![board_id, def.name],
                |r| r.get(0),
            )?;
            let mut stmt = tx.prepare(
                "SELECT writer_kind, writer_label, writer_seq, values_json, pushed_at FROM ( \
                   SELECT row_id, writer_kind, writer_label, writer_seq, values_json, pushed_at \
                     FROM board_rows WHERE board_id = ?1 AND series = ?2 \
                    ORDER BY row_id DESC LIMIT ?3 \
                 ) ORDER BY row_id ASC",
            )?;
            let raw = stmt.query_map(params![board_id, def.name, tail as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?;
            let mut rows = Vec::new();
            for row in raw {
                let (kind, label, seq, values_json, pushed_at) = row?;
                rows.push(map_row(
                    def,
                    &def.name,
                    kind,
                    label,
                    seq,
                    values_json,
                    pushed_at,
                )?);
            }
            series.push(SeriesSnapshot {
                def: def.clone(),
                row_count: count as usize,
                rows,
            });
        }

        Ok(BoardDetailSnapshot {
            board,
            source_board_id,
            series,
        })
    }

    /// Every board with its series names, total row count, and the
    /// **latest** reported writer — in three queries total, not
    /// `1 + boards × series`.
    ///
    /// The per-board loop this replaces issued `series_defs` plus a
    /// `row_count` and a one-row read per series. At 50 boards × 32
    /// series that is thousands of round trips to paint one list.
    pub fn list_summaries(&self) -> Result<Vec<BoardSummary>, BoardError> {
        let boards = self.list_boards()?;
        let db = self.db();

        let mut series_by_board: std::collections::HashMap<String, Vec<String>> =
            Default::default();
        {
            let mut stmt =
                db.prepare("SELECT board_id, name FROM board_series ORDER BY board_id, name")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (bid, name) = row?;
                series_by_board.entry(bid).or_default().push(name);
            }
        }

        let mut counts: std::collections::HashMap<String, i64> = Default::default();
        {
            let mut stmt =
                db.prepare("SELECT board_id, COUNT(*) FROM board_rows GROUP BY board_id")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (bid, n) = row?;
                counts.insert(bid, n);
            }
        }

        // Latest row per board by `row_id`, which is insertion order.
        // Ordering by a timestamp column would let a writer's clock
        // decide whose claim is shown.
        let mut latest: std::collections::HashMap<String, String> = Default::default();
        {
            let mut stmt = db.prepare(
                "SELECT r.board_id, r.writer_label FROM board_rows r \
                 JOIN (SELECT board_id, MAX(row_id) AS m FROM board_rows GROUP BY board_id) t \
                   ON t.board_id = r.board_id AND t.m = r.row_id",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (bid, label) = row?;
                latest.insert(bid, label);
            }
        }

        Ok(boards
            .into_iter()
            .map(|b| BoardSummary {
                series: series_by_board.remove(&b.board_id).unwrap_or_default(),
                total_rows: counts.get(&b.board_id).copied().unwrap_or(0) as usize,
                reported_writer: latest.get(&b.board_id).cloned(),
                board: b,
            })
            .collect())
    }

    /// The **last** `limit` rows of a series, in order.
    ///
    /// `read_rows` returns the *first* N, which is right for an export
    /// and wrong for a display: a long-lived board would render its
    /// oldest history under a header saying it just updated.
    /// One statement, not `COUNT` then `OFFSET`.
    ///
    /// The two-step version raced: a concurrent append between the count
    /// and the paged read shifted the window, so the "last N" could skip
    /// the newest row or repeat an older one. Selecting `row_id DESC
    /// LIMIT n` and reversing is atomic within the single query.
    pub fn read_rows_tail(
        &self,
        board_id: &str,
        series: &str,
        limit: usize,
    ) -> Result<Vec<Row>, BoardError> {
        let def = self.series_def(board_id, series)?;
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT writer_kind, writer_label, writer_seq, values_json, pushed_at FROM ( \
               SELECT row_id, writer_kind, writer_label, writer_seq, values_json, pushed_at \
                 FROM board_rows WHERE board_id = ?1 AND series = ?2 \
                ORDER BY row_id DESC LIMIT ?3 \
             ) ORDER BY row_id ASC",
        )?;
        let raw = stmt.query_map(params![board_id, series, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in raw {
            let (kind, label, seq, values_json, pushed_at) = row?;
            out.push(map_row(
                &def,
                series,
                kind,
                label,
                seq,
                values_json,
                pushed_at,
            )?);
        }
        Ok(out)
    }

    /// The highest `row_id` currently in a series, or 0 when empty.
    ///
    /// Captured once at the start of an export to fix an upper bound —
    /// see [`read_rows_after`](Self::read_rows_after).
    pub fn series_max_row_id(&self, board_id: &str, series: &str) -> Result<i64, BoardError> {
        self.series_def(board_id, series)?;
        let db = self.db();
        let max: Option<i64> = db
            .query_row(
                "SELECT MAX(row_id) FROM board_rows WHERE board_id = ?1 AND series = ?2",
                params![board_id, series],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(max.unwrap_or(0))
    }

    /// One page of rows with `after < row_id <= upper`, oldest first.
    ///
    /// # Why not `LIMIT`/`OFFSET`
    ///
    /// This store is written by other processes while it is being read.
    /// An offset-paged scan re-evaluates the whole result set on every
    /// page, so a concurrent insert shifts later pages and duplicates a
    /// row, while a concurrent delete shifts them the other way and
    /// **skips** one. Export produced a file that never corresponded to
    /// any single state of the board.
    ///
    /// A cursor plus an upper bound captured before the first page
    /// gives the guarantee that is actually achievable without holding
    /// a transaction open: **every row that existed when the export
    /// started, and still exists when its page is read, is emitted
    /// exactly once and in order.** Rows appended mid-export are
    /// excluded by the upper bound rather than partially included.
    /// Rows deleted mid-export are absent — that is a real limit, not a
    /// paging artifact, and no lock-free scheme avoids it.
    pub fn read_rows_after(
        &self,
        board_id: &str,
        series: &str,
        after: i64,
        upper: i64,
        limit: usize,
    ) -> Result<Vec<(i64, Row)>, BoardError> {
        let def = self.series_def(board_id, series)?;
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT writer_kind, writer_label, writer_seq, values_json, pushed_at, row_id \
             FROM board_rows WHERE board_id = ?1 AND series = ?2 \
             AND row_id > ?3 AND row_id <= ?4 \
             ORDER BY row_id LIMIT ?5",
        )?;
        let raw = stmt.query_map(params![board_id, series, after, upper, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;

        let mut out = Vec::new();
        for r in raw {
            let (kind, label, seq, values_json, pushed_at, row_id) = r?;
            let row = map_row(&def, series, kind, label, seq, values_json, pushed_at)?;
            out.push((row_id, row));
        }
        Ok(out)
    }

    fn read_rows_query(
        &self,
        board_id: &str,
        series: &str,
        tail: &str,
        a: i64,
        b: i64,
    ) -> Result<Vec<Row>, BoardError> {
        let def = self.series_def(board_id, series)?;
        let db = self.db();
        let sql = format!(
            "SELECT writer_kind, writer_label, writer_seq, values_json, pushed_at \
             FROM board_rows WHERE board_id = ?1 AND series = ?2 {tail}"
        );
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map(params![board_id, series, a, b], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;

        let mut out = Vec::new();
        for r in rows {
            let (kind, label, seq, values_json, pushed_at) = r?;
            out.push(map_row(
                &def,
                series,
                kind,
                label,
                seq,
                values_json,
                pushed_at,
            )?);
        }
        Ok(out)
    }

    pub fn row_count(&self, board_id: &str, series: &str) -> Result<usize, BoardError> {
        self.series_def(board_id, series)?;
        let db = self.db();
        let n: i64 = db.query_row(
            "SELECT COUNT(*) FROM board_rows WHERE board_id = ?1 AND series = ?2",
            params![board_id, series],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// The id this board carried in the envelope it was imported from,
    /// or `None` for a board created directly.
    pub fn source_board_id(&self, board_id: &str) -> Result<Option<String>, BoardError> {
        let db = self.db();
        db.query_row(
            "SELECT source_board_id FROM boards WHERE board_id = ?1",
            params![board_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .ok_or_else(|| BoardError::BoardNotFound(crate::board::error::redact_identifier(board_id)))
    }

    /// Import a whole envelope — board, series, and every row — in
    /// **one** transaction.
    ///
    /// Replaces a create-then-import-each-series sequence that was not
    /// atomic in two ways: a failure partway left a half-imported board
    /// behind, and the compensating `delete_board` that papered over it
    /// could delete rows another process had written to the
    /// briefly-visible board in between. One transaction has neither
    /// problem — no other connection ever observes a partial import.
    pub fn import_board(
        &self,
        name: &str,
        spec: &BoardSpec,
        source_board_id: &str,
        series: &[super::export::SeriesExport],
    ) -> Result<String, BoardError> {
        check_name(name)?;
        if series.len() > self.caps.max_series_per_board {
            return Err(BoardError::CapExceeded {
                what: "series per board",
                limit: self.caps.max_series_per_board,
                actual: series.len(),
            });
        }

        let defs: Vec<SeriesDef> = series
            .iter()
            .map(|s| SeriesDef {
                name: s.name.clone(),
                columns: s.columns.clone(),
            })
            .collect();
        for def in &defs {
            def.validate(self.caps.max_columns_per_series)?;
        }
        let mut names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        if names.len() != before {
            return Err(BoardError::InvalidSpec(
                "duplicate series names on one board".to_string(),
            ));
        }
        spec.validate(&defs)?;

        // Validate every row of every series BEFORE opening the
        // transaction, so the transaction is short and a bad envelope
        // fails without ever having taken a write lock.
        let mut prepared: Vec<(usize, Vec<Vec<Value>>, Vec<(WriterId, i64, String)>)> = Vec::new();
        for (i, s) in series.iter().enumerate() {
            let (values, meta) = self.validate_envelope_series(&defs[i], s)?;
            prepared.push((i, values, meta));
        }

        let board_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let spec_json = serde_json::to_string(spec)?;

        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO boards \
             (board_id, name, spec_json, spec_revision, created_at, updated_at, source_board_id) \
             VALUES (?1, ?2, ?3, 1, ?4, ?4, ?5)",
            params![board_id, name, spec_json, now, source_board_id],
        )?;
        for def in &defs {
            tx.execute(
                "INSERT INTO board_series (board_id, name, columns_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    board_id,
                    def.name,
                    serde_json::to_string(&def.columns)?,
                    now
                ],
            )?;
        }
        for (i, values, meta) in &prepared {
            let rows: Vec<InsertRow<'_>> = values
                .iter()
                .zip(meta.iter())
                .map(|(v, (writer, seq, pushed_at))| InsertRow {
                    values: v,
                    writer,
                    writer_seq: *seq,
                    pushed_at,
                })
                .collect();
            insert_rows_tx(&tx, &board_id, &defs[*i].name, &rows)?;
        }
        tx.commit()?;
        Ok(board_id)
    }

    /// Type- and metadata-check one envelope series against a
    /// definition. Shared by [`import_board`](Self::import_board) and
    /// [`import_rows`](Self::import_rows).
    #[allow(clippy::type_complexity)]
    fn validate_envelope_series(
        &self,
        def: &SeriesDef,
        series: &super::export::SeriesExport,
    ) -> Result<(Vec<Vec<Value>>, Vec<(WriterId, i64, String)>), BoardError> {
        if series.rows.len() > self.caps.max_rows_per_series {
            return Err(BoardError::CapExceeded {
                what: "rows per series",
                limit: self.caps.max_rows_per_series,
                actual: series.rows.len(),
            });
        }

        let mut typed = Vec::with_capacity(series.rows.len());
        let mut meta = Vec::with_capacity(series.rows.len());
        for row in &series.rows {
            if row.values.len() != def.columns.len() {
                return Err(BoardError::ColumnCountMismatch {
                    series: crate::board::error::redact_identifier(&def.name),
                    expected: def.columns.len(),
                    actual: row.values.len(),
                });
            }
            let mut values = Vec::with_capacity(row.values.len());
            for (cell, column) in row.values.iter().zip(def.columns.iter()) {
                let value = Value::from_json(cell, column, &def.name)?;
                // The same cell-size cap `push` enforces. An envelope
                // is untrusted input; letting import skip a limit the
                // write path applies is how the two paths drifted in
                // the first place.
                if let Value::String(s) = &value {
                    if s.len() > self.caps.max_cell_bytes {
                        return Err(BoardError::CapExceeded {
                            what: "bytes in a string cell",
                            limit: self.caps.max_cell_bytes,
                            actual: s.len(),
                        });
                    }
                }
                values.push(value);
            }

            check_seq_range(row.writer_seq, 1)?;

            let kind = WriterKind::parse(&row.reported_writer_kind).ok_or_else(|| {
                BoardError::CorruptRow {
                    series: crate::board::error::redact_identifier(&def.name),
                    what: "envelope row has an unknown writer kind",
                }
            })?;
            // Normalize to the store's canonical RFC 3339 rather than
            // echoing the envelope's bytes, so what goes in is exactly
            // what comes back out.
            let pushed_at = DateTime::parse_from_rfc3339(&row.pushed_at)
                .map(|t| t.with_timezone(&Utc).to_rfc3339())
                .map_err(|_| BoardError::CorruptRow {
                    series: crate::board::error::redact_identifier(&def.name),
                    what: "envelope row pushed_at is not RFC 3339",
                })?;

            typed.push(values);
            meta.push((
                WriterId::new(kind, row.reported_writer_label.clone()),
                row.writer_seq,
                pushed_at,
            ));
        }
        Ok((typed, meta))
    }

    /// Restore one series' rows into an existing board.
    ///
    /// Prefer [`import_board`](Self::import_board) for a whole
    /// envelope — it is atomic. This exists for appending a single
    /// series into a board that already exists.
    ///
    /// Preserves each row's reported writer and sequence verbatim.
    /// Rewriting them to say "Import" would destroy the record of what
    /// was originally claimed, and would not make anything more
    /// trustworthy — provenance was never authenticated (see
    /// [`super::series::WriterId`]).
    pub fn import_rows(
        &self,
        board_id: &str,
        series: &super::export::SeriesExport,
    ) -> Result<usize, BoardError> {
        let def = self.series_def(board_id, &series.name)?;
        let (values, meta) = self.validate_envelope_series(&def, series)?;

        let rows: Vec<InsertRow<'_>> = values
            .iter()
            .zip(meta.iter())
            .map(|(v, (writer, seq, pushed_at))| InsertRow {
                values: v,
                writer,
                writer_seq: *seq,
                pushed_at,
            })
            .collect();

        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        require_series_in_tx(&tx, board_id, &series.name)?;

        // `validate_envelope_series` caps the envelope's own row count,
        // which is the whole story for `import_board` (the series is
        // new). Appending into an *existing* series has to count what
        // is already there, or repeated imports walk past
        // `max_rows_per_series` one envelope at a time.
        let existing: i64 = tx.query_row(
            "SELECT COUNT(*) FROM board_rows WHERE board_id = ?1 AND series = ?2",
            params![board_id, series.name],
            |r| r.get(0),
        )?;
        let projected = existing as usize + rows.len();
        if projected > self.caps.max_rows_per_series {
            return Err(BoardError::CapExceeded {
                what: "rows per series",
                limit: self.caps.max_rows_per_series,
                actual: projected,
            });
        }

        insert_rows_tx(&tx, board_id, &series.name, &rows)?;
        tx.commit()?;
        Ok(series.rows.len())
    }

    /// `PRAGMA data_version` — bumps when **another connection**
    /// commits. See [`super::monitor`] for why this and not a file
    /// watcher.
    pub fn data_version(&self) -> Result<i64, BoardError> {
        let db = self.db();
        Ok(db.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }
}

/// Decode one stored row against its series definition.
///
/// Shared by every read path so the corruption rules cannot drift
/// between them.
#[allow(clippy::too_many_arguments)]
fn map_row(
    def: &SeriesDef,
    series: &str,
    kind: String,
    label: String,
    seq: i64,
    values_json: String,
    pushed_at: String,
) -> Result<Row, BoardError> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(&values_json)?;
    if raw.len() != def.columns.len() {
        return Err(BoardError::CorruptRow {
            series: crate::board::error::redact_identifier(series),
            what: "stored cell count does not match the series definition",
        });
    }
    let mut values = Vec::with_capacity(raw.len());
    for (cell, column) in raw.iter().zip(def.columns.iter()) {
        values.push(Value::from_json(cell, column, series)?);
    }

    // Fail loud on a row that cannot be read back.
    //
    // Both of these used to substitute a plausible value — an
    // unparseable kind became `System`, and an unparseable timestamp
    // became `Utc::now()`, so one corrupt row displayed a *different*
    // time on every read and exported a fabricated one. `SchemaTooNew`
    // already refuses a database written by a newer build, so within a
    // supported version this is corruption; inventing a substitute is
    // the "render an unverified claim as fact" failure this whole
    // design exists to avoid.
    let writer_kind = WriterKind::parse(&kind).ok_or_else(|| BoardError::CorruptRow {
        series: crate::board::error::redact_identifier(series),
        what: "unknown writer kind",
    })?;
    let pushed_at = DateTime::parse_from_rfc3339(&pushed_at)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| BoardError::CorruptRow {
            series: crate::board::error::redact_identifier(series),
            what: "pushed_at is not RFC 3339",
        })?;

    Ok(Row {
        values,
        writer_seq: seq,
        provenance: Provenance {
            writer: WriterId::new(writer_kind, label),
            pushed_at,
            // Never true in this design; see series::Provenance.
            verified: false,
        },
    })
}

/// One row on its way into `board_rows`, with its metadata already
/// validated by the caller.
struct InsertRow<'a> {
    values: &'a [Value],
    writer: &'a WriterId,
    writer_seq: i64,
    /// Canonical RFC 3339. Both callers normalize before constructing.
    pushed_at: &'a str,
}

/// The single path by which a row enters `board_rows`.
///
/// `push` and `import_rows` both used to carry their own copy of this
/// INSERT, and they drifted: import skipped the sequence-range and
/// timestamp checks that push enforced, which is how an envelope could
/// store a row the store could not read back. One path means one set of
/// rules, and a future writer cannot reintroduce the gap by adding a
/// third copy.
fn insert_rows_tx(
    tx: &rusqlite::Transaction<'_>,
    board_id: &str,
    series: &str,
    rows: &[InsertRow<'_>],
) -> Result<(), BoardError> {
    let mut stmt = tx.prepare(
        "INSERT INTO board_rows \
         (board_id, series, writer_key, writer_kind, writer_label, writer_seq, values_json, pushed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for row in rows {
        let values: Vec<serde_json::Value> = row.values.iter().map(|v| v.to_json()).collect();
        stmt.execute(params![
            board_id,
            series,
            row.writer.key(),
            row.writer.kind.as_str(),
            row.writer.label,
            row.writer_seq,
            serde_json::to_string(&values)?,
            row.pushed_at,
        ])?;
    }
    Ok(())
}

/// Assert that a board and series still exist, from inside a write
/// transaction.
///
/// The schema carries no foreign keys, so this is what stops a
/// concurrent `delete_board` in another process from stranding rows.
fn require_series_in_tx(
    tx: &rusqlite::Transaction<'_>,
    board_id: &str,
    series: &str,
) -> Result<(), BoardError> {
    let board: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM boards WHERE board_id = ?1",
            params![board_id],
            |r| r.get(0),
        )
        .optional()?;
    if board.is_none() {
        return Err(BoardError::BoardNotFound(
            crate::board::error::redact_identifier(board_id),
        ));
    }
    let found: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM board_series WHERE board_id = ?1 AND name = ?2",
            params![board_id, series],
            |r| r.get(0),
        )
        .optional()?;
    if found.is_none() {
        return Err(BoardError::SeriesNotFound {
            board: crate::board::error::redact_identifier(board_id),
            series: crate::board::error::redact_identifier(series),
        });
    }
    Ok(())
}

/// Reject a writer sequence that cannot be stored for a `rows`-row push.
///
/// Sequences start at 1, so 0 and negatives are invalid. The upper
/// bound matters because the default for the *next* push is
/// `last_seq + 1`: an imported `i64::MAX` would make that overflow, and
/// a wrapped sequence silently reorders history rather than failing.
fn check_seq_range(base_seq: i64, rows: usize) -> Result<(), BoardError> {
    let out_of_range = base_seq < 1
        || i64::try_from(rows)
            .ok()
            .and_then(|n| base_seq.checked_add(n))
            .is_none();
    if out_of_range {
        return Err(BoardError::SequenceOutOfRange {
            seq: base_seq,
            rows,
        });
    }
    Ok(())
}

fn build_board(raw: (String, String, String, i64, String, String)) -> Result<Board, BoardError> {
    let (board_id, name, spec_json, spec_revision, created_at, updated_at) = raw;
    Ok(Board {
        board_id,
        name,
        spec: serde_json::from_str(&spec_json)?,
        spec_revision,
        created_at: parse_ts(&created_at),
        updated_at: parse_ts(&updated_at),
    })
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn row_to_board(db: &Connection, board_id: &str) -> Result<Board, BoardError> {
    let raw = db
        .query_row(
            "SELECT board_id, name, spec_json, spec_revision, created_at, updated_at \
             FROM boards WHERE board_id = ?1",
            params![board_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            BoardError::BoardNotFound(crate::board::error::redact_identifier(board_id))
        })?;
    build_board(raw)
}

/// Apply migrations under a single `BEGIN IMMEDIATE`.
///
/// A version newer than this build understands is an error, not a
/// best-effort read: an older Claudepot writing rows under a newer
/// schema's assumptions is how user data gets silently mangled.
///
/// # The decision is re-read INSIDE the transaction
///
/// This store is opened directly by the GUI, the CLI, the MCP server
/// subprocess and any script, with no channel between them (see the
/// module docs). Deciding from a version read *before* `BEGIN
/// IMMEDIATE` lets two processes both observe the pre-migration number
/// and both run the same migration: the first commits, the second
/// re-applies it to an already-migrated database.
///
/// That was survivable only by accident. v1 is `CREATE TABLE IF NOT
/// EXISTS`, so a double-apply is a no-op — but the comment below
/// promises the next migration will "ALTER and backfill", and a
/// backfill applied twice corrupts rows in a file whose whole premise
/// is that it holds user data existing nowhere else.
///
/// The read outside the lock is kept as a **fast path only**. It is
/// safe there because the version is monotonic: "already current" can
/// never become "needs migrating", so an early return on that answer
/// cannot be racing anything. Every answer that leads to a *write* is
/// re-derived under the lock, where the loser of a race sees the
/// version the winner committed and does nothing. Taking `BEGIN
/// IMMEDIATE` unconditionally would be simpler and wrong in the other
/// direction — it would serialize every `open()` behind any concurrent
/// writer, on the common path where there is nothing to migrate.
fn migrate(db: &Connection) -> Result<(), BoardError> {
    if !needs_migration(read_version(db)?)? {
        return Ok(());
    }

    // `busy_timeout` (via `apply_standard_pragmas`) applies at `BEGIN
    // IMMEDIATE`, so a concurrent migration is waited on, not raced.
    db.execute_batch("BEGIN IMMEDIATE;")?;

    let apply = |db: &Connection| -> Result<(), BoardError> {
        // Authoritative re-read: whatever we saw above is now stale by
        // however long we waited for the write lock.
        let version = read_version(db)?;
        if !needs_migration(version)? {
            return Ok(());
        }
        if version < 1 {
            db.execute_batch(SCHEMA_V1)?;
        }
        // Future migrations append here as `if version < N { … }`.
        // They ALTER and backfill; they never drop and rebuild.
        db.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        Ok(())
    };

    match apply(db) {
        Ok(()) => {
            db.execute_batch("COMMIT;")?;
            Ok(())
        }
        Err(e) => {
            let _ = db.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

fn read_version(db: &Connection) -> Result<i64, BoardError> {
    Ok(db.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// `Ok(true)` when this build must migrate, `Ok(false)` when the file
/// is already current, `Err` when it was written by a newer build.
///
/// One function so the fast path and the under-lock re-read cannot
/// disagree — two copies of a three-way comparison, in the code that
/// decides whether to rewrite user data, is how the fast path ends up
/// meaning something subtly different from the authoritative one.
fn needs_migration(version: i64) -> Result<bool, BoardError> {
    if version > SCHEMA_VERSION {
        return Err(BoardError::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(version < SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::ingest::PushMode;
    use crate::board::series::{Column, ColumnType};
    use crate::board::spec::{WidgetKind, WidgetSlot};

    fn runs_series() -> SeriesDef {
        SeriesDef::new(
            "runs",
            vec![
                Column::new("at", ColumnType::Timestamp),
                Column::new("cost", ColumnType::Number),
            ],
        )
    }

    fn store() -> (tempfile::TempDir, BoardStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = BoardStore::open(&dir.path().join("boards.db")).unwrap();
        (dir, s)
    }

    fn writer() -> WriterId {
        WriterId::new(WriterKind::AgentRun, "nightly")
    }

    fn push_req(board: &str, rows: Vec<serde_json::Value>) -> PushRequest {
        PushRequest {
            board_id: board.to_string(),
            series: "runs".to_string(),
            rows,
            mode: PushMode::Append,
            writer: writer(),
            idem_key: None,
            writer_seq: None,
        }
    }

    fn row(at: &str, cost: f64) -> serde_json::Value {
        serde_json::json!([at, cost])
    }

    #[test]
    fn open_creates_schema_and_stamps_the_version() {
        let (_d, s) = store();
        let db = s.db();
        let v: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn reopen_is_idempotent_and_preserves_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");
        let id = {
            let s = BoardStore::open(&path).unwrap();
            let b = s
                .create_board("nightly", &BoardSpec::empty(), &[runs_series()])
                .unwrap();
            s.push(&push_req(
                &b.board_id,
                vec![row("2026-07-31T00:00:00+00:00", 1.5)],
            ))
            .unwrap();
            b.board_id
        };
        // Migrations must preserve rows — boards.db is user data.
        let s = BoardStore::open(&path).unwrap();
        assert_eq!(s.row_count(&id, "runs").unwrap(), 1);
    }

    #[test]
    fn a_newer_schema_version_refuses_to_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 999;").unwrap();
        }
        assert!(matches!(
            BoardStore::open(&path),
            Err(BoardError::SchemaTooNew { found: 999, .. })
        ));
    }

    /// `needs_migration` is the one three-way comparison both the fast
    /// path and the under-lock re-read consult. Locking its boundaries
    /// separately is what stops the two from drifting apart.
    #[test]
    fn needs_migration_covers_all_three_answers() {
        assert!(matches!(needs_migration(0), Ok(true)), "fresh file");
        assert!(
            matches!(needs_migration(SCHEMA_VERSION), Ok(false)),
            "already current — the early-return the fast path relies on"
        );
        assert!(matches!(
            needs_migration(SCHEMA_VERSION + 1),
            Err(BoardError::SchemaTooNew { .. })
        ));
    }

    /// Many processes opening one boards.db concurrently is the normal
    /// case — GUI, CLI and the MCP subprocess all open it directly with
    /// no channel between them. Every one must land on a consistent
    /// schema, and the rows written through them must all survive.
    ///
    /// This passed before the fix too, because v1 is `CREATE TABLE IF
    /// NOT EXISTS` and double-applying it is a no-op. It is here as the
    /// regression guard for the migration *after* this one: the moment
    /// a `version < 2` arm does an ALTER or a backfill, a re-applied
    /// migration stops being harmless and this is the shape that
    /// catches it.
    #[test]
    fn concurrent_opens_converge_on_one_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = BoardStore::open(&path).expect("concurrent open must succeed");
                    let b = s
                        .create_board(&format!("b{i}"), &BoardSpec::empty(), &[runs_series()])
                        .expect("create");
                    s.push(&push_req(
                        &b.board_id,
                        vec![row("2026-07-31T00:00:00+00:00", i as f64)],
                    ))
                    .expect("push");
                    b.board_id
                })
            })
            .collect();

        let ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let s = BoardStore::open(&path).unwrap();
        let v: i64 = s
            .db()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        for id in &ids {
            assert_eq!(
                s.row_count(id, "runs").unwrap(),
                1,
                "a concurrent writer's row was lost"
            );
        }
    }

    #[test]
    fn create_and_get_round_trip() {
        let (_d, s) = store();
        let b = s
            .create_board("nightly", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let got = s.get_board(&b.board_id).unwrap();
        assert_eq!(got.name, "nightly");
        assert_eq!(got.spec_revision, 1);
    }

    #[test]
    fn board_names_are_not_unique_because_the_id_is_the_identity() {
        let (_d, s) = store();
        let a = s
            .create_board("same", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let b = s
            .create_board("same", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        assert_ne!(a.board_id, b.board_id);
    }

    #[test]
    fn get_missing_board_is_not_found() {
        let (_d, s) = store();
        assert!(matches!(
            s.get_board("nope"),
            Err(BoardError::BoardNotFound(_))
        ));
    }

    #[test]
    fn update_spec_bumps_the_revision() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let spec = BoardSpec::new(vec![WidgetSlot {
            id: "cost".into(),
            kind: WidgetKind::Line,
            series: "runs".into(),
            title: "Cost".into(),
            x_column: Some("at".into()),
            y_column: Some("cost".into()),
        }]);
        let updated = s.update_spec(&b.board_id, &spec, 1).unwrap();
        assert_eq!(updated.spec_revision, 2);
        assert_eq!(updated.spec.widgets.len(), 1);
    }

    #[test]
    fn stale_revision_conflicts_rather_than_overwriting() {
        // Plan F8: never last-write-wins.
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.update_spec(&b.board_id, &BoardSpec::empty(), 1).unwrap();
        let err = s.update_spec(&b.board_id, &BoardSpec::empty(), 1);
        assert!(matches!(
            err,
            Err(BoardError::RevisionConflict {
                base: 1,
                current: 2,
                ..
            })
        ));
    }

    #[test]
    fn push_appends_and_reads_back_typed() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let out = s
            .push(&push_req(
                &b.board_id,
                vec![
                    row("2026-07-31T00:00:00+00:00", 1.5),
                    row("2026-07-31T01:00:00+00:00", 2.5),
                ],
            ))
            .unwrap();
        assert_eq!(out.rows_added, 2);
        assert!(!out.deduplicated);

        let rows = s.read_rows(&b.board_id, "runs", 100).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values[1], Value::Number(1.5));
        assert_eq!(rows[0].writer_seq, 1);
        assert_eq!(rows[1].writer_seq, 2);
    }

    #[test]
    fn provenance_reads_back_as_unverified() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let rows = s.read_rows(&b.board_id, "runs", 10).unwrap();
        assert!(!rows[0].provenance.verified);
        assert_eq!(rows[0].provenance.reported_label(), "Reported by: nightly");
    }

    #[test]
    fn repeated_idem_key_is_a_silent_no_op() {
        // A retried cron job must not double-append.
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let mut req = push_req(&b.board_id, vec![row("2026-07-31T00:00:00+00:00", 1.0)]);
        req.idem_key = Some("run-2026-07-31".to_string());

        let first = s.push(&req).unwrap();
        assert_eq!(first.rows_added, 1);
        assert!(!first.deduplicated);

        let second = s.push(&req).unwrap();
        assert!(second.deduplicated);
        assert_eq!(s.row_count(&b.board_id, "runs").unwrap(), 1);
    }

    #[test]
    fn replace_mode_clears_before_appending() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let mut req = push_req(&b.board_id, vec![row("2026-07-31T02:00:00+00:00", 9.0)]);
        req.mode = PushMode::Replace;
        s.push(&req).unwrap();
        let rows = s.read_rows(&b.board_id, "runs", 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values[1], Value::Number(9.0));
    }

    #[test]
    fn type_mismatch_is_rejected_and_nothing_is_written() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let bad = push_req(
            &b.board_id,
            vec![
                row("2026-07-31T00:00:00+00:00", 1.0),
                serde_json::json!(["2026-07-31T01:00:00+00:00", "not-a-number"]),
            ],
        );
        assert!(s.push(&bad).is_err());
        // The whole push is one transaction: a partial apply would
        // advance the writer sequence past rows that were never stored.
        assert_eq!(s.row_count(&b.board_id, "runs").unwrap(), 0);
    }

    #[test]
    fn push_to_a_missing_series_is_an_error() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let mut req = push_req(&b.board_id, vec![row("2026-07-31T00:00:00+00:00", 1.0)]);
        req.series = "absent".into();
        assert!(matches!(
            s.push(&req),
            Err(BoardError::SeriesNotFound { .. })
        ));
    }

    #[test]
    fn two_writers_share_a_series_with_independent_sequences() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let mut other = push_req(&b.board_id, vec![row("2026-07-31T01:00:00+00:00", 2.0)]);
        other.writer = WriterId::new(WriterKind::Cli, "manual");
        s.push(&other).unwrap();

        let rows = s.read_rows(&b.board_id, "runs", 10).unwrap();
        assert_eq!(rows.len(), 2);
        // Each writer starts its own sequence at 1 — multiple writers
        // are legal and visible, not an error.
        assert_eq!(rows[0].writer_seq, 1);
        assert_eq!(rows[1].writer_seq, 1);
        assert_ne!(
            rows[0].provenance.writer.label,
            rows[1].provenance.writer.label
        );
    }

    #[test]
    fn explicit_sequence_gap_is_surfaced_not_hidden() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let mut jump = push_req(&b.board_id, vec![row("2026-07-31T01:00:00+00:00", 2.0)]);
        jump.writer_seq = Some(5);
        let out = s.push(&jump).unwrap();
        assert_eq!(out.sequence_gap, Some((2, 4)));
    }

    #[test]
    fn sequence_replay_without_an_idem_key_is_an_error() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let mut replay = push_req(&b.board_id, vec![row("2026-07-31T01:00:00+00:00", 2.0)]);
        replay.writer_seq = Some(1);
        assert!(matches!(
            s.push(&replay),
            Err(BoardError::SequenceReplay { .. })
        ));
    }

    #[test]
    fn delete_board_removes_its_rows_and_series() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        s.delete_board(&b.board_id).unwrap();
        assert!(matches!(
            s.get_board(&b.board_id),
            Err(BoardError::BoardNotFound(_))
        ));
        assert!(s.series_defs(&b.board_id).is_err());
    }

    #[test]
    fn delete_missing_board_is_an_error_not_a_silent_success() {
        let (_d, s) = store();
        assert!(matches!(
            s.delete_board("nope"),
            Err(BoardError::BoardNotFound(_))
        ));
    }

    #[test]
    fn clear_series_keeps_the_definition() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        assert_eq!(s.clear_series(&b.board_id, "runs").unwrap(), 1);
        assert_eq!(s.row_count(&b.board_id, "runs").unwrap(), 0);
        assert_eq!(s.series_defs(&b.board_id).unwrap().len(), 1);
    }

    #[test]
    fn rows_per_series_cap_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let caps = IngestCaps {
            max_rows_per_series: 2,
            ..IngestCaps::default()
        };
        let s = BoardStore::open_with_caps(&dir.path().join("boards.db"), caps).unwrap();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![
                row("2026-07-31T00:00:00+00:00", 1.0),
                row("2026-07-31T01:00:00+00:00", 2.0),
            ],
        ))
        .unwrap();
        assert!(matches!(
            s.push(&push_req(
                &b.board_id,
                vec![row("2026-07-31T02:00:00+00:00", 3.0)]
            )),
            Err(BoardError::CapExceeded { .. })
        ));
    }

    #[test]
    fn duplicate_series_names_on_one_board_are_rejected() {
        let (_d, s) = store();
        assert!(s
            .create_board("n", &BoardSpec::empty(), &[runs_series(), runs_series()])
            .is_err());
    }

    #[test]
    fn tail_returns_the_newest_rows_in_order() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let rows: Vec<serde_json::Value> = (0..10)
            .map(|i| serde_json::json!(["2026-07-31T00:00:00+00:00", i as f64]))
            .collect();
        s.push(&push_req(&b.board_id, rows)).unwrap();

        let tail = s.read_rows_tail(&b.board_id, "runs", 3).unwrap();
        assert_eq!(tail.len(), 3);
        // Newest three, still oldest-first inside the window. Reading
        // the HEAD here is what made a long board render stale history
        // under a "just updated" header.
        assert_eq!(tail[0].values[1], Value::Number(7.0));
        assert_eq!(tail[2].values[1], Value::Number(9.0));
    }

    #[test]
    fn tail_of_an_empty_series_is_empty_not_an_error() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        assert!(s.read_rows_tail(&b.board_id, "runs", 5).unwrap().is_empty());
    }

    #[test]
    fn detail_snapshot_row_count_agrees_with_the_rows_it_returns() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![
                row("2026-07-31T00:00:00+00:00", 1.0),
                row("2026-07-31T01:00:00+00:00", 2.0),
            ],
        ))
        .unwrap();

        let snap = s.detail_snapshot(&b.board_id, 100).unwrap();
        assert_eq!(snap.board.board_id, b.board_id);
        assert_eq!(snap.series.len(), 1);
        assert_eq!(snap.series[0].row_count, 2);
        assert_eq!(snap.series[0].rows.len(), 2);
        assert_eq!(snap.source_board_id, None);
    }

    #[test]
    fn detail_snapshot_of_a_missing_board_is_not_found() {
        let (_d, s) = store();
        assert!(matches!(
            s.detail_snapshot("nope", 10),
            Err(BoardError::BoardNotFound(_))
        ));
    }

    #[test]
    fn list_summaries_reports_the_latest_writer_not_the_first() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let mut second = push_req(&b.board_id, vec![row("2026-07-31T01:00:00+00:00", 2.0)]);
        second.writer = WriterId::new(WriterKind::Cli, "later-writer");
        s.push(&second).unwrap();

        let sums = s.list_summaries().unwrap();
        assert_eq!(sums.len(), 1);
        assert_eq!(sums[0].total_rows, 2);
        assert_eq!(sums[0].series, vec!["runs".to_string()]);
        assert_eq!(sums[0].reported_writer.as_deref(), Some("later-writer"));
    }

    #[test]
    fn list_summaries_handles_a_board_with_no_rows() {
        let (_d, s) = store();
        s.create_board("empty", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let sums = s.list_summaries().unwrap();
        assert_eq!(sums[0].total_rows, 0);
        assert_eq!(sums[0].reported_writer, None);
    }

    #[test]
    fn data_version_is_readable() {
        let (_d, s) = store();
        assert!(s.data_version().is_ok());
    }

    // ---- audit regressions -----------------------------------------

    #[test]
    fn sequence_replay_is_rejected_even_with_a_fresh_idem_key() {
        // The guard used to be skipped whenever *any* idem_key was
        // present, so varying the key walked straight past it. The key
        // lookup has already returned for a genuine retry by this
        // point, so reaching the guard means a different push is
        // reusing a sequence.
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let mut first = push_req(&b.board_id, vec![row("2026-07-31T00:00:00+00:00", 1.0)]);
        first.idem_key = Some("key-a".into());
        s.push(&first).unwrap();

        let mut replay = push_req(&b.board_id, vec![row("2026-07-31T01:00:00+00:00", 2.0)]);
        replay.idem_key = Some("key-b".into());
        replay.writer_seq = Some(1);
        assert!(matches!(
            s.push(&replay),
            Err(BoardError::SequenceReplay { .. })
        ));
        assert_eq!(s.row_count(&b.board_id, "runs").unwrap(), 1);
    }

    #[test]
    fn writer_sequence_below_one_is_rejected() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        for bad in [0i64, -1, i64::MIN] {
            let mut req = push_req(&b.board_id, vec![row("2026-07-31T00:00:00+00:00", 1.0)]);
            req.writer_seq = Some(bad);
            assert!(
                matches!(s.push(&req), Err(BoardError::SequenceOutOfRange { .. })),
                "seq {bad} was accepted"
            );
        }
    }

    #[test]
    fn a_writer_sequence_that_would_overflow_is_rejected() {
        // An imported i64::MAX would make the next default of
        // `last_seq + 1` wrap, silently reordering history.
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let mut req = push_req(&b.board_id, vec![row("2026-07-31T00:00:00+00:00", 1.0)]);
        req.writer_seq = Some(i64::MAX);
        assert!(matches!(
            s.push(&req),
            Err(BoardError::SequenceOutOfRange { .. })
        ));
    }

    #[test]
    fn a_push_after_another_process_deleted_the_board_creates_no_rows() {
        // Two store handles on one file is the real topology. The
        // schema has no foreign keys, so nothing but the in-transaction
        // check stops this from stranding rows.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");
        let writer = BoardStore::open(&path).unwrap();
        let deleter = BoardStore::open(&path).unwrap();

        let b = writer
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        deleter.delete_board(&b.board_id).unwrap();

        let err = writer.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ));
        assert!(err.is_err(), "push into a deleted board succeeded");

        let orphans: i64 = deleter
            .db()
            .query_row(
                "SELECT COUNT(*) FROM board_rows WHERE board_id = ?1",
                params![b.board_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0, "orphan rows survived the delete");
    }

    #[test]
    fn clearing_a_series_moves_updated_at() {
        // Otherwise a just-emptied board sorts as untouched and its
        // freshness column reads as a lie.
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let before = s.get_board(&b.board_id).unwrap().updated_at;
        s.clear_series(&b.board_id, "runs").unwrap();
        let after = s.get_board(&b.board_id).unwrap().updated_at;
        assert!(after >= before, "updated_at went backwards");
        assert_ne!(
            s.get_board(&b.board_id).unwrap().updated_at,
            b.created_at,
            "clear left updated_at at creation time"
        );
    }

    #[test]
    fn a_corrupt_stored_timestamp_fails_loud_instead_of_reading_as_now() {
        // The old fallback made one corrupt row display a *different*
        // time on every read and export a fabricated one.
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        s.db()
            .execute(
                "UPDATE board_rows SET pushed_at = 'not-a-time' WHERE board_id = ?1",
                params![b.board_id],
            )
            .unwrap();

        assert!(matches!(
            s.read_rows(&b.board_id, "runs", 10),
            Err(BoardError::CorruptRow { .. })
        ));
    }

    #[test]
    fn a_corrupt_stored_writer_kind_fails_loud_instead_of_reading_as_system() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        s.db()
            .execute(
                "UPDATE board_rows SET writer_kind = 'wat' WHERE board_id = ?1",
                params![b.board_id],
            )
            .unwrap();

        assert!(matches!(
            s.read_rows(&b.board_id, "runs", 10),
            Err(BoardError::CorruptRow { .. })
        ));
    }

    #[test]
    fn cursor_paging_emits_every_row_exactly_once() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        let rows: Vec<serde_json::Value> = (0..25)
            .map(|i| serde_json::json!(["2026-07-31T00:00:00+00:00", i as f64]))
            .collect();
        s.push(&push_req(&b.board_id, rows)).unwrap();

        let upper = s.series_max_row_id(&b.board_id, "runs").unwrap();
        let mut seen = Vec::new();
        let mut cursor = 0i64;
        loop {
            let page = s
                .read_rows_after(&b.board_id, "runs", cursor, upper, 10)
                .unwrap();
            if page.is_empty() {
                break;
            }
            for (id, r) in &page {
                cursor = *id;
                seen.push(r.values[1].clone());
            }
        }
        assert_eq!(seen.len(), 25);
        let mut ids: Vec<String> = seen.iter().map(|v| v.to_display()).collect();
        ids.dedup();
        assert_eq!(ids.len(), 25, "cursor paging duplicated a row");
    }

    #[test]
    fn rows_appended_mid_export_are_excluded_by_the_upper_bound() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();
        let upper = s.series_max_row_id(&b.board_id, "runs").unwrap();

        // A concurrent writer appends after the bound was captured.
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T01:00:00+00:00", 2.0)],
        ))
        .unwrap();

        let page = s
            .read_rows_after(&b.board_id, "runs", 0, upper, 100)
            .unwrap();
        assert_eq!(page.len(), 1, "late row leaked into a bounded scan");
    }

    /// The whole trust model for boards is "filesystem permissions on
    /// `~/.claudepot/`" (plan §11.2). There is no channel and no
    /// credential, so if the mode is wrong there is nothing else
    /// holding the boundary — which makes this the single most
    /// important assertion in the module.
    ///
    /// Covers the sidecars too: a 0600 main DB beside a 0644 `-wal`
    /// leaks every recently-written row.
    #[cfg(unix)]
    #[test]
    fn db_and_its_wal_sidecars_are_user_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");
        let s = BoardStore::open(&path).unwrap();
        // Force the WAL and SHM sidecars to exist by committing.
        s.create_board("perms", &BoardSpec::empty(), &[runs_series()])
            .unwrap();

        for candidate in [
            path.clone(),
            path.with_extension("db-wal"),
            path.with_extension("db-shm"),
        ] {
            if !candidate.exists() {
                continue;
            }
            let mode = std::fs::metadata(&candidate).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode,
                0o600,
                "{} is {mode:o}, expected 600 — the board trust boundary is file permissions",
                candidate.display()
            );
        }
    }

    /// Plan §12: boards are user data and nothing prunes them
    /// automatically. Every row-removing path must be one the user
    /// explicitly asked for.
    #[test]
    fn nothing_removes_rows_without_an_explicit_call() {
        let (_d, s) = store();
        let b = s
            .create_board("n", &BoardSpec::empty(), &[runs_series()])
            .unwrap();
        s.push(&push_req(
            &b.board_id,
            vec![row("2026-07-31T00:00:00+00:00", 1.0)],
        ))
        .unwrap();

        // Re-opening, reading, listing, and monitoring must all be
        // non-destructive — a cache-shaped reflex here would silently
        // delete a board whose writer has gone away.
        let reopened = BoardStore::open(&_d.path().join("boards.db")).unwrap();
        reopened.list_boards().unwrap();
        reopened.read_rows(&b.board_id, "runs", 10).unwrap();
        reopened.series_defs(&b.board_id).unwrap();
        reopened.data_version().unwrap();

        assert_eq!(reopened.row_count(&b.board_id, "runs").unwrap(), 1);
    }
}
