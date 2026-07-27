//! SQL persistence for usage events and the daily rollup.
//!
//! All functions take a `&Connection` (or `&Transaction`) so callers
//! choose their own atomicity boundary. The session-index refresh
//! groups everything into one transaction; CLI tools may want a fresh
//! connection per call.
//!
//! Functions in this module are deliberately small and stateless.
//! Higher-level rollup orchestration lives in `mod.rs`.

use crate::artifact_usage::model::{ArtifactKind, Outcome, UsageEvent};
use crate::artifact_usage::schema::day_floor_unix_s;
use rusqlite::{params, Connection, Result as SqlResult};

/// Insert one event row. Updates the matching `usage_daily` bucket
/// in the same call so callers don't have to remember to do both.
pub fn insert_event(
    db: &Connection,
    event: &UsageEvent,
    file_path: &str,
    project_path: &str,
) -> SqlResult<()> {
    db.execute(
        "INSERT INTO usage_event (
            ts_ms, session_id, file_path, project_path,
            kind, artifact_key, plugin_id, outcome, duration_ms, extra_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            event.ts_ms,
            event.session_id,
            file_path,
            project_path,
            event.kind.as_str(),
            event.artifact_key,
            event.plugin_id,
            event.outcome.as_str(),
            event.duration_ms.map(|d| d as i64),
            event.extra_json,
        ],
    )?;
    bump_daily(db, event)?;
    bump_first_last(db, event)?;
    Ok(())
}

/// Record the event in the durable `artifact_first_last` ledger.
///
/// Monotonic by construction: `first_seen_ms` only moves earlier,
/// `last_seen_ms` only moves later. There is deliberately **no**
/// subtract counterpart — that is the whole point of the table.
/// `subtract_daily_for_file` exists for `usage_daily` because those
/// counters must track the live event set; "was this ever observed"
/// is not a counter and must survive transcript deletion.
///
/// Re-scanning the same transcript re-inserts the same timestamps, so
/// the MIN/MAX upsert is idempotent — no double-count risk.
fn bump_first_last(db: &Connection, event: &UsageEvent) -> SqlResult<()> {
    db.execute(
        "INSERT INTO artifact_first_last (
            kind, artifact_key, first_seen_ms, last_seen_ms
         ) VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT (kind, artifact_key) DO UPDATE SET
            first_seen_ms = MIN(first_seen_ms, excluded.first_seen_ms),
            last_seen_ms  = MAX(last_seen_ms,  excluded.last_seen_ms)",
        params![event.kind.as_str(), event.artifact_key, event.ts_ms],
    )?;
    Ok(())
}

/// Every `(kind, artifact_key)` Claudepot has ever observed fire,
/// with its first and last sighting.
///
/// This is the sound answer to "has this artifact ever run?" — see
/// [`bump_first_last`]. Callers computing an "unused" set should
/// subtract this from their installed inventory.
///
/// One query, no per-key round-trip: `usage_for_artifact` issues six
/// statements per artifact, which is fine for a badge but not for a
/// 900-row sweep.
pub fn list_ever_fired(db: &Connection) -> SqlResult<Vec<(ArtifactKind, String, i64, i64)>> {
    let mut stmt = db.prepare(
        "SELECT kind, artifact_key, first_seen_ms, last_seen_ms
           FROM artifact_first_last
          ORDER BY kind, artifact_key",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let kind_s: String = r.get(0)?;
            Ok((
                ArtifactKind::parse(&kind_s),
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<SqlResult<Vec<_>>>()?;
    // Drop rows whose `kind` no longer parses rather than silently
    // remapping them to Skill — a bad remap would make a real skill
    // look "used" because some future kind shared its key.
    Ok(rows
        .into_iter()
        .filter_map(|(k, key, f, l)| k.map(|k| (k, key, f, l)))
        .collect())
}

fn bump_daily(db: &Connection, event: &UsageEvent) -> SqlResult<()> {
    let day = day_floor_unix_s(event.ts_ms);
    let is_error = matches!(event.outcome, Outcome::Error) as i64;
    let dur_total = event.duration_ms.unwrap_or(0) as i64;
    let dur_count = event.duration_ms.is_some() as i64;
    db.execute(
        "INSERT INTO usage_daily (
            day_unix_s, kind, artifact_key, plugin_id,
            fire_count, error_count, total_duration_ms, duration_count
         ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7)
         ON CONFLICT (day_unix_s, kind, artifact_key) DO UPDATE SET
            fire_count        = fire_count + 1,
            error_count       = error_count + excluded.error_count,
            total_duration_ms = total_duration_ms + excluded.total_duration_ms,
            duration_count    = duration_count + excluded.duration_count,
            plugin_id         = COALESCE(plugin_id, excluded.plugin_id)",
        params![
            day,
            event.kind.as_str(),
            event.artifact_key,
            event.plugin_id,
            is_error,
            dur_total,
            dur_count,
        ],
    )?;
    Ok(())
}

/// Drop every event row tied to `file_path`. Called before a re-scan
/// so the next set of inserts is the only contribution from that
/// transcript. Callers that want the daily rollup to track raw events
/// must call `subtract_daily_for_file` first — otherwise the next
/// `insert_event` pass double-counts on top of the daily counters
/// that already reflected the soon-to-be-deleted raw events.
pub fn delete_events_for_file(db: &Connection, file_path: &str) -> SqlResult<usize> {
    db.execute(
        "DELETE FROM usage_event WHERE file_path = ?1",
        params![file_path],
    )
}

/// Subtract the per-day aggregate of `file_path`'s raw events from
/// `usage_daily`. Called BEFORE `delete_events_for_file` + re-insert
/// so the daily counters accurately reflect the new event set.
///
/// Implementation: insert a "negative" daily row per (day, kind, key)
/// and rely on the existing ON CONFLICT clause to subtract via
/// addition with negative numbers. SQLite handles this safely; rows
/// are allowed to go negative briefly during a re-scan but the
/// matching positive inserts that follow restore them.
///
/// Day-floor SQL must match `schema::day_floor_unix_s`:
/// `(ts_ms / 86_400_000) * 86_400` — equivalent for ts_ms ≥ 0, which
/// every realistic session timestamp is.
pub fn subtract_daily_for_file(db: &Connection, file_path: &str) -> SqlResult<()> {
    db.execute(
        "INSERT INTO usage_daily (
            day_unix_s, kind, artifact_key, plugin_id,
            fire_count, error_count, total_duration_ms, duration_count
         )
         SELECT
            (ts_ms / 86400000) * 86400,
            kind, artifact_key, plugin_id,
            -COUNT(*),
            -SUM(CASE WHEN outcome = 'error' THEN 1 ELSE 0 END),
            -COALESCE(SUM(duration_ms), 0),
            -SUM(CASE WHEN duration_ms IS NOT NULL THEN 1 ELSE 0 END)
         FROM usage_event
         WHERE file_path = ?1
         GROUP BY (ts_ms / 86400000) * 86400, kind, artifact_key, plugin_id
         ON CONFLICT (day_unix_s, kind, artifact_key) DO UPDATE SET
            fire_count        = fire_count + excluded.fire_count,
            error_count       = error_count + excluded.error_count,
            total_duration_ms = total_duration_ms + excluded.total_duration_ms,
            duration_count    = duration_count + excluded.duration_count",
        params![file_path],
    )?;
    Ok(())
}

/// Garbage-collect raw events older than `cutoff_ms`. Returns the
/// number of rows deleted. Daily rollups are untouched.
pub fn gc_events_older_than(db: &Connection, cutoff_ms: i64) -> SqlResult<usize> {
    db.execute(
        "DELETE FROM usage_event WHERE ts_ms < ?1",
        params![cutoff_ms],
    )
}

/// Truncate both tables. Only used by `rebuild()`.
/// Clear the rebuildable usage tables.
///
/// **`artifact_first_last` is deliberately NOT cleared.** It is the
/// durable "ever observed" ledger; wiping it on rebuild would
/// reintroduce exactly the failure it exists to prevent (an artifact
/// that fired only in since-deleted transcripts reading as never-run).
/// It is monotonic, so a rebuild's re-scan can only re-confirm rows,
/// never inflate them.
pub fn truncate_all(db: &Connection) -> SqlResult<()> {
    db.execute_batch(
        "DELETE FROM usage_event;
         DELETE FROM usage_daily;",
    )
}

// ---------- query helpers ----------------------------------------------

/// Sum `fire_count` and `error_count` from `usage_daily` for the
/// requested artifact key over the last `days` days. `now_ms` is the
/// current wall-clock time; tests pass a fixed value.
///
/// Window semantics: the cutoff floors to UTC midnight, so `days=7`
/// is "the last 7 UTC calendar days" (anywhere from 7×24 to 8×24
/// hours depending on time-of-day). Acceptable for 7d/30d framing.
/// **Do not call this with days=1** — see `count_24h_from_raw` for
/// the precise rolling-24h window.
pub fn count_for_window(
    db: &Connection,
    kind: ArtifactKind,
    artifact_key: &str,
    now_ms: i64,
    days: i64,
) -> SqlResult<(u64, u64)> {
    let cutoff_day = day_floor_unix_s(now_ms - days * 86_400_000);
    let mut stmt = db.prepare(
        "SELECT COALESCE(SUM(fire_count), 0), COALESCE(SUM(error_count), 0)
         FROM usage_daily
         WHERE kind = ?1 AND artifact_key = ?2 AND day_unix_s >= ?3",
    )?;
    let row: (i64, i64) = stmt
        .query_row(params![kind.as_str(), artifact_key, cutoff_day], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    Ok((row.0.max(0) as u64, row.1.max(0) as u64))
}

/// Precise rolling-24h fire and error counts, drawn from raw events.
/// Uses the daily rollup would round 24h up to "the last 1-2 UTC
/// days," which is a meaningful UI lie at midday. Raw events are
/// kept on a 30-day window so this query is bounded.
pub fn count_24h_from_raw(
    db: &Connection,
    kind: ArtifactKind,
    artifact_key: &str,
    now_ms: i64,
) -> SqlResult<(u64, u64)> {
    let cutoff = now_ms - 86_400_000;
    let row: (i64, i64) = db.query_row(
        "SELECT
            COUNT(*),
            SUM(CASE WHEN outcome = 'error' THEN 1 ELSE 0 END)
         FROM usage_event
         WHERE kind = ?1 AND artifact_key = ?2 AND ts_ms >= ?3",
        params![kind.as_str(), artifact_key, cutoff],
        |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
    )?;
    Ok((row.0.max(0) as u64, row.1.max(0) as u64))
}

/// Last-seen ms for an artifact, drawn from raw events (last 30 days).
/// Returns None when the artifact has no recent events.
pub fn last_seen_ms(
    db: &Connection,
    kind: ArtifactKind,
    artifact_key: &str,
) -> SqlResult<Option<i64>> {
    db.query_row(
        "SELECT MAX(ts_ms) FROM usage_event WHERE kind = ?1 AND artifact_key = ?2",
        params![kind.as_str(), artifact_key],
        |r| r.get::<_, Option<i64>>(0),
    )
}

/// 30-day average duration in ms (NULL if no rows have a duration).
pub fn avg_duration_ms_30d(
    db: &Connection,
    kind: ArtifactKind,
    artifact_key: &str,
    now_ms: i64,
) -> SqlResult<Option<u64>> {
    let cutoff_day = day_floor_unix_s(now_ms - 30 * 86_400_000);
    db.query_row(
        "SELECT
            CASE WHEN SUM(duration_count) > 0
                 THEN SUM(total_duration_ms) / SUM(duration_count)
                 ELSE NULL END
         FROM usage_daily
         WHERE kind = ?1 AND artifact_key = ?2 AND day_unix_s >= ?3",
        params![kind.as_str(), artifact_key, cutoff_day],
        |r| r.get::<_, Option<i64>>(0),
    )
    .map(|opt| opt.map(|v| v.max(0) as u64))
}

/// p50 over the last 24h, computed from raw events. SQLite has no
/// native percentile so we sort the matching durations and pick the
/// middle. Returns None when fewer than one row has a duration.
pub fn p50_ms_24h(
    db: &Connection,
    kind: ArtifactKind,
    artifact_key: &str,
    now_ms: i64,
) -> SqlResult<Option<u64>> {
    let cutoff = now_ms - 86_400_000;
    let mut stmt = db.prepare(
        "SELECT duration_ms FROM usage_event
         WHERE kind = ?1 AND artifact_key = ?2 AND ts_ms >= ?3
            AND duration_ms IS NOT NULL
         ORDER BY duration_ms",
    )?;
    let mut durs: Vec<i64> = stmt
        .query_map(params![kind.as_str(), artifact_key, cutoff], |r| {
            r.get::<_, i64>(0)
        })?
        .collect::<SqlResult<Vec<_>>>()?;
    if durs.is_empty() {
        return Ok(None);
    }
    let mid = durs.len() / 2;
    // Median uses lower mid for even counts — p50 is conventionally
    // either, and the lower keeps the math integer-friendly.
    durs.sort_unstable();
    Ok(Some(durs[mid].max(0) as u64))
}

/// All rows in `usage_daily` collapsed by (kind, artifact_key) with
/// 30-day totals and last_seen joined from raw events. Used for the
/// "Usage" subview's table render.
pub fn list_all(db: &Connection, now_ms: i64) -> SqlResult<Vec<UsageListItem>> {
    let cutoff_day = day_floor_unix_s(now_ms - 30 * 86_400_000);
    let mut stmt = db.prepare(
        "SELECT kind, artifact_key, plugin_id,
                SUM(fire_count), SUM(error_count),
                SUM(total_duration_ms), SUM(duration_count)
         FROM usage_daily
         WHERE day_unix_s >= ?1
         GROUP BY kind, artifact_key, plugin_id
         ORDER BY SUM(fire_count) DESC",
    )?;
    let rows: Vec<UsageListItem> = stmt
        .query_map(params![cutoff_day], |r| {
            let kind_s: String = r.get(0)?;
            let kind = ArtifactKind::parse(&kind_s).unwrap_or(ArtifactKind::Skill);
            let dur_count: i64 = r.get(6)?;
            let dur_total: i64 = r.get(5)?;
            let avg_ms: Option<u64> = if dur_count > 0 {
                Some((dur_total / dur_count).max(0) as u64)
            } else {
                None
            };
            Ok(UsageListItem {
                kind,
                artifact_key: r.get(1)?,
                plugin_id: r.get(2)?,
                fire_count_30d: r.get::<_, i64>(3)?.max(0) as u64,
                error_count_30d: r.get::<_, i64>(4)?.max(0) as u64,
                avg_ms_30d: avg_ms,
            })
        })?
        .collect::<SqlResult<Vec<_>>>()?;
    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct UsageListItem {
    pub kind: ArtifactKind,
    pub artifact_key: String,
    pub plugin_id: Option<String>,
    pub fire_count_30d: u64,
    pub error_count_30d: u64,
    pub avg_ms_30d: Option<u64>,
}
