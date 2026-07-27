//! DDL fragments for the usage_event + usage_daily tables.
//!
//! Lives in `sessions.db` as schema v2 (additive over the v1 sessions
//! table). Migration is forward-only — no downgrade path because the
//! cache is always rebuildable from disk.
//!
//! Why two tables:
//!   - `usage_event` is the raw stream, kept on a 30-day rolling window.
//!     Used for last-24h percentiles and drill-down. Cheap to evict.
//!   - `usage_daily` is the rollup, kept indefinitely. Used for the
//!     7d/30d/all-time counters in the UI. ~100 bytes per (artifact,
//!     day) row — even a power user accumulates < 10 MB/year.
//!
//! Cleanup is MANUAL, not a cascade: `usage_event` declares no
//! FOREIGN KEY, so deleting a `sessions` row removes nothing here —
//! even though `PRAGMA foreign_keys=ON` is set on every sessions.db
//! connection and real cascades exist elsewhere in the same DB
//! (`exchanges` → `sessions`). `SessionIndex::refresh` calls
//! `store::subtract_daily_for_file` then `store::delete_events_for_file`
//! inside its write transaction for every re-scanned or vanished
//! file; removing those calls would silently orphan events and
//! corrupt the daily-rollup subtract/re-add invariant. `usage_daily`
//! is never deleted per-file — those aggregates survive transcript
//! deletion. A full `rebuild()` truncates both tables.

/// Schema version stamped into `meta.schema_version`. Acts as the
/// migration trigger across the whole `sessions.db` file (the
/// `sessions` table itself stays at v1 — see the per-table version
/// in `session_index/schema.rs`). Bumped each time *any* table that
/// shares this DB needs an existing-user backfill.
///
/// History:
///   - v1: original `sessions` table.
///   - v2: added `usage_event` + `usage_daily` (artifact usage tracking).
///   - v3: added `session_turns` (per-turn token detail). Bumping this
///     forces a re-scan for existing users so historical transcripts
///     populate the new table on next `refresh()`. Without the bump,
///     unchanged transcripts would never produce per-turn rows and the
///     `top_costly_turns` query would silently return only fresh-after-
///     this-release sessions.
///   - v4: Shared Memory tables (`exchanges`, `tool_calls`,
///     `exchange_fts` + triggers, `memories`, `decisions`,
///     `evidence_records`, `memory_links`) plus `source_kind` column
///     on `sessions`. Bumping forces re-scan so the new `exchanges`
///     table populates for every existing transcript. PRAGMA
///     foreign_keys is also enabled on every connection as part of
///     this version — the existing v3 schema's by-convention FKs
///     finally start enforcing.
///   - v5: `exchanges.id` namespaced by source_kind to remove the
///     theoretical cross-harness collision between identical Claude
///     and Codex session UUIDs. Format went from
///     `<session_id>:<turn_index>` to
///     `<source_kind>:<session_id>:<turn_index>`. Bump forces re-scan
///     so existing rows are rewritten in the new format. memory_links
///     and tool_calls FKs cascade-clear via the existing v4
///     migration's `DELETE FROM sessions`/`session_turns` path.
///   - v6: added `artifact_first_last` — the durable "ever observed"
///     ledger. `usage_daily` cannot answer that question:
///     `subtract_daily_for_file` backs out a transcript's contribution
///     on every re-scan *or vanish*, and `truncate_all` clears it, so
///     its true semantic is "fires from transcripts currently on disk".
///     Pruning sessions (Settings → Cleanup) would therefore make
///     regularly-used artifacts read as never-fired. The ledger is
///     monotonic (MIN/MAX upsert), is never subtracted, and is
///     deliberately NOT cleared by `truncate_all`. The bump forces a
///     re-scan so retained transcripts backfill it — note this is a
///     backfill from transcripts, not from `usage_daily` row
///     existence, which would import zeroed-out false positives.
///   - v7: `ArtifactKind::Mcp` — MCP tool calls are now extracted as
///     usage events (`mcp__<server>__<tool>`). The bump forces a
///     re-scan so historical MCP calls populate `usage_event`,
///     `usage_daily` AND `artifact_first_last`; without it, MCP usage
///     would only start accruing from the next session and every
///     bundled-MCP plugin would keep reading as never-used.
pub const SCHEMA_VERSION: &str = "7";

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_event (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms         INTEGER NOT NULL,
    session_id    TEXT    NOT NULL,
    file_path     TEXT    NOT NULL,
    project_path  TEXT    NOT NULL,
    kind          TEXT    NOT NULL,
    artifact_key  TEXT    NOT NULL,
    plugin_id     TEXT,
    outcome       TEXT    NOT NULL,
    duration_ms   INTEGER,
    extra_json    TEXT
);

CREATE INDEX IF NOT EXISTS idx_usage_event_kind_key_ts
    ON usage_event(kind, artifact_key, ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_event_plugin_ts
    ON usage_event(plugin_id, ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_event_file_path
    ON usage_event(file_path);
CREATE INDEX IF NOT EXISTS idx_usage_event_ts
    ON usage_event(ts_ms);

CREATE TABLE IF NOT EXISTS usage_daily (
    day_unix_s        INTEGER NOT NULL,
    kind              TEXT    NOT NULL,
    artifact_key      TEXT    NOT NULL,
    plugin_id         TEXT,
    fire_count        INTEGER NOT NULL DEFAULT 0,
    error_count       INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL DEFAULT 0,
    duration_count    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day_unix_s, kind, artifact_key)
);

CREATE INDEX IF NOT EXISTS idx_usage_daily_kind_key
    ON usage_daily(kind, artifact_key);
CREATE INDEX IF NOT EXISTS idx_usage_daily_plugin
    ON usage_daily(plugin_id);

-- Durable "ever observed" ledger (schema v6).
--
-- One row per (kind, artifact_key), written monotonically: first_seen
-- only ever moves earlier, last_seen only ever moves later. NEVER
-- subtracted (unlike usage_daily) and NEVER cleared by
-- `store::truncate_all`, so it survives both a session prune and a
-- full index rebuild.
--
-- This is the ONLY sound source for "has this artifact ever fired?".
-- Do not answer that question from usage_daily — see the v6 note on
-- SCHEMA_VERSION for why.
--
-- Coverage boundary: "ever observed **by Claudepot**". Usage that
-- predates this table, or that happened in transcripts deleted before
-- the v6 backfill re-scan, is not represented. The UI must say
-- "no invocation on record", never "never used".
--
-- Growth: one row per DISTINCT (kind, artifact_key) ever observed —
-- not per invocation — and there is deliberately no GC. The bound is
-- the number of distinct artifacts a machine ever runs (hundreds,
-- low thousands for a heavy plugin user) at ~60 bytes/row, so even a
-- 10k-artifact history is well under 1 MB. Adding a retention sweep
-- would reintroduce the exact false-negative this table prevents.
CREATE TABLE IF NOT EXISTS artifact_first_last (
    kind          TEXT    NOT NULL,
    artifact_key  TEXT    NOT NULL,
    first_seen_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL,
    PRIMARY KEY (kind, artifact_key)
);

CREATE INDEX IF NOT EXISTS idx_artifact_first_last_kind
    ON artifact_first_last(kind);
"#;

/// 86400 seconds per day. Floor a ms timestamp to its UTC day.
pub fn day_floor_unix_s(ts_ms: i64) -> i64 {
    let secs = ts_ms.div_euclid(1000);
    secs.div_euclid(86_400) * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_floor_zero_is_epoch() {
        assert_eq!(day_floor_unix_s(0), 0);
    }

    #[test]
    fn day_floor_one_second_before_midnight_returns_same_day() {
        // 86399_999 ms = 23:59:59.999 on day 0
        assert_eq!(day_floor_unix_s(86_399_999), 0);
    }

    #[test]
    fn day_floor_one_ms_into_next_day_advances() {
        // 86_400_000 ms = exactly midnight day 1
        assert_eq!(day_floor_unix_s(86_400_000), 86_400);
    }

    #[test]
    fn day_floor_negative_ts_floors_correctly() {
        // -1 ms → day -1 (UTC seconds floor: -1 / 86400 = -1)
        assert_eq!(day_floor_unix_s(-1), -86_400);
    }
}
