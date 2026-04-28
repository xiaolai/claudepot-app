//! Artifact usage tracking — counts and outcomes for invocations of
//! installed CC artifacts (skills, hooks, agents, slash commands).
//!
//! See `dev-docs/usage-tracking-plan.md` for the full design. The
//! short version:
//!
//! - `extract.rs` — pure JSONL → `UsageEvent` extractor (sibling to
//!   `activity::classifier`, but for successes).
//! - `store.rs`   — SQL persistence for `usage_event` + `usage_daily`.
//! - `schema.rs`  — DDL fragments and the day-floor helper.
//! - `mod.rs`     — public API consumed by Tauri commands and the CLI.
//!
//! Storage lives in the same `sessions.db` that backs the Sessions
//! tab — same files are scanned, same refresh cycle owns the writes.

pub mod extract;
mod extract_helpers;
pub mod model;
pub mod schema;
pub mod store;

pub use extract_helpers::hook_artifact_key;

pub use model::{ArtifactKind, Outcome, UsageEvent, UsageListRow, UsageStats};

use rusqlite::{Connection, Result as SqlResult};

/// Roll up the three time windows + last-seen + percentiles into one
/// `UsageStats` for a single artifact.
///
/// Returns an empty `UsageStats` when the artifact has no recorded
/// events — the caller can use `UsageStats::is_empty()` to render the
/// "never used" state.
pub fn usage_for_artifact(
    db: &Connection,
    kind: ArtifactKind,
    artifact_key: &str,
    now_ms: i64,
) -> SqlResult<UsageStats> {
    // 24h must come from raw events — the daily rollup floors to UTC
    // midnight and would inflate the window to up to 48h.
    let (fire_24h, _err_24h) = store::count_24h_from_raw(db, kind, artifact_key, now_ms)?;
    let (fire_7d, _err_7d) = store::count_for_window(db, kind, artifact_key, now_ms, 7)?;
    let (fire_30d, err_30d) = store::count_for_window(db, kind, artifact_key, now_ms, 30)?;
    let last = store::last_seen_ms(db, kind, artifact_key)?;
    let p50 = store::p50_ms_24h(db, kind, artifact_key, now_ms)?;
    let avg = store::avg_duration_ms_30d(db, kind, artifact_key, now_ms)?;
    Ok(UsageStats {
        count_24h: fire_24h,
        count_7d: fire_7d,
        count_30d: fire_30d,
        error_count_30d: err_30d,
        last_seen_ms: last,
        p50_ms_24h: p50,
        avg_ms_30d: avg,
    })
}

/// Batch fetch — used by the Config-tree renderer to populate badges
/// for every visible artifact in one round-trip. Returns one entry
/// per requested key (empty `UsageStats` when no events exist).
pub fn batch_usage(
    db: &Connection,
    keys: &[(ArtifactKind, String)],
    now_ms: i64,
) -> SqlResult<Vec<((ArtifactKind, String), UsageStats)>> {
    keys.iter()
        .map(|(k, key)| {
            let stats = usage_for_artifact(db, *k, key, now_ms)?;
            Ok(((*k, key.clone()), stats))
        })
        .collect()
}

/// Top N most-fired artifacts in the last 30 days, optionally
/// filtered by kind. Used by the Activity Usage subview's default
/// sort.
pub fn list_top_used(
    db: &Connection,
    kind: Option<ArtifactKind>,
    limit: usize,
    now_ms: i64,
) -> SqlResult<Vec<UsageListRow>> {
    let mut all = store::list_all(db, now_ms)?;
    if let Some(k) = kind {
        all.retain(|r| r.kind == k);
    }
    all.sort_by(|a, b| b.fire_count_30d.cmp(&a.fire_count_30d));
    all.truncate(limit);
    let now_ms_local = now_ms;
    let rows: SqlResult<Vec<UsageListRow>> = all
        .into_iter()
        .map(|item| {
            let stats = usage_for_artifact(db, item.kind, &item.artifact_key, now_ms_local)?;
            Ok(UsageListRow {
                kind: item.kind,
                artifact_key: item.artifact_key,
                plugin_id: item.plugin_id,
                stats,
            })
        })
        .collect();
    rows
}

/// Discover every (kind, key) pair the index has *ever* recorded
/// usage for. Used by the "Unused" filter — the caller intersects
/// this with the set of installed artifacts (from `config_view`)
/// to find the difference.
///
/// This deliberately ignores the rolling window — an artifact that
/// fired once a year ago and was then forgotten still shows up here.
/// Callers that want "unused in last N days" should compare against
/// `last_seen_ms` from `usage_for_artifact`.
pub fn list_all_known(db: &Connection) -> SqlResult<Vec<(ArtifactKind, String)>> {
    let mut stmt = db.prepare(
        "SELECT DISTINCT kind, artifact_key FROM usage_daily ORDER BY kind, artifact_key",
    )?;
    let rows: Vec<(ArtifactKind, String)> = stmt
        .query_map([], |r| {
            let kind_s: String = r.get(0)?;
            let key: String = r.get(1)?;
            Ok((
                ArtifactKind::parse(&kind_s).unwrap_or(ArtifactKind::Skill),
                key,
            ))
        })?
        .collect::<SqlResult<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_in_memory() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::artifact_usage::schema::SCHEMA)
            .unwrap();
        db
    }

    fn ev(
        kind: ArtifactKind,
        key: &str,
        ts_ms: i64,
        outcome: Outcome,
        dur: Option<u64>,
    ) -> UsageEvent {
        UsageEvent {
            ts_ms,
            session_id: "S1".into(),
            kind,
            artifact_key: key.into(),
            plugin_id: None,
            outcome,
            duration_ms: dur,
            extra_json: None,
        }
    }

    #[test]
    fn empty_db_returns_empty_stats() {
        let db = open_in_memory();
        let stats = usage_for_artifact(&db, ArtifactKind::Skill, "plugin:p:a", 1_000_000).unwrap();
        assert!(stats.is_empty());
        assert_eq!(stats.count_30d, 0);
        assert!(stats.last_seen_ms.is_none());
    }

    #[test]
    fn three_events_roll_up_into_correct_30d_count() {
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64; // arbitrary
        for i in 0..3 {
            let e = ev(
                ArtifactKind::Hook,
                "node /h.js",
                now - (i * 3600 * 1000),
                Outcome::Ok,
                Some(50 + i as u64),
            );
            store::insert_event(&db, &e, "/sess.jsonl", "/proj").unwrap();
        }
        let stats = usage_for_artifact(&db, ArtifactKind::Hook, "node /h.js", now).unwrap();
        assert_eq!(stats.count_30d, 3);
        assert_eq!(stats.count_24h, 3);
        assert_eq!(stats.error_count_30d, 0);
        assert_eq!(stats.last_seen_ms, Some(now));
        // p50 of [50, 51, 52] = 51 (lower mid would be index 1)
        assert_eq!(stats.p50_ms_24h, Some(51));
        assert_eq!(stats.avg_ms_30d, Some(51));
    }

    #[test]
    fn error_outcomes_count_in_error_total_only() {
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        store::insert_event(
            &db,
            &ev(ArtifactKind::Hook, "h", now, Outcome::Ok, Some(10)),
            "/s",
            "/p",
        )
        .unwrap();
        store::insert_event(
            &db,
            &ev(
                ArtifactKind::Hook,
                "h",
                now - 1000,
                Outcome::Error,
                Some(20),
            ),
            "/s",
            "/p",
        )
        .unwrap();
        let stats = usage_for_artifact(&db, ArtifactKind::Hook, "h", now).unwrap();
        assert_eq!(stats.count_30d, 2);
        assert_eq!(stats.error_count_30d, 1);
    }

    #[test]
    fn events_outside_window_excluded_from_24h() {
        let db = open_in_memory();
        let now = 100 * 86_400_000_i64; // day 100
        store::insert_event(
            &db,
            &ev(
                ArtifactKind::Skill,
                "k",
                now - 25 * 3600 * 1000,
                Outcome::Ok,
                None,
            ),
            "/s",
            "/p",
        )
        .unwrap();
        store::insert_event(
            &db,
            &ev(ArtifactKind::Skill, "k", now, Outcome::Ok, None),
            "/s",
            "/p",
        )
        .unwrap();
        let stats = usage_for_artifact(&db, ArtifactKind::Skill, "k", now).unwrap();
        assert_eq!(stats.count_24h, 1, "only the recent event counts in 24h");
        assert_eq!(stats.count_30d, 2, "both count in 30d");
    }

    #[test]
    fn count_24h_uses_rolling_window_not_utc_day_floor() {
        // Regression for the audit Medium finding: the daily rollup
        // floors to UTC midnight, so a query at 2:00 UTC would have
        // counted events from "yesterday 0:00 UTC" — almost 26h back.
        // The fix moves 24h to read raw events with a precise rolling
        // window. This test fires an event 25h ago at "now=02:00 UTC"
        // and proves it does NOT count toward 24h.
        let db = open_in_memory();
        // Pick a `now` that's 02:00 UTC on day 100.
        let now = 100 * 86_400_000_i64 + 2 * 3_600_000;
        // Event 25 hours ago — outside the rolling 24h window but
        // inside the previous UTC day, so the bug would have included
        // it.
        store::insert_event(
            &db,
            &ev(
                ArtifactKind::Skill,
                "k",
                now - 25 * 3_600_000,
                Outcome::Ok,
                None,
            ),
            "/s",
            "/p",
        )
        .unwrap();
        let stats = usage_for_artifact(&db, ArtifactKind::Skill, "k", now).unwrap();
        assert_eq!(
            stats.count_24h, 0,
            "event 25h ago must NOT count toward 24h regardless of UTC day boundary"
        );
        assert_eq!(stats.count_30d, 1, "but does count toward 30d");
    }

    #[test]
    fn delete_events_for_file_drops_raw_but_keeps_daily() {
        // Without the matching `subtract_daily_for_file` call the daily
        // rollup is intentionally left intact — that's the contract.
        // session_index::refresh always pairs the two; this test
        // documents the lower-level primitive.
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        store::insert_event(
            &db,
            &ev(ArtifactKind::Hook, "h", now, Outcome::Ok, Some(10)),
            "/sess.jsonl",
            "/proj",
        )
        .unwrap();
        let n = store::delete_events_for_file(&db, "/sess.jsonl").unwrap();
        assert_eq!(n, 1);
        // Last-seen draws from raw events → now empty
        let stats = usage_for_artifact(&db, ArtifactKind::Hook, "h", now).unwrap();
        assert!(stats.last_seen_ms.is_none());
        // Daily counter survives — that's the documented behavior
        assert_eq!(stats.count_30d, 1);
    }

    #[test]
    fn subtract_daily_then_delete_drops_both_raw_and_daily() {
        // This is the contract refresh() relies on: subtract → delete →
        // re-insert leaves the daily rollup arithmetically consistent.
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        for _ in 0..3 {
            store::insert_event(
                &db,
                &ev(ArtifactKind::Hook, "h", now, Outcome::Ok, Some(10)),
                "/sess.jsonl",
                "/proj",
            )
            .unwrap();
        }
        store::subtract_daily_for_file(&db, "/sess.jsonl").unwrap();
        store::delete_events_for_file(&db, "/sess.jsonl").unwrap();
        let stats = usage_for_artifact(&db, ArtifactKind::Hook, "h", now).unwrap();
        assert_eq!(
            stats.count_30d, 0,
            "subtract+delete must leave the daily rollup at zero"
        );
        assert!(stats.last_seen_ms.is_none());
    }

    #[test]
    fn rescan_pattern_does_not_inflate_daily_counts() {
        // Direct regression test for the High-severity audit finding:
        // re-scanning a file twice must produce the same daily count
        // as a single scan. Earlier the rollup doubled on every
        // re-scan because delete_events_for_file alone left the
        // daily counters intact.
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        let file = "/sess.jsonl";

        // First scan: insert 5 events.
        for _ in 0..5 {
            store::insert_event(
                &db,
                &ev(ArtifactKind::Skill, "k", now, Outcome::Ok, None),
                file,
                "/p",
            )
            .unwrap();
        }
        let after_first = usage_for_artifact(&db, ArtifactKind::Skill, "k", now)
            .unwrap()
            .count_30d;
        assert_eq!(after_first, 5);

        // Second scan of the SAME file with the SAME 5 events: simulate
        // the refresh() path — subtract then delete then re-insert.
        store::subtract_daily_for_file(&db, file).unwrap();
        store::delete_events_for_file(&db, file).unwrap();
        for _ in 0..5 {
            store::insert_event(
                &db,
                &ev(ArtifactKind::Skill, "k", now, Outcome::Ok, None),
                file,
                "/p",
            )
            .unwrap();
        }
        let after_rescan = usage_for_artifact(&db, ArtifactKind::Skill, "k", now)
            .unwrap()
            .count_30d;
        assert_eq!(
            after_rescan, 5,
            "re-scan must not double-count daily; got {after_rescan} (expected 5)"
        );
    }

    #[test]
    fn truncate_all_clears_both_tables() {
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        store::insert_event(
            &db,
            &ev(ArtifactKind::Hook, "h", now, Outcome::Ok, Some(10)),
            "/s",
            "/p",
        )
        .unwrap();
        store::truncate_all(&db).unwrap();
        let stats = usage_for_artifact(&db, ArtifactKind::Hook, "h", now).unwrap();
        assert!(stats.is_empty());
    }

    #[test]
    fn gc_drops_only_old_raw_events() {
        let db = open_in_memory();
        let now = 100 * 86_400_000_i64;
        // Insert one event per day from now back through now-4d.
        for i in 0..5 {
            let e = ev(
                ArtifactKind::Hook,
                "h",
                now - (i * 86_400_000),
                Outcome::Ok,
                Some(10),
            );
            store::insert_event(&db, &e, "/s", "/p").unwrap();
        }
        // Cutoff = strictly less than `now - 3d` should drop only the
        // event at now-4d (now-3d is exactly at the boundary and kept).
        let cutoff = now - 3 * 86_400_000;
        let dropped = store::gc_events_older_than(&db, cutoff).unwrap();
        assert_eq!(dropped, 1, "only the now-4d event is strictly older");
        // Double-check the boundary case: drop everything older than now+1ms
        // wipes all 5.
        let dropped_all = store::gc_events_older_than(&db, now + 1).unwrap();
        assert_eq!(dropped_all, 4, "remaining four events older than now+1ms");
    }

    #[test]
    fn list_top_used_orders_by_30d_count() {
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        for _ in 0..5 {
            store::insert_event(
                &db,
                &ev(ArtifactKind::Skill, "popular", now, Outcome::Ok, None),
                "/s",
                "/p",
            )
            .unwrap();
        }
        store::insert_event(
            &db,
            &ev(ArtifactKind::Skill, "rare", now, Outcome::Ok, None),
            "/s",
            "/p",
        )
        .unwrap();
        let rows = list_top_used(&db, Some(ArtifactKind::Skill), 10, now).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].artifact_key, "popular");
        assert_eq!(rows[0].stats.count_30d, 5);
        assert_eq!(rows[1].artifact_key, "rare");
    }

    #[test]
    fn batch_usage_returns_one_entry_per_input() {
        let db = open_in_memory();
        let now = 1_000_000_000_000_i64;
        store::insert_event(
            &db,
            &ev(ArtifactKind::Skill, "a", now, Outcome::Ok, None),
            "/s",
            "/p",
        )
        .unwrap();
        let result = batch_usage(
            &db,
            &[
                (ArtifactKind::Skill, "a".to_string()),
                (ArtifactKind::Skill, "missing".to_string()),
            ],
            now,
        )
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1.count_30d, 1);
        assert!(result[1].1.is_empty());
    }
}
