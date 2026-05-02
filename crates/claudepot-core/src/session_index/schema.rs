//! Schema DDL for the persistent session index.
//!
//! Version 1: one row per `.jsonl` transcript. Keyed by canonical file
//! path because that's the only globally unique identifier — two
//! transcripts can legitimately share a `session_id` after an
//! interrupted rescue/adopt.
//!
//! The `(file_size_bytes, file_mtime_ns, file_inode)` triple is the
//! re-parse guard: if any of the three diverges from what the fs
//! reports, the row is re-scanned. Inode catches in-place rewrites
//! (e.g. `session_move` replacing a transcript atomically via
//! create-temp-and-rename) that happen to preserve size+mtime. On
//! platforms where the metadata API doesn't expose an inode (non-Unix),
//! the column is stored as 0 and the guard degrades to (size, mtime_ns).
//!
//! Token counts and message counts are stored as INTEGER. SQLite's
//! i64 upper bound (≈9.2e18) is fine for both — even a session with
//! a trillion tokens wouldn't come close.
//!
//! Future schema bumps land new DDL below and a `schema_version`
//! write in `mod.rs::apply_schema`; see `account.rs` for the
//! additive-ALTER pattern.

/// Sessions-table schema version. The `meta.schema_version` row stores
/// the *highest* version the index file has ever seen (currently the
/// `artifact_usage` schema version "2"). This constant only ratchets
/// when the `sessions` table itself changes.
pub const SCHEMA_VERSION: &str = "1";

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    file_path                TEXT    PRIMARY KEY,
    slug                     TEXT    NOT NULL,
    session_id               TEXT    NOT NULL,
    file_size_bytes          INTEGER NOT NULL,
    file_mtime_ns            INTEGER NOT NULL,
    file_inode               INTEGER NOT NULL,
    project_path             TEXT    NOT NULL,
    project_from_transcript  INTEGER NOT NULL,
    first_ts_ms              INTEGER,
    last_ts_ms               INTEGER,
    event_count              INTEGER NOT NULL,
    message_count            INTEGER NOT NULL,
    user_message_count       INTEGER NOT NULL,
    assistant_message_count  INTEGER NOT NULL,
    first_user_prompt        TEXT,
    models_json              TEXT    NOT NULL,
    tokens_input             INTEGER NOT NULL,
    tokens_output            INTEGER NOT NULL,
    tokens_cache_creation    INTEGER NOT NULL,
    tokens_cache_read        INTEGER NOT NULL,
    git_branch               TEXT,
    cc_version               TEXT,
    display_slug             TEXT,
    has_error                INTEGER NOT NULL,
    is_sidechain             INTEGER NOT NULL,
    indexed_at_ms            INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_last_ts      ON sessions(last_ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_slug         ON sessions(slug);
CREATE INDEX IF NOT EXISTS idx_sessions_project_path ON sessions(project_path);
CREATE INDEX IF NOT EXISTS idx_sessions_session_id   ON sessions(session_id);

-- Per-turn token usage. Populated alongside the session row on every
-- UPSERT (i.e. cold first scan and any subsequent re-scan triggered
-- by the (size, mtime, inode) re-parse guard). Rows for re-scanned
-- files are deleted-and-reinserted in the same transaction so the
-- cache stays consistent with the transcript.
--
-- Sessions on disk before this table existed will be missing turn
-- rows until their next re-scan. Consumer queries must tolerate
-- empty results — the foreign-key relation to `sessions(file_path)`
-- is by convention, not by SQLite constraint, to preserve the
-- existing wipe-and-rebuild recovery semantics.
CREATE TABLE IF NOT EXISTS session_turns (
    file_path           TEXT    NOT NULL,
    turn_index          INTEGER NOT NULL,  -- 0-based assistant-turn ordinal in the transcript
    ts_ms               INTEGER,           -- server-side message timestamp, NULL when unparseable
    model               TEXT    NOT NULL,  -- model id ("claude-opus-4-7"); empty string if missing
    tokens_input        INTEGER NOT NULL,
    tokens_output       INTEGER NOT NULL,
    tokens_cache_creation INTEGER NOT NULL,
    tokens_cache_read   INTEGER NOT NULL,
    user_prompt_preview TEXT,              -- truncated user prompt that drove this assistant turn (NULL if none)
    PRIMARY KEY (file_path, turn_index)
);

CREATE INDEX IF NOT EXISTS idx_turns_file_path ON session_turns(file_path);
CREATE INDEX IF NOT EXISTS idx_turns_ts        ON session_turns(ts_ms DESC);
"#;
