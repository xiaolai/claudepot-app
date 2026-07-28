//! What has already been distilled, and at what cost.
//!
//! # The bug this replaces
//!
//! Selection used to be "sessions with no memory carrying my
//! `origin_file_path`". That treats *provenance* as a *job ledger*, and
//! the two disagree precisely where it costs money: a transcript the
//! distiller read correctly and which honestly contained no durable
//! lesson produces no memory row, so it looks unmined forever. Every
//! harvest re-selected it, re-spawned `claude -p`, and re-paid for the
//! same verdict.
//!
//! On the reference machine that was 464 sessions. After the corpus
//! index (C1) it is 14,059 transcripts, so the bug scales with the fix
//! — which is why the plan sequences this *before* C1.
//!
//! # What "already processed" means here
//!
//! A transcript is processed when this ledger holds a **succeeded** row
//! for it under the **current extractor version**, whose `(size,
//! mtime_ns)` still match what `sessions` reports. Any of those three
//! failing re-opens it:
//!
//! - no row → never attempted;
//! - different size/mtime → the transcript changed, so the old verdict
//!   is about a different file;
//! - different extractor version → a changed prompt or output schema is
//!   a different extractor, and its verdict is not interchangeable;
//! - `failed` under [`MAX_ATTEMPTS`] → a transient error deserves a
//!   retry, but a permanently-unparseable transcript must not be an
//!   infinite money sink.
//!
//! Zero claims is a **recorded success**, not an absence. That single
//! distinction is the fix.

use crate::session_index::SessionIndex;
use crate::shared_memory::durable::DurableError;
use rusqlite::params;

/// Identifies the extractor whose verdict a ledger row records. Bump
/// when the distiller's prompt, output schema, or model changes in a
/// way that could change its answer — that re-harvests the corpus on
/// purpose.
///
/// Kept here rather than derived from the prompt text: a hash of the
/// prompt would churn on typo fixes, and re-harvesting a 14,059-file
/// corpus is not a thing to trigger by accident.
pub const EXTRACTOR_VERSION: &str = "distiller-v1";

/// How many times a *failing* transcript is retried under one extractor
/// version before it is left alone. Three covers transient spawn / API
/// failures without letting one malformed file bill indefinitely.
pub const MAX_ATTEMPTS: i64 = 3;

/// Outcome of one distillation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The extractor ran and returned a verdict — including "no claims".
    Succeeded,
    /// The extractor could not produce a verdict.
    Failed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Succeeded => "succeeded",
            Outcome::Failed => "failed",
        }
    }
}

/// One attempt, as recorded.
#[derive(Debug, Clone)]
pub struct Attempt<'a> {
    pub file_path: &'a str,
    pub outcome: Outcome,
    pub claims_produced: u32,
    pub model: Option<&'a str>,
    pub cost_usd: Option<f64>,
    pub error: Option<&'a str>,
    pub attempted_at_ms: i64,
}

/// Record an attempt, carrying the freshness guard over from `sessions`.
///
/// `attempts` accumulates across retries of the same (file, extractor)
/// so [`MAX_ATTEMPTS`] can bound them, and **resets to 1 when the
/// freshness guard changes**. Without the reset a transcript that
/// failed twice, was then edited, would get one retry instead of three
/// — inheriting a retry budget spent on a different revision of the
/// file. A success resets nothing, because a succeeded row is terminal
/// until the file or extractor changes.
///
/// A transcript with no `sessions` row (e.g. deleted between selection
/// and recording) stores a zero guard, which simply makes the next
/// harvest re-open it — the safe direction.
pub fn record(idx: &SessionIndex, a: &Attempt<'_>) -> Result<(), DurableError> {
    let db = idx.db();
    let (size, mtime): (i64, i64) = db
        .query_row(
            "SELECT file_size_bytes, file_mtime_ns FROM sessions WHERE file_path = ?1",
            [a.file_path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));

    db.execute(
        "INSERT INTO harvest_ledger \
           (file_path, extractor_version, file_size_bytes, file_mtime_ns, \
            attempted_at_ms, outcome, attempts, claims_produced, model, cost_usd, error) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?10) \
         ON CONFLICT(file_path, extractor_version) DO UPDATE SET \
            file_size_bytes = excluded.file_size_bytes, \
            file_mtime_ns   = excluded.file_mtime_ns, \
            attempted_at_ms = excluded.attempted_at_ms, \
            outcome         = excluded.outcome, \
            attempts        = CASE \
                WHEN harvest_ledger.file_size_bytes != excluded.file_size_bytes \
                  OR harvest_ledger.file_mtime_ns   != excluded.file_mtime_ns \
                THEN 1 ELSE harvest_ledger.attempts + 1 END, \
            claims_produced = excluded.claims_produced, \
            model           = excluded.model, \
            cost_usd        = excluded.cost_usd, \
            error           = excluded.error",
        params![
            a.file_path,
            EXTRACTOR_VERSION,
            size,
            mtime,
            a.attempted_at_ms,
            a.outcome.as_str(),
            a.claims_produced as i64,
            a.model,
            a.cost_usd,
            a.error,
        ],
    )?;
    Ok(())
}

/// Seed the ledger from the scheme it replaces.
///
/// A memory carrying `origin_file_path` is proof that transcript was
/// distilled before this table existed. Without seeding those rows, the
/// first run after an upgrade sees every previously harvested
/// transcript as unharvested and pays for it again — the exact bug this
/// table was added to fix, inverted, and billed to the user.
///
/// **Must run after `schema::apply_memories_compiler_columns`**, not
/// inside the v4 DDL block: `origin_file_path` is one of the columns
/// that block does *not* create (it arrives by probe + ALTER
/// afterwards), so a backfill embedded in the DDL fails with
/// `no such column`.
///
/// Idempotent — `INSERT OR IGNORE` against the `(file_path,
/// extractor_version)` primary key, so re-running on every open is a
/// no-op once seeded.
///
/// Inner-joins `sessions` deliberately: a memory whose transcript is no
/// longer indexed has no reconstructable freshness guard, and a
/// fabricated one would either wrongly suppress a harvest or be ignored
/// on the next join anyway. Those re-open, which is the safe direction.
pub fn backfill_from_provenance(db: &rusqlite::Connection) -> rusqlite::Result<usize> {
    db.execute(
        "INSERT OR IGNORE INTO harvest_ledger \
           (file_path, extractor_version, file_size_bytes, file_mtime_ns, \
            attempted_at_ms, outcome, attempts, claims_produced) \
         SELECT DISTINCT m.origin_file_path, ?1, \
                s.file_size_bytes, s.file_mtime_ns, \
                COALESCE(m.created_at_ms, 0), 'succeeded', 1, 0 \
           FROM memories m \
           JOIN sessions s ON s.file_path = m.origin_file_path \
          WHERE m.origin_file_path IS NOT NULL",
        params![EXTRACTOR_VERSION],
    )
}

/// Sessions in `project_path` still needing a harvest under the current
/// extractor.
///
/// This is the query the old `origin_file_path NOT IN (...)` scheme got
/// wrong. The `LEFT JOIN` is against the *current* extractor version
/// only, so bumping [`EXTRACTOR_VERSION`] re-opens the corpus without
/// deleting history.
pub fn unharvested_sessions(
    idx: &SessionIndex,
    project_path: &str,
    limit: u32,
) -> Result<Vec<String>, DurableError> {
    let limit = if limit == 0 { 20 } else { limit.min(500) };
    let db = idx.db();
    let mut stmt = db.prepare(
        "SELECT s.file_path FROM sessions s \
         LEFT JOIN harvest_ledger h \
                ON h.file_path = s.file_path \
               AND h.extractor_version = ?1 \
          WHERE s.project_path = ?2 \
            AND ( h.file_path IS NULL \
               OR h.file_size_bytes != s.file_size_bytes \
               OR h.file_mtime_ns   != s.file_mtime_ns \
               OR (h.outcome = 'failed' AND h.attempts < ?3) ) \
          ORDER BY s.last_ts_ms DESC NULLS LAST \
          LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![EXTRACTOR_VERSION, project_path, MAX_ATTEMPTS, limit as i64],
        |r| r.get::<_, String>(0),
    )?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// What a harvest has cost so far under the current extractor. Powers
/// the "estimated cost" line without re-deriving it from session counts.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LedgerStats {
    pub succeeded: u32,
    pub failed: u32,
    pub claims_produced: u32,
    pub cost_usd: f64,
}

pub fn stats(idx: &SessionIndex) -> Result<LedgerStats, DurableError> {
    let db = idx.db();
    let (succeeded, failed, claims, cost): (i64, i64, i64, Option<f64>) = db.query_row(
        "SELECT \
           COALESCE(SUM(outcome = 'succeeded'), 0), \
           COALESCE(SUM(outcome = 'failed'), 0), \
           COALESCE(SUM(claims_produced), 0), \
           SUM(cost_usd) \
         FROM harvest_ledger WHERE extractor_version = ?1",
        [EXTRACTOR_VERSION],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(LedgerStats {
        succeeded: succeeded.max(0) as u32,
        failed: failed.max(0) as u32,
        claims_produced: claims.max(0) as u32,
        cost_usd: cost.unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_index::SessionIndex;
    use tempfile::TempDir;

    fn idx() -> (SessionIndex, TempDir) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        (idx, tmp)
    }

    /// Insert a minimal `sessions` row — only the columns this module
    /// joins on need to be meaningful.
    fn seed_session(idx: &SessionIndex, path: &str, project: &str, size: i64, mtime: i64) {
        idx.db()
            .execute(
                "INSERT OR REPLACE INTO sessions (\
                   file_path, slug, session_id, file_size_bytes, file_mtime_ns, file_inode, \
                   project_path, project_from_transcript, event_count, message_count, \
                   user_message_count, assistant_message_count, models_json, \
                   tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read, \
                   has_error, is_sidechain, indexed_at_ms, last_ts_ms) \
                 VALUES (?1,'slug','sid',?2,?3,0,?4,0,0,0,0,0,'[]',0,0,0,0,0,0,0,1)",
                params![path, size, mtime, project],
            )
            .unwrap();
    }

    fn attempt<'a>(path: &'a str, outcome: Outcome, claims: u32) -> Attempt<'a> {
        Attempt {
            file_path: path,
            outcome,
            claims_produced: claims,
            model: Some("claude-haiku-4-5"),
            cost_usd: Some(0.01),
            error: None,
            attempted_at_ms: 1_800_000_000_000,
        }
    }

    #[test]
    fn an_unattempted_session_is_selected() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"]
        );
    }

    /// The bug. A correct harvest that found nothing must not be
    /// re-selected — under the old scheme it was, forever.
    #[test]
    fn a_success_with_zero_claims_is_not_reselected() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 0)).unwrap();
        assert!(unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty());
    }

    #[test]
    fn a_success_with_claims_is_not_reselected_either() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 4)).unwrap();
        assert!(unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty());
    }

    #[test]
    fn a_changed_transcript_is_reselected() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 1)).unwrap();
        // Same path, new bytes: the old verdict is about a different file.
        seed_session(&idx, "/a.jsonl", "/proj", 99, 20);
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"]
        );

        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 1)).unwrap();
        seed_session(&idx, "/a.jsonl", "/proj", 99, 21); // mtime moved
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"]
        );
    }

    #[test]
    fn a_failure_retries_then_stops_at_the_cap() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        for i in 1..MAX_ATTEMPTS {
            record(&idx, &attempt("/a.jsonl", Outcome::Failed, 0)).unwrap();
            assert_eq!(
                unharvested_sessions(&idx, "/proj", 10).unwrap(),
                ["/a.jsonl"],
                "attempt {i} should still be retryable"
            );
        }
        record(&idx, &attempt("/a.jsonl", Outcome::Failed, 0)).unwrap();
        assert!(
            unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty(),
            "a permanently-failing transcript must stop billing"
        );
    }

    /// A retry budget is per *revision*. A file that burned its
    /// attempts, then changed, must get a fresh three — otherwise an
    /// edited transcript inherits a budget spent on different content.
    #[test]
    fn changing_the_file_resets_the_retry_budget() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        for _ in 0..MAX_ATTEMPTS {
            record(&idx, &attempt("/a.jsonl", Outcome::Failed, 0)).unwrap();
        }
        assert!(
            unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty(),
            "precondition: budget exhausted"
        );

        seed_session(&idx, "/a.jsonl", "/proj", 99, 21); // edited
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"]
        );
        record(&idx, &attempt("/a.jsonl", Outcome::Failed, 0)).unwrap();
        let n: i64 = idx
            .db()
            .query_row(
                "SELECT attempts FROM harvest_ledger WHERE file_path='/a.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the counter must restart for a new revision");
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"],
            "and it must still be retryable"
        );
    }

    /// A later success supersedes earlier failures.
    #[test]
    fn a_success_after_failures_closes_the_transcript() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Failed, 0)).unwrap();
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 0)).unwrap();
        assert!(unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty());
    }

    #[test]
    fn bumping_the_extractor_version_reopens_the_corpus() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 0)).unwrap();
        assert!(unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty());

        // Simulate a version bump by writing a row under a different one.
        idx.db()
            .execute(
                "UPDATE harvest_ledger SET extractor_version = 'distiller-v0'",
                [],
            )
            .unwrap();
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"],
            "a verdict from another extractor is not interchangeable"
        );
    }

    #[test]
    fn selection_is_scoped_to_the_project() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj-a", 10, 20);
        seed_session(&idx, "/b.jsonl", "/proj-b", 10, 20);
        assert_eq!(
            unharvested_sessions(&idx, "/proj-a", 10).unwrap(),
            ["/a.jsonl"]
        );
    }

    #[test]
    fn stats_sum_the_current_extractors_rows() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        seed_session(&idx, "/b.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 3)).unwrap();
        record(&idx, &attempt("/b.jsonl", Outcome::Failed, 0)).unwrap();
        let s = stats(&idx).unwrap();
        assert_eq!(s.succeeded, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.claims_produced, 3);
        assert!((s.cost_usd - 0.02).abs() < 1e-9);
    }

    /// Upgrading must not re-open transcripts a previous harvest
    /// already paid for. Provenance from the old scheme
    /// (`memories.origin_file_path`) is the only record of that work.
    #[test]
    fn the_backfill_closes_previously_harvested_transcripts() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        // A memory from the old scheme, with no ledger row.
        idx.db()
            .execute(
                "INSERT INTO memories (id, scope, kind, content, created_by_kind, created_by, \
                   created_at_ms, updated_at_ms, review_state, origin_file_path, project_path) \
                 VALUES ('m1','project','pattern','x','agent','t',5,5,'accepted','/a.jsonl','/proj')",
                [],
            )
            .unwrap();
        assert_eq!(
            unharvested_sessions(&idx, "/proj", 10).unwrap(),
            ["/a.jsonl"],
            "precondition: without a ledger row it is selected"
        );

        let n = backfill_from_provenance(&idx.db()).unwrap();
        assert_eq!(n, 1);
        assert!(
            unharvested_sessions(&idx, "/proj", 10).unwrap().is_empty(),
            "backfilled provenance must close the transcript"
        );
    }

    /// Runs on every index open, so it must be a no-op once seeded.
    #[test]
    fn the_backfill_is_idempotent() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        idx.db()
            .execute(
                "INSERT INTO memories (id, scope, kind, content, created_by_kind, created_by, \
                   created_at_ms, updated_at_ms, review_state, origin_file_path, project_path) \
                 VALUES ('m1','project','pattern','x','agent','t',5,5,'accepted','/a.jsonl','/proj')",
                [],
            )
            .unwrap();
        assert_eq!(backfill_from_provenance(&idx.db()).unwrap(), 1);
        assert_eq!(
            backfill_from_provenance(&idx.db()).unwrap(),
            0,
            "second run must insert nothing"
        );
    }

    /// A memory whose transcript is no longer indexed has no
    /// reconstructable freshness guard, so it is left to re-open rather
    /// than closed on a fabricated one.
    #[test]
    fn the_backfill_skips_memories_whose_transcript_is_gone() {
        let (idx, _t) = idx();
        idx.db()
            .execute(
                "INSERT INTO memories (id, scope, kind, content, created_by_kind, created_by, \
                   created_at_ms, updated_at_ms, review_state, origin_file_path, project_path) \
                 VALUES ('m1','project','pattern','x','agent','t',5,5,'accepted','/gone.jsonl','/proj')",
                [],
            )
            .unwrap();
        assert_eq!(backfill_from_provenance(&idx.db()).unwrap(), 0);
    }

    #[test]
    fn stats_on_an_empty_ledger_are_zero_not_an_error() {
        let (idx, _t) = idx();
        assert_eq!(stats(&idx).unwrap(), LedgerStats::default());
    }

    /// The ledger is durable: a rebuild wipes the transcript cache but
    /// must not make the next harvest re-pay for completed work.
    #[test]
    fn the_ledger_survives_a_rebuild() {
        let (idx, _t) = idx();
        seed_session(&idx, "/a.jsonl", "/proj", 10, 20);
        record(&idx, &attempt("/a.jsonl", Outcome::Succeeded, 2)).unwrap();
        idx.rebuild().unwrap();
        assert_eq!(
            stats(&idx).unwrap().succeeded,
            1,
            "ledger must outlive a rebuild"
        );
    }
}
