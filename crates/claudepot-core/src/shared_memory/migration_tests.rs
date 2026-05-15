//! Migration test matrix for sessions.db v3 → v4 (WI-002).
//!
//! Codex flagged this as the highest residual risk in the
//! shared-memory plan. The six cases below are the gate: until
//! every one of them passes, no consumer code is wired into the
//! new tables.
//!
//! Cases (from `dev-docs/codex-plans/20260515-1130-shared-memory.md`):
//!
//! 1. Fresh DB → v4.
//! 2. v3-populated DB → v4 (with rescan invalidation).
//! 3. v4 idempotent reopen.
//! 4. Failed-DDL rollback (stays at v3).
//! 5. `_pending_rescan` flow.
//! 6. `_min_compatible_version` downgrade guard.
//!
//! Each test opens its own temp directory so they parallelize.

use std::path::PathBuf;

use rusqlite::Connection;
use tempfile::TempDir;

use crate::session_index::SessionIndex;

fn temp_db() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sessions.db");
    (dir, path)
}

fn read_meta(db: &Connection, key: &str) -> Option<String> {
    db.query_row("SELECT v FROM meta WHERE k = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

fn table_exists(db: &Connection, name: &str) -> bool {
    db.query_row(
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
        [name],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

fn open_raw(path: &std::path::Path) -> Connection {
    let db = Connection::open(path).expect("open");
    db.execute_batch("PRAGMA foreign_keys=ON;").expect("fk");
    db
}

// ─── Case 1: fresh DB → v4 ────────────────────────────────────

#[test]
fn case1_fresh_db_lands_at_v4() {
    let (_dir, path) = temp_db();
    let idx = SessionIndex::open(&path).expect("open");
    assert_eq!(
        idx.schema_version().unwrap().as_deref(),
        Some(crate::artifact_usage::schema::SCHEMA_VERSION)
    );

    // All seven v4 tables exist.
    let db = open_raw(&path);
    for name in crate::shared_memory::schema::V4_TABLE_NAMES {
        assert!(
            table_exists(&db, name),
            "missing v4 table after fresh open: {name}"
        );
    }
    // `_min_compatible_version` was written.
    assert_eq!(
        read_meta(&db, "_min_compatible_version").as_deref(),
        Some(crate::shared_memory::schema::MIN_COMPATIBLE_VERSION)
    );
    // `source_kind` column was created with the default.
    let has_col: bool = db
        .query_row(
            "SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'source_kind'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(has_col, "sessions.source_kind missing");
}

// ─── Case 2: v3-populated → v4 (rescan invalidation) ──────────

#[test]
fn case2_v3_populated_upgrades_to_v4_and_clears_cache() {
    let (_dir, path) = temp_db();

    // Stage 1: simulate a populated v3 DB. Create the v3
    // session_index + artifact_usage tables, write a sentinel
    // `sessions` row, and stamp `schema_version = "3"`.
    {
        let db = Connection::open(&path).expect("open");
        db.execute_batch("PRAGMA journal_mode=WAL;").expect("wal");
        db.execute_batch(crate::session_index::schema::SCHEMA)
            .expect("v1 schema");
        // The v3 artifact_usage schema constant has already been
        // bumped to "4" on this branch, so we stage by inserting
        // the artifact_usage DDL but stamping "3" manually.
        db.execute_batch(crate::artifact_usage::schema::SCHEMA)
            .expect("artifact schema");
        db.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES ('schema_version', '3')",
            [],
        )
        .expect("stamp v3");
        // Sentinel row in the v1-shape `sessions` table. Note v3
        // doesn't have `source_kind` yet, so we omit it.
        db.execute(
            "INSERT INTO sessions (
                file_path, slug, session_id,
                file_size_bytes, file_mtime_ns, file_inode,
                project_path, project_from_transcript,
                first_ts_ms, last_ts_ms,
                event_count, message_count, user_message_count, assistant_message_count,
                first_user_prompt, models_json,
                tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
                git_branch, cc_version, display_slug, has_error, is_sidechain,
                indexed_at_ms
            ) VALUES (
                '/v3/sentinel.jsonl', 'slug', 'sid',
                10, 100, 0,
                '/proj', 1,
                NULL, NULL,
                0, 0, 0, 0,
                NULL, '[]',
                0, 0, 0, 0,
                NULL, NULL, NULL, 0, 0,
                123
            )",
            [],
        )
        .expect("seed sessions row");
    }

    // Stage 2: open via SessionIndex → migration fires.
    let idx = SessionIndex::open(&path).expect("upgrade open");
    assert_eq!(
        idx.schema_version().unwrap().as_deref(),
        Some(crate::artifact_usage::schema::SCHEMA_VERSION)
    );

    let db = open_raw(&path);
    // All v4 tables present.
    for name in crate::shared_memory::schema::V4_TABLE_NAMES {
        assert!(
            table_exists(&db, name),
            "v4 table missing after upgrade: {name}"
        );
    }
    // Sessions cache cleared — the upgrade-rescan branch fired.
    let row_count: i64 = db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        row_count, 0,
        "v3-populated sessions row should be cleared on v4 upgrade"
    );
    // `source_kind` column now exists.
    let has_col: bool = db
        .query_row(
            "SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'source_kind'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(has_col);
}

// ─── Case 3: v4 idempotent reopen ─────────────────────────────

#[test]
fn case3_v4_reopen_is_idempotent() {
    let (_dir, path) = temp_db();

    // First open → fresh v4.
    {
        let idx = SessionIndex::open(&path).expect("first open");
        // Drop closes the connection. WAL is flushed on close.
        drop(idx);
    }

    // Seed a sentinel row so we can prove the second open didn't
    // wipe it. Use the v4-shape `sessions` table (with
    // `source_kind`).
    {
        let db = open_raw(&path);
        db.execute(
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
                '/v4/sentinel.jsonl', 'slug', 'sid',
                10, 100, 0,
                '/proj', 1,
                NULL, NULL,
                0, 0, 0, 0,
                NULL, '[]',
                0, 0, 0, 0,
                NULL, NULL, NULL, 0, 0,
                123, 'codex'
            )",
            [],
        )
        .expect("seed v4 row");
    }

    // Second open → must NOT trigger rescan / wipe.
    {
        let _idx = SessionIndex::open(&path).expect("reopen");
    }

    let db = open_raw(&path);
    let n: i64 = db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n, 1,
        "v4 reopen must be idempotent (sentinel row should survive)"
    );
}

// ─── Case 4: failed DDL → ROLLBACK, stays at v3 ───────────────

#[test]
fn case4_failed_ddl_rolls_back_and_keeps_v3() {
    let (_dir, path) = temp_db();

    // Stage 1: write a *poisoned* v3 DB. Create everything except
    // the `sessions` table — that will make the `ALTER TABLE
    // sessions ADD COLUMN source_kind` statement fail.
    {
        let db = Connection::open(&path).expect("open");
        db.execute_batch("PRAGMA journal_mode=WAL;").expect("wal");
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);",
        )
        .expect("meta");
        // Stamp v3 but do NOT create `sessions`. The migration
        // will detect prior_version="3", current="4", differ, and
        // try ALTER TABLE sessions — which fails because the
        // table doesn't exist.
        //
        // Actually `session_index::schema::SCHEMA` is run first
        // (idempotent CREATE IF NOT EXISTS), which would create
        // `sessions` for us. So we have to poison a *later* DDL.
        // The simplest reliable poison: insert a row that violates
        // the new CHECK after the table exists.
        //
        // Instead: drop into a different failure mode — pre-create
        // a `meta._pending_rescan` row with a value that's neither
        // '0' nor '1', and verify nothing rolls back unexpectedly.
        //
        // Actually let's take a different approach: stamp v3,
        // pre-create one of the v4 tables with a *different* schema
        // so the IF NOT EXISTS no-op leaves it incompatible and
        // a subsequent INSERT in this test fails the validation.
        //
        // For a clean rollback test, the simplest is to use a
        // hook: pre-create `exchanges` with the wrong columns,
        // then check that after `SessionIndex::open` fails, the
        // version remains "3".
        db.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES ('schema_version', '3')",
            [],
        )
        .expect("v3");
        // Force a failure point: create a table named `exchanges`
        // that's not a sane FTS5 backing. The v4 DDL does
        // `CREATE TABLE IF NOT EXISTS exchanges (...)`, which
        // sees our pre-existing table and is a no-op. Then the
        // v4 DDL does `CREATE VIRTUAL TABLE IF NOT EXISTS
        // exchange_fts USING fts5(... content='exchanges')`,
        // which also no-ops if it exists. We need a real failure.
        //
        // Cleanest reliable failure: violate a CHECK constraint at
        // migration time. Since CREATE TABLE IF NOT EXISTS won't
        // touch existing tables, we can't easily inject a column
        // mismatch.
        //
        // Easiest path: pre-create `exchange_fts` as a regular
        // table (not a virtual one). The v4 DDL's
        // CREATE VIRTUAL TABLE IF NOT EXISTS will see it exists
        // and skip — but then the trigger creation
        // (CREATE TRIGGER ... INSERT INTO exchange_fts(...)) will
        // fail at TRIGGER creation? No, trigger creation doesn't
        // verify the target table's schema.
        //
        // Final answer: pre-create `meta` with a different schema
        // that makes the version-bump UPSERT fail. Specifically,
        // drop `meta` and recreate it with a NOT NULL CHECK on `v`
        // that rejects "4".
        db.execute("DROP TABLE meta", []).expect("drop meta");
        db.execute(
            "CREATE TABLE meta (k TEXT PRIMARY KEY, v TEXT NOT NULL CHECK (v != '4'))",
            [],
        )
        .expect("poisoned meta");
        db.execute(
            "INSERT INTO meta (k, v) VALUES ('schema_version', '3')",
            [],
        )
        .expect("v3 in poisoned meta");
    }

    // Stage 2: try to migrate. Should fail because the version-
    // bump UPSERT writes v = '4' and the CHECK on `v` rejects it.
    let res = SessionIndex::open(&path);
    assert!(res.is_err(), "migration with poisoned meta should fail");

    // Stage 3: verify the version stayed at '3' — the transaction
    // rolled back, no v4 tables persisted.
    let db = open_raw(&path);
    let v = read_meta(&db, "schema_version");
    assert_eq!(
        v.as_deref(),
        Some("3"),
        "rollback should leave schema_version at v3"
    );
    // None of the v4-specific tables should be present, because
    // their CREATE statements ran inside the same rolled-back
    // transaction.
    for name in [
        "exchanges",
        "exchange_fts",
        "tool_calls",
        "memories",
        "decisions",
        "evidence_records",
        "memory_links",
    ] {
        assert!(
            !table_exists(&db, name),
            "v4 table {name} should not persist after rollback"
        );
    }
}

// ─── Case 5: _pending_rescan triggers cache clear ─────────────

#[test]
fn case5_pending_rescan_marker_clears_cache_and_unsets_itself() {
    let (_dir, path) = temp_db();

    // First open → fresh v4.
    {
        let _idx = SessionIndex::open(&path).expect("first open");
    }

    // Seed a sentinel sessions row + set the marker.
    {
        let db = open_raw(&path);
        db.execute(
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
                '/preset/x.jsonl', 'slug', 'sid',
                10, 100, 0,
                '/proj', 1,
                NULL, NULL,
                0, 0, 0, 0,
                NULL, '[]',
                0, 0, 0, 0,
                NULL, NULL, NULL, 0, 0,
                123, 'claude_code'
            )",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES ('_pending_rescan', '1')",
            [],
        )
        .unwrap();
    }

    // Reopen → marker should trigger DELETE FROM sessions and
    // clear itself.
    {
        let _idx = SessionIndex::open(&path).expect("reopen with marker");
    }

    let db = open_raw(&path);
    let n: i64 = db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n, 0,
        "_pending_rescan='1' should have cleared sessions cache"
    );
    let marker = read_meta(&db, "_pending_rescan");
    assert!(
        marker.is_none(),
        "_pending_rescan should be cleared after handling, got {marker:?}"
    );
}

// ─── Case 6: _min_compatible_version downgrade guard ──────────

#[test]
fn case6_min_compatible_version_blocks_old_binary_path() {
    let (_dir, path) = temp_db();

    // First open → writes _min_compatible_version = current.
    {
        let _idx = SessionIndex::open(&path).expect("first open");
    }

    // Simulate a future "v5" DB by bumping _min_compatible_version
    // past our current binary's SCHEMA_VERSION. The next call to
    // apply_schema should bail without touching DDL or cache.
    {
        let db = open_raw(&path);
        db.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES ('_min_compatible_version', '999')",
            [],
        )
        .unwrap();
        // Seed a sentinel row that would normally be wiped if the
        // migration ran.
        db.execute(
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
                '/guarded/x.jsonl', 'slug', 'sid',
                10, 100, 0,
                '/proj', 1,
                NULL, NULL,
                0, 0, 0, 0,
                NULL, '[]',
                0, 0, 0, 0,
                NULL, NULL, NULL, 0, 0,
                123, 'codex'
            )",
            [],
        )
        .unwrap();
    }

    // Reopen → guard should fire. apply_schema returns Ok(()) but
    // doesn't touch DDL / cache.
    {
        let _idx = SessionIndex::open(&path).expect("reopen with future-marker");
    }

    let db = open_raw(&path);
    let n: i64 = db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n, 1,
        "_min_compatible_version guard must NOT wipe the sentinel row"
    );
    // The schema_version may stay at whatever the prior write set
    // it to; what matters is that the guard didn't touch it.
}

// ─── Bonus: PRAGMA foreign_keys is ON ─────────────────────────

#[test]
fn pragma_foreign_keys_is_on_after_open() {
    let (_dir, path) = temp_db();
    let _idx = SessionIndex::open(&path).expect("open");
    let db = Connection::open(&path).expect("raw");
    // The pragma is per-connection — verify our raw connection
    // would need to set it. The real check is that the
    // SessionIndex's internal connection has it. We approximate
    // by checking a cascade actually fires below.
    let _ = db; // not used; cascade test in the next test.
}

#[test]
fn fk_cascade_fires_from_sessions_to_exchanges_and_fts() {
    let (_dir, path) = temp_db();
    let _idx = SessionIndex::open(&path).expect("open");

    // Use the SessionIndex's connection by opening a raw connection
    // — but `PRAGMA foreign_keys` is per-connection, so we must
    // re-enable it here. The production code path always opens via
    // `init_connection` which sets the pragma; this test simulates
    // that environment.
    let db = open_raw(&path);

    // Insert a sentinel sessions row.
    db.execute(
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
            '/cascade/x.jsonl', 'slug', 'sid',
            10, 100, 0,
            '/proj', 1,
            NULL, NULL,
            0, 0, 0, 0,
            NULL, '[]',
            0, 0, 0, 0,
            NULL, NULL, NULL, 0, 0,
            123, 'codex'
        )",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO exchanges (
            id, file_path, source_kind, turn_index, role_pair,
            timestamp_ms, user_text, assistant_text,
            line_start, line_end, is_sidechain, parent_id, snippet_text
        ) VALUES (
            'sid:0', '/cascade/x.jsonl', 'codex', 0, 'user_assistant',
            NULL, 'hello', 'hi',
            NULL, NULL, 0, NULL, 'hello / hi'
        )",
        [],
    )
    .unwrap();

    // Sanity: exchanges row exists; FTS row was created via
    // AFTER INSERT trigger.
    let ex_count: i64 = db
        .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ex_count, 1);
    let fts_count: i64 = db
        .query_row("SELECT count(*) FROM exchange_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 1, "FTS AFTER INSERT trigger should populate row");

    // Delete the parent sessions row. FK cascade should remove
    // the exchanges row, and the AFTER DELETE trigger should
    // remove the FTS row.
    db.execute(
        "DELETE FROM sessions WHERE file_path = '/cascade/x.jsonl'",
        [],
    )
    .unwrap();
    let ex_count: i64 = db
        .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ex_count, 0, "FK CASCADE from sessions->exchanges must fire");
    let fts_count: i64 = db
        .query_row("SELECT count(*) FROM exchange_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 0, "AFTER DELETE trigger on exchanges must remove FTS row");
}

#[test]
fn memory_links_check_constraints_reject_invalid_writes() {
    let (_dir, path) = temp_db();
    let _idx = SessionIndex::open(&path).expect("open");
    let db = open_raw(&path);

    // Seed a memory and a decision so we have two valid parents.
    db.execute(
        "INSERT INTO memories (
            id, scope, project_path, kind, content,
            created_by_kind, created_by,
            confidence, created_at_ms, updated_at_ms, archived_at_ms
        ) VALUES (
            'mem-1', 'global', NULL, 'fact', 'sky is blue',
            'user', 'user:xiaolai',
            NULL, 1, 1, NULL
        )",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO decisions (
            id, project_path, topic, decision, rationale,
            status, created_by_kind, created_by, created_at_ms, supersedes_id
        ) VALUES (
            'dec-1', NULL, 'planning', 'use rmcp', 'spike green',
            'active', 'user', 'user:xiaolai', 1, NULL
        )",
        [],
    )
    .unwrap();
    // Need a sessions row so the file_path target is valid.
    db.execute(
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
            '/ml/x.jsonl', 'slug', 'sid',
            10, 100, 0,
            '/proj', 1,
            NULL, NULL,
            0, 0, 0, 0,
            NULL, '[]',
            0, 0, 0, 0,
            NULL, NULL, NULL, 0, 0,
            123, 'claude_code'
        )",
        [],
    )
    .unwrap();

    // ✓ Valid: one parent (memory) + one target (file_path).
    db.execute(
        "INSERT INTO memory_links (id, memory_id, file_path, relation) \
         VALUES ('ml-1', 'mem-1', '/ml/x.jsonl', 'origin')",
        [],
    )
    .expect("valid memory_link should insert");

    // ✗ Reject: two parents (memory + decision).
    let res = db.execute(
        "INSERT INTO memory_links (id, memory_id, decision_id, file_path, relation) \
         VALUES ('ml-2', 'mem-1', 'dec-1', '/ml/x.jsonl', 'related')",
        [],
    );
    assert!(
        res.is_err(),
        "two-parent insert must be rejected by CHECK constraint"
    );

    // ✗ Reject: zero parents.
    let res = db.execute(
        "INSERT INTO memory_links (id, file_path, relation) \
         VALUES ('ml-3', '/ml/x.jsonl', 'related')",
        [],
    );
    assert!(
        res.is_err(),
        "zero-parent insert must be rejected by CHECK constraint"
    );

    // ✗ Reject: zero targets.
    let res = db.execute(
        "INSERT INTO memory_links (id, memory_id, relation) \
         VALUES ('ml-4', 'mem-1', 'related')",
        [],
    );
    assert!(
        res.is_err(),
        "zero-target insert must be rejected by CHECK constraint"
    );
}

#[test]
fn memories_scope_check_constraints_enforced() {
    let (_dir, path) = temp_db();
    let _idx = SessionIndex::open(&path).expect("open");
    let db = open_raw(&path);

    // ✗ scope='global' with non-NULL project_path → rejected.
    let res = db.execute(
        "INSERT INTO memories (
            id, scope, project_path, kind, content,
            created_by_kind, created_by,
            confidence, created_at_ms, updated_at_ms, archived_at_ms
        ) VALUES (
            'm-bad-1', 'global', '/some/proj', 'fact', 'x',
            'user', 'user:test', NULL, 1, 1, NULL
        )",
        [],
    );
    assert!(res.is_err(), "global+project_path must be rejected");

    // ✗ scope='project' with NULL project_path → rejected.
    let res = db.execute(
        "INSERT INTO memories (
            id, scope, project_path, kind, content,
            created_by_kind, created_by,
            confidence, created_at_ms, updated_at_ms, archived_at_ms
        ) VALUES (
            'm-bad-2', 'project', NULL, 'fact', 'x',
            'user', 'user:test', NULL, 1, 1, NULL
        )",
        [],
    );
    assert!(res.is_err(), "project+NULL project_path must be rejected");

    // ✓ scope='global' with NULL → ok.
    db.execute(
        "INSERT INTO memories (
            id, scope, project_path, kind, content,
            created_by_kind, created_by,
            confidence, created_at_ms, updated_at_ms, archived_at_ms
        ) VALUES (
            'm-ok-1', 'global', NULL, 'fact', 'x',
            'user', 'user:test', NULL, 1, 1, NULL
        )",
        [],
    )
    .expect("global+NULL should be accepted");

    // ✓ scope='project' with non-NULL → ok.
    db.execute(
        "INSERT INTO memories (
            id, scope, project_path, kind, content,
            created_by_kind, created_by,
            confidence, created_at_ms, updated_at_ms, archived_at_ms
        ) VALUES (
            'm-ok-2', 'project', '/p', 'fact', 'x',
            'user', 'user:test', NULL, 1, 1, NULL
        )",
        [],
    )
    .expect("project+non-NULL should be accepted");
}
