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
"#;

/// Names of the seven tables created by the v4 DDL block. Used by
/// the migration's post-write validation to assert the transaction
/// produced what it promised before committing.
pub const V4_TABLE_NAMES: &[&str] = &[
    "exchanges",
    "exchange_fts",
    "tool_calls",
    "memories",
    "decisions",
    "evidence_records",
    "memory_links",
];

/// Names of the FTS5 maintenance triggers on `exchanges`. The
/// migration validator checks for each one; a partial DDL apply
/// that loses any of these would silently desynchronize the FTS
/// index from `exchanges` on subsequent writes.
pub const V4_TRIGGER_NAMES: &[&str] = &[
    "exchange_fts_ai",
    "exchange_fts_ad",
    "exchange_fts_au",
];
