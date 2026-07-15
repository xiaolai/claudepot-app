//! Recurrence detection — the one honest "is any of this working?" signal.
//!
//! # What no other memory tool measures
//!
//! Copilot expires memories by usage; Cursor never measures outcome. This
//! measures the thing the whole compiler exists to prevent: **did a
//! failure class we already learned about happen again?** If the agent hit
//! the same wall a second time after we'd learned it, that is the failure —
//! surfaced.
//!
//! # Detection, not a verdict
//!
//! At ingest ([`crate::shared_memory::proposal::ingest_proposals`]), before
//! filing a new proposal, we check it against the project's **accepted** or
//! **suspect** lessons (the ones a human committed to — a `proposed` or
//! `rejected` match doesn't count). Two cheap, dependency-free signals:
//!
//! - **Anchor overlap** — the new claim's files intersect a prior lesson's
//!   anchored files, using the same segment-boundary rule as
//!   [`crate::shared_memory::invalidate::paths_match`] (`core.rs` ≠
//!   `score.rs`). Precise and high-signal.
//! - **Claim/directive similarity** — a normalized token-overlap (Jaccard)
//!   over the claim + directive. No embeddings: auditable and offline.
//!
//! A match is filed as a `pending` [`RecurrenceEvent`] and surfaced in
//! Review. **It is never auto-counted.** METR showed developers feel 20%
//! faster while being 19% slower — soft signals lie, and a fuzzy match
//! silently incrementing a counter would be exactly that lie. Only a
//! human-**confirmed** recurrence is a real datum, and the dashboard counts
//! only those. The payoff of a confirmed recurrence is an action: compile
//! the class to a guard so it *cannot* recur, driving the number to zero.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::session_index::SessionIndex;
use crate::shared_memory::durable::DurableError;
use crate::shared_memory::invalidate::paths_match;

/// Jaccard token-overlap at or above this is a similarity candidate.
///
/// Tuned for **precision over recall**: a false candidate the user never
/// confirms is the failure mode (see the doc's kill criteria — "if
/// candidates are almost never confirmed, drop the automated matching").
/// Anchor overlap carries the recall; similarity is the softer net, so it
/// is deliberately strict. This threshold is the one number in the whole
/// feature that wants a real eval against your own history.
pub const SIMILARITY_THRESHOLD: f64 = 0.6;

/// Tokens shorter than this are dropped before scoring — they are noise
/// (`a`, `to`, `is`) that inflates overlap between unrelated claims.
const MIN_TOKEN_LEN: usize = 3;

/// How a recurrence was spotted. `anchor` is the precise signal; keeping
/// it on the row lets the UI say *why* and lets a later eval weigh the two
/// detectors separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectedBy {
    Anchor,
    Similarity,
}

impl DetectedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            DetectedBy::Anchor => "anchor",
            DetectedBy::Similarity => "similarity",
        }
    }
}

/// A committed lesson (accepted or suspect) a new claim is checked against.
#[derive(Debug, Clone)]
pub struct PriorLesson {
    pub id: String,
    pub content: String,
    pub directive: Option<String>,
    pub files: Vec<String>,
}

/// The prior lesson a new claim recurred against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurrenceMatch {
    pub memory_id: String,
    pub detected_by: DetectedBy,
}

// ─── pure detection ───────────────────────────────────────────

/// Does this new claim recur against any prior committed lesson? Pure and
/// I/O-free so the heuristic and its threshold are testable without a DB.
///
/// Anchor overlap wins over similarity when both fire — it is the more
/// precise signal and the one worth surfacing as the reason.
pub fn detect_match(
    new_files: &[String],
    new_claim: &str,
    new_directive: &str,
    priors: &[PriorLesson],
) -> Option<RecurrenceMatch> {
    // 1. Anchor overlap — cheap and precise. Empty path strings are
    // skipped on both sides: `paths_match`'s suffix rule treats an empty
    // needle as matching any path ending in `/`, and model-generated
    // `files` arrays (unlike git-diff output) can contain "".
    for p in priors {
        let overlaps = new_files.iter().filter(|nf| !nf.is_empty()).any(|nf| {
            p.files
                .iter()
                .filter(|pf| !pf.is_empty())
                .any(|pf| paths_match(nf, pf))
        });
        if overlaps {
            return Some(RecurrenceMatch {
                memory_id: p.id.clone(),
                detected_by: DetectedBy::Anchor,
            });
        }
    }

    // 2. Claim/directive similarity — the softer net, strict threshold.
    let new_tokens = tokenize(&format!("{new_claim} {new_directive}"));
    if new_tokens.is_empty() {
        return None;
    }
    let mut best: Option<(String, f64)> = None;
    for p in priors {
        let prior_text = match &p.directive {
            Some(d) => format!("{} {}", p.content, d),
            None => p.content.clone(),
        };
        let score = jaccard(&new_tokens, &tokenize(&prior_text));
        if score >= SIMILARITY_THRESHOLD && best.as_ref().is_none_or(|(_, b)| score > *b) {
            best = Some((p.id.clone(), score));
        }
    }
    best.map(|(memory_id, _)| RecurrenceMatch {
        memory_id,
        detected_by: DetectedBy::Similarity,
    })
}

/// Lowercased alphanumeric tokens, dropping sub-[`MIN_TOKEN_LEN`] noise.
/// A `BTreeSet` both de-dupes and makes the Jaccard sets deterministic.
pub fn tokenize(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= MIN_TOKEN_LEN)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// |A ∩ B| / |A ∪ B|. Zero when either set is empty.
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.len() + b.len() - inter;
    inter as f64 / union as f64
}

// ─── the durable row ──────────────────────────────────────────

/// One filed recurrence. `pending` until a human confirms or dismisses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecurrenceEvent {
    pub id: String,
    pub matched_memory_id: String,
    pub project_path: String,
    /// The recurring claim, in the new session's words (redacted at
    /// ingest, so safe to store).
    pub new_content: String,
    pub new_exchange_id: Option<String>,
    pub new_file_path: Option<String>,
    pub detected_by: String,
    pub detected_at_ms: i64,
    pub status: String,
    pub confirmed_at_ms: Option<i64>,
    /// Joined from the matched lesson for display: what we already learned.
    pub matched_content: Option<String>,
    /// The matched lesson's current review state (`accepted` / `suspect`).
    pub matched_state: Option<String>,
}

/// Args to [`record`].
#[derive(Debug, Clone)]
pub struct NewRecurrence<'a> {
    pub matched_memory_id: &'a str,
    pub project_path: &'a str,
    pub new_content: &'a str,
    pub new_exchange_id: Option<&'a str>,
    pub new_file_path: Option<&'a str>,
    pub detected_by: DetectedBy,
}

// ─── store ────────────────────────────────────────────────────

/// The committed lessons in `project_path` a new claim can recur against:
/// accepted or suspect, not archived. `proposed` and `rejected` are
/// excluded — we only count recurrences of things a human agreed were real.
pub fn prior_lessons(
    idx: &SessionIndex,
    project_path: &str,
) -> Result<Vec<PriorLesson>, DurableError> {
    let db = idx.db();
    let mut stmt = db.prepare(
        "SELECT id, content, directive, anchor_json FROM memories \
         WHERE project_path = ?1 AND archived_at_ms IS NULL \
           AND review_state IN ('accepted','suspect')",
    )?;
    let rows = stmt.query_map([project_path], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, content, directive, anchor_json) = row?;
        let files = anchor_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| {
                v.get("files").and_then(|f| f.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        out.push(PriorLesson {
            id,
            content,
            directive,
            files,
        });
    }
    Ok(out)
}

/// File a recurrence. Idempotent per `(matched_memory_id, new_content)`:
/// re-running the same harvest does not pile up duplicate events, and a
/// candidate the user already **dismissed** stays dismissed (the dedup
/// check ignores status — same reason a rejected proposal never re-files).
/// Returns the new id, or `None` if this recurrence was already recorded.
pub fn record(
    idx: &SessionIndex,
    new: &NewRecurrence<'_>,
    now_ms: i64,
) -> Result<Option<String>, DurableError> {
    let db = idx.db();
    let already: bool = db
        .query_row(
            "SELECT 1 FROM recurrence_events \
             WHERE matched_memory_id = ?1 AND new_content = ?2 LIMIT 1",
            rusqlite::params![new.matched_memory_id, new.new_content],
            |_| Ok(true),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(DurableError::from(other)),
        })?;
    if already {
        return Ok(None);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let res = db.execute(
        "INSERT INTO recurrence_events \
         (id, matched_memory_id, project_path, new_content, new_exchange_id, \
          new_file_path, detected_by, detected_at_ms, status, confirmed_at_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL)",
        rusqlite::params![
            id,
            new.matched_memory_id,
            new.project_path,
            new.new_content,
            new.new_exchange_id,
            new.new_file_path,
            new.detected_by.as_str(),
            now_ms,
        ],
    );
    match res {
        Ok(_) => Ok(Some(id)),
        // The UNIQUE(matched_memory_id, new_content) backstop firing means
        // a concurrent harvest recorded this same recurrence between our
        // SELECT and INSERT — same outcome as the pre-check: already there.
        // Match ONLY the unique collision (extended code), not any
        // constraint violation — an FK/CHECK failure is a real error and
        // must propagate, not be masked as "already recorded".
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Ok(None)
        }
        Err(e) => Err(DurableError::from(e)),
    }
}

/// Pending recurrences (the Review surface), newest first, joined to the
/// matched lesson's content + state so the UI can say *what* recurred and
/// *what we already knew* without a second query.
pub fn list_pending(
    idx: &SessionIndex,
    project_path: Option<&str>,
    limit: u32,
) -> Result<Vec<RecurrenceEvent>, DurableError> {
    let limit = if limit == 0 { 100 } else { limit.min(500) };
    let db = idx.db();
    let (sql, binds): (String, Vec<rusqlite::types::Value>) = {
        let base = "SELECT r.id, r.matched_memory_id, r.project_path, r.new_content, \
                    r.new_exchange_id, r.new_file_path, r.detected_by, r.detected_at_ms, \
                    r.status, r.confirmed_at_ms, m.content, m.review_state \
             FROM recurrence_events r \
             LEFT JOIN memories m ON m.id = r.matched_memory_id \
             WHERE r.status = 'pending'"
            .to_string();
        match project_path {
            Some(p) => (
                format!("{base} AND r.project_path = ?1 ORDER BY r.detected_at_ms DESC LIMIT ?2"),
                vec![
                    rusqlite::types::Value::Text(p.to_string()),
                    rusqlite::types::Value::Integer(limit as i64),
                ],
            ),
            None => (
                format!("{base} ORDER BY r.detected_at_ms DESC LIMIT ?1"),
                vec![rusqlite::types::Value::Integer(limit as i64)],
            ),
        }
    };
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| {
        Ok(RecurrenceEvent {
            id: r.get(0)?,
            matched_memory_id: r.get(1)?,
            project_path: r.get(2)?,
            new_content: r.get(3)?,
            new_exchange_id: r.get(4)?,
            new_file_path: r.get(5)?,
            detected_by: r.get(6)?,
            detected_at_ms: r.get(7)?,
            status: r.get(8)?,
            confirmed_at_ms: r.get(9)?,
            matched_content: r.get(10)?,
            matched_state: r.get(11)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Mark a pending recurrence confirmed — the human looked and it is the
/// same class. Only a pending row transitions.
pub fn confirm(idx: &SessionIndex, id: &str, now_ms: i64) -> Result<bool, DurableError> {
    let n = idx.db().execute(
        "UPDATE recurrence_events SET status = 'confirmed', confirmed_at_ms = ?1 \
         WHERE id = ?2 AND status = 'pending'",
        rusqlite::params![now_ms, id],
    )?;
    Ok(n > 0)
}

/// Mark a pending recurrence dismissed — a false candidate. Kept (not
/// deleted) so the ingest dedup never re-files it.
pub fn dismiss(idx: &SessionIndex, id: &str) -> Result<bool, DurableError> {
    let n = idx.db().execute(
        "UPDATE recurrence_events SET status = 'dismissed' \
         WHERE id = ?1 AND status = 'pending'",
        [id],
    )?;
    Ok(n > 0)
}

/// Confirmed recurrences detected on or after `since_ms` — the dashboard's
/// headline, trending to zero as guards absorb the classes.
pub fn confirmed_count_since(idx: &SessionIndex, since_ms: i64) -> Result<i64, DurableError> {
    Ok(idx.db().query_row(
        "SELECT COUNT(*) FROM recurrence_events \
         WHERE status = 'confirmed' AND detected_at_ms >= ?1",
        [since_ms],
        |r| r.get(0),
    )?)
}

/// How many recurrences are awaiting a human decision.
pub fn pending_count(idx: &SessionIndex) -> Result<i64, DurableError> {
    Ok(idx.db().query_row(
        "SELECT COUNT(*) FROM recurrence_events WHERE status = 'pending'",
        [],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_memory::proposal::{
        ingest_proposals, DistilledClaim, DistilledClaims, ProposalOrigin,
    };
    use crate::shared_memory::review;
    use tempfile::TempDir;

    // ─── pure detection ────────────────────────────────────────

    fn prior(id: &str, content: &str, files: &[&str]) -> PriorLesson {
        PriorLesson {
            id: id.into(),
            content: content.into(),
            directive: None,
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn anchor_overlap_is_a_match_and_wins_over_similarity() {
        let priors = vec![prior("m1", "totally unrelated words here", &["src/foo.rs"])];
        let m = detect_match(
            &["src/foo.rs".into()],
            "brand new claim",
            "do a thing",
            &priors,
        )
        .expect("overlapping file must match");
        assert_eq!(m.memory_id, "m1");
        assert_eq!(m.detected_by, DetectedBy::Anchor);
    }

    #[test]
    fn anchor_overlap_respects_segment_boundaries() {
        // core.rs must not match score.rs — the shared paths_match rule.
        let priors = vec![prior("m1", "x", &["src/score.rs"])];
        assert!(detect_match(&["core.rs".into()], "c", "d", &priors).is_none());
    }

    #[test]
    fn high_token_overlap_is_a_similarity_match() {
        let priors = vec![prior(
            "m1",
            "run preflight guards before pushing to remote",
            &[],
        )];
        let m = detect_match(
            &[],
            "run preflight guards before pushing to remote",
            "",
            &priors,
        )
        .expect("identical text must clear the threshold");
        assert_eq!(m.detected_by, DetectedBy::Similarity);
    }

    #[test]
    fn unrelated_claims_do_not_match() {
        let priors = vec![prior("m1", "the sky is blue on tuesdays", &[])];
        assert!(detect_match(&[], "windows paths use backslash separators", "", &priors).is_none());
    }

    #[test]
    fn an_empty_new_file_string_does_not_false_match() {
        // A model-emitted files:[""] against a prior anchor path ending in
        // "/" would false-match through paths_match's empty-needle quirk.
        let priors = vec![prior("m1", "unrelated words entirely here", &["src/"])];
        assert!(
            detect_match(&["".into()], "brand new unrelated claim", "", &priors).is_none(),
            "an empty file path must not fire an anchor recurrence"
        );
    }

    #[test]
    fn jaccard_is_zero_for_disjoint_and_one_for_identical() {
        let a = tokenize("alpha bravo charlie");
        let b = tokenize("alpha bravo charlie");
        assert!((jaccard(&a, &b) - 1.0).abs() < 1e-9);
        let c = tokenize("delta echo foxtrot");
        assert_eq!(jaccard(&a, &c), 0.0);
    }

    // ─── store + ingest wiring ─────────────────────────────────

    fn seed_accepted(idx: &SessionIndex, project: &str, claim: &str, files: &[&str]) -> String {
        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: claim.into(),
                directive: format!("Directive for {claim}."),
                kind: "constraint".into(),
                files: files.iter().map(|s| s.to_string()).collect(),
                evidence: "it burned us".into(),
                confidence: 90,
            }],
        };
        ingest_proposals(
            idx,
            &claims,
            &ProposalOrigin {
                project_path: project,
                file_path: Some("/t/old.jsonl"),
                exchange_id: None,
                created_by: "agent:distiller",
            },
            1_000,
        )
        .unwrap();
        let id: String = idx
            .db()
            .query_row("SELECT id FROM memories WHERE content = ?1", [claim], |r| {
                r.get(0)
            })
            .unwrap();
        review::accept(idx, &id, Some("abc123"), 2_000).unwrap();
        id
    }

    fn new_idx() -> (TempDir, SessionIndex) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        (tmp, idx)
    }

    #[test]
    fn a_new_claim_overlapping_an_accepted_lesson_files_a_recurrence() {
        let (_t, idx) = new_idx();
        let matched = seed_accepted(&idx, "/proj/app", "call foo before bar", &["src/foo.rs"]);

        // A DIFFERENT claim in the same project that touches the same file.
        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: "foo must be initialised first".into(),
                directive: "Init foo() early.".into(),
                kind: "constraint".into(),
                files: vec!["src/foo.rs".into()],
                evidence: "same wall again".into(),
                confidence: 88,
            }],
        };
        let report = ingest_proposals(
            &idx,
            &claims,
            &ProposalOrigin {
                project_path: "/proj/app",
                file_path: Some("/t/new.jsonl"),
                exchange_id: Some("s2:3"),
                created_by: "agent:distiller",
            },
            3_000,
        )
        .unwrap();
        assert_eq!(report.recurrences_detected, 1);

        let pending = list_pending(&idx, Some("/proj/app"), 0).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].matched_memory_id, matched);
        assert_eq!(pending[0].detected_by, "anchor");
        assert_eq!(pending[0].new_file_path.as_deref(), Some("/t/new.jsonl"));
        assert_eq!(pending[0].matched_state.as_deref(), Some("accepted"));
    }

    #[test]
    fn a_match_in_a_different_project_is_not_a_recurrence() {
        let (_t, idx) = new_idx();
        seed_accepted(&idx, "/proj/a", "call foo before bar", &["src/foo.rs"]);

        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: "foo needs early init".into(),
                directive: "Init foo().".into(),
                kind: "constraint".into(),
                files: vec!["src/foo.rs".into()],
                evidence: "e".into(),
                confidence: 88,
            }],
        };
        let report = ingest_proposals(
            &idx,
            &claims,
            &ProposalOrigin {
                project_path: "/proj/b", // different project
                file_path: Some("/t/new.jsonl"),
                exchange_id: None,
                created_by: "agent:distiller",
            },
            3_000,
        )
        .unwrap();
        assert_eq!(report.recurrences_detected, 0);
        assert!(list_pending(&idx, None, 0).unwrap().is_empty());
    }

    #[test]
    fn a_match_against_a_rejected_lesson_is_not_a_recurrence() {
        // We only count recurrences of things a human COMMITTED to.
        let (_t, idx) = new_idx();
        let id = seed_accepted(&idx, "/proj/app", "call foo before bar", &["src/foo.rs"]);
        review::reject(&idx, &id, 2_500).unwrap();

        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: "foo early init".into(),
                directive: "Init foo().".into(),
                kind: "constraint".into(),
                files: vec!["src/foo.rs".into()],
                evidence: "e".into(),
                confidence: 88,
            }],
        };
        let report = ingest_proposals(
            &idx,
            &claims,
            &ProposalOrigin {
                project_path: "/proj/app",
                file_path: Some("/t/new.jsonl"),
                exchange_id: None,
                created_by: "agent:distiller",
            },
            3_000,
        )
        .unwrap();
        assert_eq!(report.recurrences_detected, 0);
    }

    #[test]
    fn recording_the_same_recurrence_twice_is_idempotent() {
        let (_t, idx) = new_idx();
        let matched = seed_accepted(&idx, "/proj/app", "call foo before bar", &["src/foo.rs"]);
        let ev = NewRecurrence {
            matched_memory_id: &matched,
            project_path: "/proj/app",
            new_content: "foo early init",
            new_exchange_id: None,
            new_file_path: None,
            detected_by: DetectedBy::Anchor,
        };
        assert!(record(&idx, &ev, 3_000).unwrap().is_some());
        assert!(
            record(&idx, &ev, 3_100).unwrap().is_none(),
            "dup suppressed"
        );
        assert_eq!(list_pending(&idx, None, 0).unwrap().len(), 1);
    }

    #[test]
    fn confirm_moves_it_out_of_pending_and_into_the_count() {
        let (_t, idx) = new_idx();
        let matched = seed_accepted(&idx, "/proj/app", "call foo before bar", &["src/foo.rs"]);
        let id = record(
            &idx,
            &NewRecurrence {
                matched_memory_id: &matched,
                project_path: "/proj/app",
                new_content: "foo early init",
                new_exchange_id: None,
                new_file_path: None,
                detected_by: DetectedBy::Anchor,
            },
            3_000,
        )
        .unwrap()
        .unwrap();

        assert_eq!(confirmed_count_since(&idx, 0).unwrap(), 0);
        assert!(confirm(&idx, &id, 4_000).unwrap());
        assert!(list_pending(&idx, None, 0).unwrap().is_empty());
        assert_eq!(confirmed_count_since(&idx, 0).unwrap(), 1);
        // Window excludes older detections.
        assert_eq!(confirmed_count_since(&idx, 3_500).unwrap(), 0);
    }

    #[test]
    fn a_dismissed_candidate_stays_gone_and_never_re_files() {
        let (_t, idx) = new_idx();
        let matched = seed_accepted(&idx, "/proj/app", "call foo before bar", &["src/foo.rs"]);
        let ev = NewRecurrence {
            matched_memory_id: &matched,
            project_path: "/proj/app",
            new_content: "foo early init",
            new_exchange_id: None,
            new_file_path: None,
            detected_by: DetectedBy::Anchor,
        };
        let id = record(&idx, &ev, 3_000).unwrap().unwrap();
        assert!(dismiss(&idx, &id).unwrap());
        assert!(list_pending(&idx, None, 0).unwrap().is_empty());
        // The ingest dedup must not resurrect a dismissed candidate.
        assert!(record(&idx, &ev, 3_200).unwrap().is_none());
    }
}
