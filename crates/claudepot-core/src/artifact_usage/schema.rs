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
//! Cascade: when a `sessions` row is deleted by `refresh()`, its
//! `usage_event` rows go with it (`ON DELETE CASCADE`). `usage_daily`
//! is NOT cascade-deleted — those are aggregates that survive
//! transcript deletion. A full `rebuild()` truncates both.

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
pub const SCHEMA_VERSION: &str = "3";

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
