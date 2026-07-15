//! DDL fragments for the Shared Memory tables.
//!
//! Lives in `sessions.db` as part of schema v4 (additive over v3).
//! Migration is forward-only — the cache is always rebuildable
//! from disk, and durable rows (`memories`, `decisions`,
//! `evidence_records`, `memory_links`) survive a "Rebuild Shared
//! Memory" operation by design.
//!
//! Layout:
//!
//! - `exchanges` — one row per user/assistant turn, with
//!   `source_kind` so Claude and Codex coexist. Cascade-deleted
//!   when the parent `sessions` row is removed.
//! - `exchange_fts` — external-content FTS5 over `exchanges`'
//!   text columns. Maintained by AFTER INSERT / DELETE / UPDATE
//!   triggers on `exchanges`.
//! - `tool_calls` — function call + call output paired by
//!   `call_id`. Cascade-deleted when the parent `exchanges` row is
//!   removed (which itself cascades from `sessions`).
//! - `memories`, `decisions`, `evidence_records`, `memory_links`
//!   — user/agent-authored durable rows. NOT cascade-cleared by
//!   "Rebuild Shared Memory"; only "Forget Shared Memory" wipes
//!   them.
//!
//! All CHECK constraints land at the DDL level so partial writes
//! from indexer bugs fail loudly at INSERT time rather than
//! silently corrupting search results.

/// Minimum compatible Claudepot schema version. Written to
/// `meta._min_compatible_version` at v4-migration time. A future
/// older binary that opens this DB checks this row before
/// running its `apply_schema` and bails out (without running its
/// own rescan branch) when its `SCHEMA_VERSION` is lower. See the
/// v4 migration plan in `shared-memory.md` for the forward-only
/// downgrade semantics.
pub const MIN_COMPATIBLE_VERSION: &str = "4";

/// Full v4 DDL block. All statements are `IF NOT EXISTS` so the
/// block is idempotent; running it on a populated v4 DB is a no-op.
///
/// Order matters for foreign keys: parent tables before children.
pub const SCHEMA: &str = r#"
-- ─── exchanges ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS exchanges (
    id              TEXT    PRIMARY KEY,             -- <session_id>:<turn_index>
    file_path       TEXT    NOT NULL,                -- FK to sessions.file_path
    source_kind     TEXT    NOT NULL CHECK (source_kind IN ('claude_code','codex')),
    turn_index      INTEGER NOT NULL,
    role_pair       TEXT    NOT NULL DEFAULT 'user_assistant',
    timestamp_ms    INTEGER,
    user_text       TEXT    NOT NULL,                -- unredacted at rest; redact on emission
    assistant_text  TEXT    NOT NULL,                -- unredacted at rest; redact on emission
    line_start      INTEGER,                         -- 1-based physical JSONL line (or NULL)
    line_end        INTEGER,                         -- 1-based inclusive (or NULL)
    is_sidechain    INTEGER NOT NULL DEFAULT 0,
    parent_id       TEXT,
    snippet_text    TEXT    NOT NULL,                -- pre-redacted (indexer::build_snippet runs RedactionPolicy::default)
    FOREIGN KEY (file_path) REFERENCES sessions(file_path) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_exchanges_file        ON exchanges(file_path);
-- L2 fix: drop the misleadingly-named idx_exchanges_project_ts
-- (it was on (file_path, timestamp_ms), not project_path) and
-- recreate under a name that matches its columns. EXPLAIN QUERY
-- PLAN now reads truthfully. Safe to drop on every migration:
-- DROP INDEX IF EXISTS is idempotent and the recreate is too.
DROP INDEX IF EXISTS idx_exchanges_project_ts;
CREATE INDEX IF NOT EXISTS idx_exchanges_file_ts     ON exchanges(file_path, timestamp_ms);

-- ─── exchange_fts (external-content) ──────────────────────────
-- Text columns only. Metadata filters (source_kind, project_path,
-- git_branch, model, timestamp_ms) live on `exchanges` and join
-- via the FTS rowid. Index does not match on operator tokens
-- inside `user_text` / `assistant_text` because the search call
-- wraps user input as an FTS5 phrase query before MATCH.
CREATE VIRTUAL TABLE IF NOT EXISTS exchange_fts USING fts5(
    user_text,
    assistant_text,
    snippet_text,
    content='exchanges'
);

-- ─── exchange_fts maintenance triggers ────────────────────────
-- AFTER INSERT: index the new row.
CREATE TRIGGER IF NOT EXISTS exchange_fts_ai
AFTER INSERT ON exchanges
BEGIN
    INSERT INTO exchange_fts(rowid, user_text, assistant_text, snippet_text)
    VALUES (new.rowid, new.user_text, new.assistant_text, new.snippet_text);
END;

-- AFTER DELETE: tell FTS to drop the row. Fires under FK cascade too
-- (SQLite runs the cascade before the AFTER DELETE trigger).
CREATE TRIGGER IF NOT EXISTS exchange_fts_ad
AFTER DELETE ON exchanges
BEGIN
    INSERT INTO exchange_fts(exchange_fts, rowid, user_text, assistant_text, snippet_text)
    VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.snippet_text);
END;

-- AFTER UPDATE: delete-then-insert the row (FTS5 doesn't support
-- in-place column updates on external-content tables).
CREATE TRIGGER IF NOT EXISTS exchange_fts_au
AFTER UPDATE ON exchanges
BEGIN
    INSERT INTO exchange_fts(exchange_fts, rowid, user_text, assistant_text, snippet_text)
    VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.snippet_text);
    INSERT INTO exchange_fts(rowid, user_text, assistant_text, snippet_text)
    VALUES (new.rowid, new.user_text, new.assistant_text, new.snippet_text);
END;

-- ─── exchange_state ───────────────────────────────────────────
-- The exchange backfill's OWN staleness marker: the (size, mtime, inode)
-- of each transcript as of the last time its `exchanges` were written.
--
-- It cannot reuse the `sessions` tuple for this. `session_index::refresh`
-- owns that tuple and updates it to match disk; a backfill running after
-- a refresh (which is exactly the startup order) then compares disk
-- against an already-current tuple, concludes nothing changed, and skips
-- the file. Appended turns were therefore never indexed — a transcript
-- grew all session long and its new content never reached `exchanges`
-- or the FTS index.
--
-- Cascades with the session, so a pruned transcript takes its marker
-- with it and a re-added one re-indexes from scratch.
CREATE TABLE IF NOT EXISTS exchange_state (
    file_path  TEXT    PRIMARY KEY,
    size       INTEGER NOT NULL,
    mtime_ns   INTEGER NOT NULL,
    inode      INTEGER NOT NULL,
    FOREIGN KEY (file_path) REFERENCES sessions(file_path) ON DELETE CASCADE
);

-- ─── tool_calls ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS tool_calls (
    id              TEXT    PRIMARY KEY,
    exchange_id     TEXT    NOT NULL,
    tool_name       TEXT    NOT NULL,
    tool_input_json TEXT,
    tool_result_text TEXT,
    is_error        INTEGER NOT NULL DEFAULT 0,
    timestamp_ms    INTEGER,
    FOREIGN KEY (exchange_id) REFERENCES exchanges(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tool_calls_exchange ON tool_calls(exchange_id);

-- ─── memories ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS memories (
    id               TEXT    PRIMARY KEY,
    scope            TEXT    NOT NULL CHECK (scope IN ('global','project')),
    project_path     TEXT,
    kind             TEXT    NOT NULL CHECK (kind IN ('fact','preference','pattern','constraint','summary')),
    content          TEXT    NOT NULL,
    created_by_kind  TEXT    NOT NULL CHECK (created_by_kind IN ('user','agent','import','system')),
    created_by       TEXT    NOT NULL,
    confidence       INTEGER,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    archived_at_ms   INTEGER,
    CHECK (scope = 'global' OR project_path IS NOT NULL),
    CHECK (scope = 'project' OR project_path IS NULL)
);

CREATE INDEX IF NOT EXISTS idx_memories_scope_project ON memories(scope, project_path);
CREATE INDEX IF NOT EXISTS idx_memories_kind          ON memories(kind);

-- ─── decisions ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS decisions (
    id                TEXT    PRIMARY KEY,
    project_path      TEXT,
    topic             TEXT,
    decision          TEXT    NOT NULL,
    rationale         TEXT,
    status            TEXT    NOT NULL CHECK (status IN ('active','superseded','archived')),
    created_by_kind   TEXT    NOT NULL CHECK (created_by_kind IN ('user','agent','import','system')),
    created_by        TEXT    NOT NULL,
    created_at_ms     INTEGER NOT NULL,
    supersedes_id     TEXT REFERENCES decisions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_decisions_project ON decisions(project_path);
CREATE INDEX IF NOT EXISTS idx_decisions_status  ON decisions(status);

-- ─── evidence_records ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS evidence_records (
    id                  TEXT    PRIMARY KEY,
    project_path        TEXT,
    topic               TEXT,
    summary             TEXT    NOT NULL,
    verification        TEXT    NOT NULL,
    files_changed_json  TEXT    NOT NULL,
    confidence          INTEGER NOT NULL,
    created_by_kind     TEXT    NOT NULL CHECK (created_by_kind IN ('user','agent','import','system')),
    created_by          TEXT    NOT NULL,
    created_at_ms       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_evidence_project ON evidence_records(project_path);

-- ─── memory_links ─────────────────────────────────────────────
-- Connects exactly one parent (memory / decision / evidence) to
-- exactly one target (exchange / file_path). CHECK constraints
-- enforce the cardinality so partial writes fail at INSERT.
CREATE TABLE IF NOT EXISTS memory_links (
    id            TEXT    PRIMARY KEY,
    memory_id     TEXT REFERENCES memories(id) ON DELETE CASCADE,
    decision_id   TEXT REFERENCES decisions(id) ON DELETE CASCADE,
    evidence_id   TEXT REFERENCES evidence_records(id) ON DELETE CASCADE,
    exchange_id   TEXT REFERENCES exchanges(id) ON DELETE CASCADE,
    file_path     TEXT REFERENCES sessions(file_path) ON DELETE CASCADE,
    relation      TEXT    NOT NULL CHECK (relation IN ('evidence','origin','related','supersedes')),
    CHECK (
        (CASE WHEN memory_id   IS NOT NULL THEN 1 ELSE 0 END)
      + (CASE WHEN decision_id IS NOT NULL THEN 1 ELSE 0 END)
      + (CASE WHEN evidence_id IS NOT NULL THEN 1 ELSE 0 END) = 1
    ),
    CHECK (
        (CASE WHEN exchange_id IS NOT NULL THEN 1 ELSE 0 END)
      + (CASE WHEN file_path   IS NOT NULL THEN 1 ELSE 0 END) = 1
    )
);

CREATE INDEX IF NOT EXISTS idx_memory_links_memory   ON memory_links(memory_id);
CREATE INDEX IF NOT EXISTS idx_memory_links_decision ON memory_links(decision_id);
CREATE INDEX IF NOT EXISTS idx_memory_links_evidence ON memory_links(evidence_id);
CREATE INDEX IF NOT EXISTS idx_memory_links_exchange ON memory_links(exchange_id);
CREATE INDEX IF NOT EXISTS idx_memory_links_file     ON memory_links(file_path);

-- ─── recurrence_events ────────────────────────────────────────
-- A recurrence: the distiller re-derived, from a NEW session, a lesson
-- that matches one already accepted or suspect in the SAME project — the
-- agent hit a wall we had already learned about. Detected at ingest, and
-- deliberately NOT auto-counted: filed here as 'pending' and surfaced in
-- Review for a human to confirm. Only a confirmed recurrence is a real
-- datum; a fuzzy match silently incrementing a metric is a soft lie (the
-- METR self-report trap), so the metric the dashboard shows counts only
-- confirmed rows.
--
-- Durable, like `memories`: NOT cascade-cleared by "Rebuild Shared
-- Memory". `new_exchange_id` / `new_file_path` are denormalized with NO
-- foreign key — the exchange row is transcript cache and a rebuild deletes
-- it, exactly the reason provenance is denormalized onto `memories`.
CREATE TABLE IF NOT EXISTS recurrence_events (
    id                 TEXT    PRIMARY KEY,
    matched_memory_id  TEXT    NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    project_path       TEXT    NOT NULL,
    new_content        TEXT    NOT NULL,
    new_exchange_id    TEXT,
    new_file_path      TEXT,
    detected_by        TEXT    NOT NULL CHECK (detected_by IN ('anchor','similarity')),
    detected_at_ms     INTEGER NOT NULL,
    status             TEXT    NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending','confirmed','dismissed')),
    confirmed_at_ms    INTEGER
);

CREATE INDEX IF NOT EXISTS idx_recurrence_project ON recurrence_events(project_path);
CREATE INDEX IF NOT EXISTS idx_recurrence_status  ON recurrence_events(status);
CREATE INDEX IF NOT EXISTS idx_recurrence_matched ON recurrence_events(matched_memory_id);
-- Dedup backstop. `record`'s check-then-insert is atomic only within one
-- process's connection mutex; GUI + CLI both open this DB, so a concurrent
-- overlapping harvest could pass two SELECTs and file the same recurrence
-- twice. This UNIQUE index makes the second INSERT fail, which `record`
-- maps back to "already recorded". Safe to add: `record` already dedups,
-- so no existing row set violates it.
CREATE UNIQUE INDEX IF NOT EXISTS idx_recurrence_dedup
    ON recurrence_events(matched_memory_id, new_content);
"#;

/// Names of the tables created by the v4 DDL block. Used by the
/// migration's post-write validation to assert the transaction produced
/// what it promised before committing. `recurrence_events` was added
/// additively (the block is idempotent `IF NOT EXISTS`, so an existing DB
/// gains the table on its next open without a version bump — the same
/// non-destructive path as the `memories` compiler columns).
pub const V4_TABLE_NAMES: &[&str] = &[
    "exchanges",
    "exchange_fts",
    "exchange_state",
    "tool_calls",
    "memories",
    "decisions",
    "evidence_records",
    "memory_links",
    "recurrence_events",
];

/// Names of the FTS5 maintenance triggers on `exchanges`. The
/// migration validator checks for each one; a partial DDL apply
/// that loses any of these would silently desynchronize the FTS
/// index from `exchanges` on subsequent writes.
pub const V4_TRIGGER_NAMES: &[&str] = &["exchange_fts_ai", "exchange_fts_ad", "exchange_fts_au"];

// ─── knowledge-compiler columns on `memories` ───────────────────
//
// Added after the v4 DDL block, by probe + ALTER rather than by a
// SCHEMA_VERSION bump. Read the next paragraph before touching this;
// the reasoning is load-bearing and not obvious.
//
// **A version bump would be actively destructive here.** `apply_schema`
// treats any change in `meta.schema_version` as a *cache invalidation*:
// it runs `DELETE FROM sessions`, and `sessions -> exchanges ->
// memory_links` all cascade on delete. So bumping the version to add a
// column would wipe the transcript cache (recoverable — it re-parses
// from disk) AND delete every row in `memory_links` (NOT recoverable —
// nothing on disk to re-derive it from), even though `memories`,
// `decisions` and `evidence_records` survive "by design".
//
// The version exists to invalidate the *cache*. These columns do not
// invalidate anything: they are additive, on a durable table, and every
// existing row gets a sensible default. So: probe + ALTER, exactly as
// `sessions.source_kind` already does, and leave the version alone.
//
// That same cascade is why provenance is denormalized onto the memory
// row (`origin_exchange_id`, `origin_file_path` — deliberately NO
// foreign keys). A triage queue whose whole value is "click through to
// the exchange that burned you" cannot hang its provenance on
// `memory_links`, which any rebuild deletes. The link table stays a
// convenience index; the memory row is self-sufficient.

/// `(column_name, full ALTER TABLE ... ADD COLUMN fragment)`.
///
/// Order is irrelevant (each is probed independently), but keep the
/// list stable so migration tests read as a diff.
pub const MEMORIES_COMPILER_COLUMNS: &[(&str, &str)] = &[
    // The review gate. Every writer — the MCP `remember` tool, the
    // distiller agent — lands rows as 'proposed'. Only a human moves a
    // row to 'accepted'. A wrong memory that survives review is worse
    // than no memory, because it will be trusted.
    (
        "review_state",
        "ALTER TABLE memories ADD COLUMN review_state TEXT NOT NULL DEFAULT 'accepted' \
         CHECK (review_state IN ('proposed','accepted','rejected','suspect'))",
    ),
    // The compiled imperative form: one line, names a command or a
    // file. Never an overview — those cost tokens and buy nothing.
    (
        "directive",
        "ALTER TABLE memories ADD COLUMN directive TEXT",
    ),
    (
        "compile_target",
        "ALTER TABLE memories ADD COLUMN compile_target TEXT \
         CHECK (compile_target IS NULL OR compile_target IN ('guard','directive','note'))",
    ),
    // Where the compiled guard landed, e.g. "scripts/repo-invariants.sh:6".
    (
        "guard_ref",
        "ALTER TABLE memories ADD COLUMN guard_ref TEXT",
    ),
    // {"files": [...], "commit": "sha"} as of acceptance. When an
    // anchored file changes, the claim goes back to triage as SUSPECT.
    // Invalidation by correctness, not by age or by usage.
    (
        "anchor_json",
        "ALTER TABLE memories ADD COLUMN anchor_json TEXT",
    ),
    (
        "suspect_reason",
        "ALTER TABLE memories ADD COLUMN suspect_reason TEXT",
    ),
    // Provenance. NO foreign key, on purpose — see the note above.
    (
        "origin_exchange_id",
        "ALTER TABLE memories ADD COLUMN origin_exchange_id TEXT",
    ),
    (
        "origin_file_path",
        "ALTER TABLE memories ADD COLUMN origin_file_path TEXT",
    ),
];

/// Idempotently add the knowledge-compiler columns to `memories`.
///
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, so each column is probed
/// against `pragma_table_info` first. Safe to run on every open; a
/// no-op once applied. Runs inside the caller's migration transaction.
///
/// `DEFAULT 'accepted'` for `review_state` is right for pre-existing
/// rows: anything already in `memories` was hand-authored through the
/// MCP `remember` tool, so it was already a human's choice. Defaulting
/// them to 'proposed' would dump a user's entire existing memory set
/// into the triage queue for re-litigation.
pub fn apply_memories_compiler_columns(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    for (name, ddl) in MEMORIES_COMPILER_COLUMNS {
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM pragma_table_info('memories') WHERE name = ?1",
                [name],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !exists {
            tx.execute_batch(ddl)?;
        }
    }
    Ok(())
}
