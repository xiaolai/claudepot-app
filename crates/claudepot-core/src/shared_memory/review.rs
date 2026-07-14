//! The triage queue: read proposals, accept them, reject them.
//!
//! # The product is the queue, not the store
//!
//! Every "AI knowledge base" ships a place to *put* things. The reason
//! they die is that nobody puts anything there — authoring is work, and
//! the payoff is deferred and invisible. So the user never authors here.
//! The distiller proposes; the user's entire job is a yes or a no.
//!
//! That inverts the usual failure: the cost of *maintaining* the base is
//! pushed down to seconds, and the cost of *generating* it is pushed
//! onto a Haiku call that runs while the user is asleep.
//!
//! # Accepting is what stamps the anchor
//!
//! A proposal is anchored to the *files* it depends on. Acceptance
//! stamps the *commit* those files were at when the human agreed the
//! claim was true. That pair — files + commit — is what makes
//! invalidation possible later: when the anchored code moves on, the
//! claim goes back in this queue as SUSPECT.
//!
//! Nobody else expires memories this way. Copilot expires by *usage*,
//! which measures whether a memory is popular, not whether it is still
//! true. A stale-but-popular memory is the worst thing in the system.

use serde::Serialize;

use crate::session_index::SessionIndex;
use crate::shared_memory::durable::DurableError;

/// Review states a memory row can be in. Mirrors the DDL CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    /// Filed by a harvester. Inert: never compiled, never in context.
    Proposed,
    /// A human agreed. Eligible for compilation into a guard/directive.
    Accepted,
    /// A human disagreed. Kept forever so the harvester cannot re-file
    /// it — a queue that resurrects rejected items is a queue nobody
    /// opens twice.
    Rejected,
    /// Was accepted; the code it was anchored to has since changed.
    /// Back in the queue, with the reason attached.
    Suspect,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::Proposed => "proposed",
            ReviewState::Accepted => "accepted",
            ReviewState::Rejected => "rejected",
            ReviewState::Suspect => "suspect",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "proposed" => Some(ReviewState::Proposed),
            "accepted" => Some(ReviewState::Accepted),
            "rejected" => Some(ReviewState::Rejected),
            "suspect" => Some(ReviewState::Suspect),
            _ => None,
        }
    }
}

/// One row of the triage queue.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReviewRow {
    pub id: String,
    pub review_state: String,
    pub kind: String,
    /// The claim, in the distiller's words.
    pub content: String,
    /// The imperative one-liner a future agent would see.
    pub directive: Option<String>,
    pub confidence: Option<i64>,
    /// `{"files":[…],"evidence":"…","commit":"…"}`
    pub anchor_json: Option<String>,
    pub suspect_reason: Option<String>,
    /// The transcript this was learned from — the "show me what burned
    /// me" link. Denormalized, so it survives an index rebuild.
    pub origin_file_path: Option<String>,
    pub origin_exchange_id: Option<String>,
    pub project_path: Option<String>,
    pub created_at_ms: i64,
}

/// Counts for the dashboard. Deliberately NOT "N memories stored" —
/// that is a vanity metric, and vanity metrics are how these products
/// convince themselves they are working.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReviewCounts {
    pub proposed: i64,
    pub accepted: i64,
    pub rejected: i64,
    pub suspect: i64,
    /// Accepted claims that were compiled into a binding check.
    pub enforced: i64,
}

/// List rows in a given review state, newest first.
pub fn list(
    idx: &SessionIndex,
    project_path: Option<&str>,
    state: Option<ReviewState>,
    limit: u32,
) -> Result<Vec<ReviewRow>, DurableError> {
    let limit = if limit == 0 { 50 } else { limit.min(500) };
    let mut sql = String::from(
        "SELECT id, review_state, kind, content, directive, confidence, anchor_json, \
         suspect_reason, origin_file_path, origin_exchange_id, project_path, created_at_ms \
         FROM memories WHERE archived_at_ms IS NULL",
    );
    let mut binds: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(p) = project_path {
        sql.push_str(&format!(" AND project_path = ?{}", binds.len() + 1));
        binds.push(rusqlite::types::Value::Text(p.to_string()));
    }
    if let Some(s) = state {
        sql.push_str(&format!(" AND review_state = ?{}", binds.len() + 1));
        binds.push(rusqlite::types::Value::Text(s.as_str().to_string()));
    }
    sql.push_str(&format!(
        " ORDER BY created_at_ms DESC LIMIT ?{}",
        binds.len() + 1
    ));
    binds.push(rusqlite::types::Value::Integer(limit as i64));

    let db = idx.db();
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| {
        Ok(ReviewRow {
            id: r.get(0)?,
            review_state: r.get(1)?,
            kind: r.get(2)?,
            content: r.get(3)?,
            directive: r.get(4)?,
            confidence: r.get(5)?,
            anchor_json: r.get(6)?,
            suspect_reason: r.get(7)?,
            origin_file_path: r.get(8)?,
            origin_exchange_id: r.get(9)?,
            project_path: r.get(10)?,
            created_at_ms: r.get(11)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// The `project_path` a memory row belongs to (`None` if the row is
/// global or the id doesn't exist). Lets a caller resolve the lesson's
/// own repo before running git against it.
pub fn memory_project_path(idx: &SessionIndex, id: &str) -> Result<Option<String>, DurableError> {
    match idx.db().query_row(
        "SELECT project_path FROM memories WHERE id = ?1",
        [id],
        |r| r.get::<_, Option<String>>(0),
    ) {
        Ok(pp) => Ok(pp),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(DurableError::from(e)),
    }
}

/// Accept a claim, stamping the commit its anchored files were at.
///
/// `anchor_commit` is what makes the claim *invalidatable*: it records
/// the state of the world the human was agreeing about. When those files
/// move past this commit, the agreement is no longer known to hold, and
/// the claim returns to the queue.
///
/// Passing `None` accepts without an anchor — the claim can never go
/// suspect, which is right for a lesson that isn't about code (a
/// preference, a fact about the user) and wrong for anything else.
pub fn accept(
    idx: &SessionIndex,
    id: &str,
    anchor_commit: Option<&str>,
    now_ms: i64,
) -> Result<bool, DurableError> {
    let anchor = match anchor_commit {
        Some(sha) => merge_commit_into_anchor(idx, id, sha)?,
        None => None,
    };
    let n = match anchor {
        Some(json) => idx.db().execute(
            "UPDATE memories SET review_state = 'accepted', suspect_reason = NULL, \
             anchor_json = ?1, updated_at_ms = ?2 \
             WHERE id = ?3 AND archived_at_ms IS NULL",
            rusqlite::params![json, now_ms, id],
        )?,
        None => idx.db().execute(
            // Clear anchor_json too: accepting with NO anchor means "this
            // can never go suspect". Leaving a stale commit/files behind
            // (e.g. re-accepting a previously-anchored suspect lesson
            // with --no-anchor) would let the next invalidation sweep
            // flip it straight back to suspect — the opposite of what
            // --no-anchor promises.
            "UPDATE memories SET review_state = 'accepted', suspect_reason = NULL, \
             anchor_json = NULL, updated_at_ms = ?1 \
             WHERE id = ?2 AND archived_at_ms IS NULL",
            rusqlite::params![now_ms, id],
        )?,
    };
    Ok(n > 0)
}

/// Reject a claim. The row is KEPT — that is the whole point. The
/// harvester re-derives the same lesson from the same transcript on
/// every run, and `proposal::ingest_proposals` refuses to re-file
/// anything it has seen in any state. Deleting a rejection would make
/// it come back forever.
pub fn reject(idx: &SessionIndex, id: &str, now_ms: i64) -> Result<bool, DurableError> {
    let n = idx.db().execute(
        "UPDATE memories SET review_state = 'rejected', updated_at_ms = ?1 \
         WHERE id = ?2 AND archived_at_ms IS NULL",
        rusqlite::params![now_ms, id],
    )?;
    Ok(n > 0)
}

/// Record that an accepted claim was compiled into a binding check.
pub fn mark_compiled(
    idx: &SessionIndex,
    id: &str,
    target: &str,
    guard_ref: &str,
    now_ms: i64,
) -> Result<bool, DurableError> {
    let n = idx.db().execute(
        "UPDATE memories SET compile_target = ?1, guard_ref = ?2, updated_at_ms = ?3 \
         WHERE id = ?4 AND review_state = 'accepted'",
        rusqlite::params![target, guard_ref, now_ms, id],
    )?;
    Ok(n > 0)
}

pub fn counts(
    idx: &SessionIndex,
    project_path: Option<&str>,
) -> Result<ReviewCounts, DurableError> {
    let db = idx.db();
    let mut c = ReviewCounts::default();
    let (sql, bind): (&str, Vec<rusqlite::types::Value>) = match project_path {
        Some(p) => (
            "SELECT review_state, COUNT(*) FROM memories \
             WHERE archived_at_ms IS NULL AND project_path = ?1 GROUP BY review_state",
            vec![rusqlite::types::Value::Text(p.to_string())],
        ),
        None => (
            "SELECT review_state, COUNT(*) FROM memories \
             WHERE archived_at_ms IS NULL GROUP BY review_state",
            vec![],
        ),
    };
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind.iter()), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (state, n) = row?;
        match state.as_str() {
            "proposed" => c.proposed = n,
            "accepted" => c.accepted = n,
            "rejected" => c.rejected = n,
            "suspect" => c.suspect = n,
            _ => {}
        }
    }
    // `enforced` must respect the same project scope as the other
    // counts — a per-project dashboard was showing the GLOBAL enforced
    // total next to this project's proposed/accepted numbers.
    c.enforced = match project_path {
        Some(p) => db.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived_at_ms IS NULL \
             AND review_state = 'accepted' AND compile_target = 'guard' \
             AND project_path = ?1",
            [p],
            |r| r.get(0),
        )?,
        None => db.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived_at_ms IS NULL \
             AND review_state = 'accepted' AND compile_target = 'guard'",
            [],
            |r| r.get(0),
        )?,
    };
    Ok(c)
}

/// Sessions in `project_path` that have never produced a proposal.
///
/// `origin_file_path` is our record of what has already been mined. It
/// is denormalized onto the memory row (rather than living in
/// `memory_links`) precisely so it survives an index rebuild — otherwise
/// a rebuild would make the next harvest re-distill, and re-pay for,
/// every transcript already processed.
pub fn undistilled_sessions(
    idx: &SessionIndex,
    project_path: &str,
    limit: u32,
) -> Result<Vec<String>, DurableError> {
    let limit = if limit == 0 { 20 } else { limit.min(500) };
    let db = idx.db();
    let mut stmt = db.prepare(
        "SELECT s.file_path FROM sessions s \
         WHERE s.project_path = ?1 \
           AND s.file_path NOT IN \
               (SELECT origin_file_path FROM memories WHERE origin_file_path IS NOT NULL) \
         ORDER BY s.last_ts_ms DESC NULLS LAST \
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![project_path, limit as i64], |r| {
        r.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Merge `{"commit": sha}` into the row's existing anchor object,
/// preserving the files the distiller recorded.
fn merge_commit_into_anchor(
    idx: &SessionIndex,
    id: &str,
    sha: &str,
) -> Result<Option<String>, DurableError> {
    let existing: Option<String> = idx
        .db()
        .query_row(
            "SELECT anchor_json FROM memories WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let mut v: serde_json::Value = existing
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("commit".into(), serde_json::Value::String(sha.to_string()));
    }
    Ok(serde_json::to_string(&v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_memory::proposal::{
        ingest_proposals, DistilledClaim, DistilledClaims, ProposalOrigin,
    };
    use tempfile::TempDir;

    fn seed() -> (TempDir, SessionIndex, String) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: "preflight runs guards that cargo test does not".into(),
                directive: "Run scripts/preflight.sh before pushing.".into(),
                kind: "constraint".into(),
                files: vec!["scripts/preflight.sh".into()],
                evidence: "CI went red after a clean local run.".into(),
                confidence: 90,
            }],
        };
        ingest_proposals(
            &idx,
            &claims,
            &ProposalOrigin {
                project_path: "/work/app",
                file_path: Some("/t/s.jsonl"),
                exchange_id: None,
                created_by: "agent:knowledge-distiller",
            },
            1_000,
        )
        .unwrap();
        let id: String = idx
            .db()
            .query_row("SELECT id FROM memories", [], |r| r.get(0))
            .unwrap();
        (tmp, idx, id)
    }

    #[test]
    fn a_fresh_proposal_shows_up_in_the_queue_with_its_evidence() {
        let (_t, idx, _id) = seed();
        let rows = list(&idx, Some("/work/app"), Some(ReviewState::Proposed), 0).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(r.directive.as_deref().unwrap().contains("preflight.sh"));
        // The "show me what burned me" link survives an index rebuild
        // because it is denormalized onto the row.
        assert_eq!(r.origin_file_path.as_deref(), Some("/t/s.jsonl"));
    }

    #[test]
    fn accepting_stamps_the_commit_and_keeps_the_files() {
        // files + commit together are what make invalidation possible:
        // "these files, at this revision, are what I agreed about".
        let (_t, idx, id) = seed();
        assert!(accept(&idx, &id, Some("abc1234"), 2_000).unwrap());

        let rows = list(&idx, None, Some(ReviewState::Accepted), 0).unwrap();
        assert_eq!(rows.len(), 1);
        let anchor = rows[0].anchor_json.as_deref().unwrap();
        assert!(anchor.contains("abc1234"), "commit must be stamped");
        assert!(
            anchor.contains("preflight.sh"),
            "the distiller's files must survive the merge"
        );
    }

    #[test]
    fn accepting_without_an_anchor_is_allowed_but_can_never_go_suspect() {
        // Right for a lesson that is not about code — a preference, a
        // fact about the user. Wrong for anything else, which is why it
        // has to be an explicit choice rather than the default.
        let (_t, idx, id) = seed();
        assert!(accept(&idx, &id, None, 2_000).unwrap());
        let rows = list(&idx, None, Some(ReviewState::Accepted), 0).unwrap();
        let anchor = rows[0].anchor_json.as_deref().unwrap_or("{}");
        assert!(!anchor.contains("commit"));
    }

    #[test]
    fn a_rejected_row_is_kept_not_deleted() {
        // Deleting it would let the harvester re-derive and re-file the
        // same lesson from the same transcript, forever.
        let (_t, idx, id) = seed();
        assert!(reject(&idx, &id, 2_000).unwrap());

        assert!(list(&idx, None, Some(ReviewState::Proposed), 0)
            .unwrap()
            .is_empty());
        assert_eq!(
            list(&idx, None, Some(ReviewState::Rejected), 0)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn only_an_accepted_claim_can_be_marked_compiled() {
        // Compiling a proposal would put an unreviewed claim into a
        // binding check — the exact thing the review gate exists for.
        let (_t, idx, id) = seed();
        assert!(
            !mark_compiled(&idx, &id, "guard", "scripts/repo-invariants.sh:6", 2_000).unwrap(),
            "a proposal must not be compilable"
        );
        accept(&idx, &id, Some("abc"), 2_000).unwrap();
        assert!(mark_compiled(&idx, &id, "guard", "scripts/repo-invariants.sh:6", 3_000).unwrap());
    }

    #[test]
    fn counts_report_enforced_not_merely_stored() {
        let (_t, idx, id) = seed();
        let c = counts(&idx, None).unwrap();
        assert_eq!(c.proposed, 1);
        assert_eq!(c.enforced, 0, "nothing is enforced until it is compiled");

        accept(&idx, &id, Some("abc"), 2_000).unwrap();
        mark_compiled(&idx, &id, "guard", "scripts/repo-invariants.sh:6", 3_000).unwrap();
        let c = counts(&idx, None).unwrap();
        assert_eq!(c.accepted, 1);
        assert_eq!(c.enforced, 1);
    }

    #[test]
    fn accepting_an_unknown_id_reports_false_rather_than_pretending() {
        let (_t, idx, _id) = seed();
        assert!(!accept(&idx, "no-such-id", None, 1).unwrap());
        assert!(!reject(&idx, "no-such-id", 1).unwrap());
    }
}
