//! Invalidation by correctness — the moat.
//!
//! # What everyone else does, and why it isn't enough
//!
//! Copilot expires a memory by **usage**: a memory nobody triggers
//! decays. That measures popularity, not truth. The dangerous memory is
//! the one that is *frequently used and quietly wrong* — usage-decay
//! actively protects it, because it keeps getting used.
//!
//! # What this does
//!
//! An accepted lesson is anchored to the *files* it depends on and the
//! *commit* those files were at when a human agreed it was true (see
//! `shared_memory::review::accept`). This module asks a different
//! question: **have any of those files changed since that commit?** If
//! they have, the world the human agreed about is gone, and the claim
//! is no longer known to hold. It goes back to the triage queue as
//! `suspect`, with the reason attached — *"the code this was based on
//! changed; still true?"*
//!
//! # Why the logic is pure
//!
//! "Did these files change between commit A and now" is a `git` call,
//! and `git` is I/O the orchestrator owns. This module takes the answer
//! as data (a [`ChangedFiles`] closure) so the decision — which claims
//! flip, and what reason is recorded — is testable without a
//! repository. The Tauri bridge in `invalidation_orchestrator.rs`
//! supplies the real `git diff`.

use serde::Serialize;

use crate::session_index::SessionIndex;
use crate::shared_memory::durable::DurableError;

/// An accepted, code-anchored claim that is a candidate for staleness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredClaim {
    pub id: String,
    pub project_path: String,
    /// The commit the anchored files were at when accepted.
    pub anchor_commit: String,
    /// Repo-relative paths the claim depends on.
    pub files: Vec<String>,
}

/// One claim flipped to `suspect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Invalidation {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct InvalidateReport {
    /// Claims moved to `suspect` this sweep.
    pub invalidated: Vec<Invalidation>,
    /// Claims checked and confirmed still fresh. (Unanchored claims are
    /// filtered out by `anchored_claims` before they reach `evaluate`,
    /// so there is no "skipped" bucket — everything here was checkable.)
    pub still_fresh: u32,
}

/// Every accepted claim in `project_path` that carries a commit anchor
/// AND at least one file. A claim with files but no commit was accepted
/// with `--no-anchor`; a claim with a commit but no files has nothing
/// to diff. Both are intentionally excluded — they can never go
/// suspect, and that is a property of how they were accepted, not a bug.
pub fn anchored_claims(
    idx: &SessionIndex,
    project_path: &str,
) -> Result<Vec<AnchoredClaim>, DurableError> {
    let db = idx.db();
    let mut stmt = db.prepare(
        "SELECT id, anchor_json FROM memories \
         WHERE review_state = 'accepted' AND archived_at_ms IS NULL \
           AND project_path = ?1 AND anchor_json IS NOT NULL",
    )?;
    let rows = stmt.query_map([project_path], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, anchor_json) = row?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&anchor_json) else {
            continue;
        };
        let commit = v.get("commit").and_then(|c| c.as_str());
        let files: Vec<String> = v
            .get("files")
            .and_then(|f| f.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        match commit {
            Some(c) if !files.is_empty() => out.push(AnchoredClaim {
                id,
                project_path: project_path.to_string(),
                anchor_commit: c.to_string(),
                files,
            }),
            _ => {}
        }
    }
    Ok(out)
}

/// Every accepted, code-anchored claim across ALL projects. The
/// orchestrator uses this to discover which project directories are
/// worth shelling out to `git` for — sweeping every path in the index
/// would run `git` in directories that may not be repos.
pub fn anchored_claims_all(idx: &SessionIndex) -> Result<Vec<AnchoredClaim>, DurableError> {
    let projects: Vec<String> = {
        let db = idx.db();
        let mut stmt = db.prepare(
            "SELECT DISTINCT project_path FROM memories \
             WHERE review_state = 'accepted' AND archived_at_ms IS NULL \
               AND project_path IS NOT NULL AND anchor_json IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        out
    };
    let mut all = Vec::new();
    for p in projects {
        all.extend(anchored_claims(idx, &p)?);
    }
    Ok(all)
}

/// The set of files that changed between a commit and the working tree.
/// The orchestrator implements this with `git diff --name-only <commit>`;
/// tests supply a fake. Returns `None` when the answer is *unknowable*
/// (e.g. the anchor commit is not in this repo — a shallow clone, a
/// force-push, a different checkout), which must NOT be treated as
/// "nothing changed": see [`evaluate`].
pub type ChangedFiles<'a> = dyn Fn(&str) -> Option<Vec<String>> + 'a;

/// Decide which claims to invalidate, purely.
///
/// A claim is invalidated when at least one of its anchored files
/// appears in the change set since its anchor commit.
///
/// **Unknowable ≠ unchanged.** When `changed_since` returns `None` (the
/// commit isn't in the repo, so we cannot compute a diff), the claim is
/// NOT invalidated — but it is NOT confirmed fresh either. Flipping a
/// claim to suspect because we lost its commit would punish the user for
/// a rebase; silently counting it fresh would let a genuinely-stale
/// claim hide behind a missing commit. It is simply left untouched and
/// not counted as fresh, so a caller can tell the two apart.
pub fn evaluate(claims: &[AnchoredClaim], changed_since: &ChangedFiles<'_>) -> InvalidateReport {
    let mut report = InvalidateReport::default();
    for claim in claims {
        match changed_since(&claim.anchor_commit) {
            None => {
                // Unknowable. Leave it accepted; don't count it fresh.
            }
            Some(changed) => {
                let hit = claim
                    .files
                    .iter()
                    .find(|f| changed.iter().any(|c| paths_match(c, f)));
                match hit {
                    Some(file) => report.invalidated.push(Invalidation {
                        id: claim.id.clone(),
                        reason: format!(
                            "{} changed since this was accepted at {} — still true?",
                            file,
                            short_commit(&claim.anchor_commit)
                        ),
                    }),
                    None => report.still_fresh += 1,
                }
            }
        }
    }
    report
}

/// Apply the decision: flip each invalidated claim to `suspect` with its
/// reason, and stamp `updated_at_ms`. Returns how many rows changed.
pub fn apply(
    idx: &SessionIndex,
    report: &InvalidateReport,
    now_ms: i64,
) -> Result<u32, DurableError> {
    // One transaction for the whole batch: a mid-loop SQLite error must
    // not leave some claims flipped to suspect and others not while the
    // caller receives an error. All-or-nothing.
    let db = idx.db();
    let tx = db.unchecked_transaction()?;
    let mut n = 0u32;
    for inv in &report.invalidated {
        let changed = tx.execute(
            "UPDATE memories SET review_state = 'suspect', suspect_reason = ?1, \
             updated_at_ms = ?2 WHERE id = ?3 AND review_state = 'accepted'",
            rusqlite::params![inv.reason, now_ms, inv.id],
        )?;
        n += changed as u32;
    }
    tx.commit()?;
    Ok(n)
}

/// A `git diff --name-only` path matches an anchor path when one is a
/// suffix of the other on a path-segment boundary. The distiller records
/// repo-relative paths; `git diff` also emits repo-relative paths, so
/// they usually match exactly — but the distiller sometimes records a
/// path relative to a subdirectory, so a boundary-aware suffix match is
/// the forgiving-but-not-sloppy rule. (`core.rs` must not match
/// `score.rs`; the segment boundary is what prevents that.)
fn paths_match(git_path: &str, anchor_path: &str) -> bool {
    // Normalize separators before comparing. `git diff` emits
    // forward-slash paths on every platform, but the distiller records
    // whatever the model wrote — which on a Windows repo can be
    // `src\foo.rs`. Without this, a Windows anchor never matches its own
    // git output and the lesson silently never goes suspect.
    let g = normalize_sep(git_path);
    let a = normalize_sep(anchor_path);
    g == a || ends_on_boundary(&g, &a) || ends_on_boundary(&a, &g)
}

fn normalize_sep(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
}

fn ends_on_boundary(haystack: &str, needle: &str) -> bool {
    if !haystack.ends_with(needle) {
        return false;
    }
    let prefix_len = haystack.len() - needle.len();
    prefix_len == 0 || haystack.as_bytes()[prefix_len - 1] == b'/'
}

fn short_commit(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_memory::proposal::{
        ingest_proposals, DistilledClaim, DistilledClaims, ProposalOrigin,
    };
    use crate::shared_memory::review;
    use tempfile::TempDir;

    fn seed_accepted(commit: &str, files: &[&str]) -> (TempDir, SessionIndex, String) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: "must call foo before bar".into(),
                directive: "Call foo() before bar().".into(),
                kind: "constraint".into(),
                files: files.iter().map(|s| s.to_string()).collect(),
                evidence: "bar panicked when foo hadn't run.".into(),
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
                created_by: "agent:distiller",
            },
            1_000,
        )
        .unwrap();
        let id: String = idx
            .db()
            .query_row("SELECT id FROM memories", [], |r| r.get(0))
            .unwrap();
        review::accept(&idx, &id, Some(commit), 2_000).unwrap();
        (tmp, idx, id)
    }

    #[test]
    fn a_claim_whose_anchored_file_changed_goes_suspect() {
        let (_t, idx, id) = seed_accepted("abc123", &["src/foo.rs"]);
        let claims = anchored_claims(&idx, "/work/app").unwrap();
        assert_eq!(claims.len(), 1);

        // The file it depends on changed since the anchor commit.
        let changed = |_c: &str| Some(vec!["src/foo.rs".to_string()]);
        let report = evaluate(&claims, &changed);
        assert_eq!(report.invalidated.len(), 1);
        assert!(report.invalidated[0].reason.contains("src/foo.rs"));

        let n = apply(&idx, &report, 3_000).unwrap();
        assert_eq!(n, 1);
        let state: String = idx
            .db()
            .query_row(
                "SELECT review_state FROM memories WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "suspect");
    }

    #[test]
    fn a_claim_whose_files_are_untouched_stays_accepted() {
        let (_t, idx, _id) = seed_accepted("abc123", &["src/foo.rs"]);
        let claims = anchored_claims(&idx, "/work/app").unwrap();
        // Something else changed, not our file.
        let changed = |_c: &str| Some(vec!["src/unrelated.rs".to_string()]);
        let report = evaluate(&claims, &changed);
        assert!(report.invalidated.is_empty());
        assert_eq!(report.still_fresh, 1);
    }

    #[test]
    fn an_unknowable_commit_neither_invalidates_nor_confirms() {
        // A rebase or force-push can leave the anchor commit unreachable.
        // We must not flip the claim to suspect (that punishes the user
        // for rewriting history) NOR count it fresh (that would let a
        // genuinely stale claim hide behind a lost commit).
        let (_t, idx, _id) = seed_accepted("deadbeef", &["src/foo.rs"]);
        let claims = anchored_claims(&idx, "/work/app").unwrap();
        let unknowable = |_c: &str| None;
        let report = evaluate(&claims, &unknowable);
        assert!(report.invalidated.is_empty());
        assert_eq!(report.still_fresh, 0, "unknowable is not fresh");
    }

    #[test]
    fn a_no_anchor_acceptance_is_never_a_candidate() {
        // Accepted with --no-anchor: no commit, so nothing to check.
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let claims = DistilledClaims {
            claims: vec![DistilledClaim {
                claim: "the user prefers tabs".into(),
                directive: "Indent with tabs.".into(),
                kind: "preference".into(),
                files: vec![],
                evidence: "".into(),
                confidence: 90,
            }],
        };
        ingest_proposals(
            &idx,
            &claims,
            &ProposalOrigin {
                project_path: "/work/app",
                file_path: None,
                exchange_id: None,
                created_by: "agent:distiller",
            },
            1_000,
        )
        .unwrap();
        let id: String = idx
            .db()
            .query_row("SELECT id FROM memories", [], |r| r.get(0))
            .unwrap();
        review::accept(&idx, &id, None, 2_000).unwrap();

        assert!(anchored_claims(&idx, "/work/app").unwrap().is_empty());
    }

    #[test]
    fn only_accepted_claims_are_candidates_not_proposed_ones() {
        // A proposal is anchored to files but has no commit and isn't
        // accepted; it must never be swept.
        let (_t, idx, id) = seed_accepted("abc", &["src/foo.rs"]);
        review::reject(&idx, &id, 3_000).unwrap();
        assert!(anchored_claims(&idx, "/work/app").unwrap().is_empty());
    }

    #[test]
    fn path_matching_respects_segment_boundaries() {
        // core.rs must not match score.rs.
        assert!(paths_match("crates/core/src/core.rs", "core.rs"));
        assert!(paths_match("core.rs", "crates/core/src/core.rs"));
        assert!(!paths_match("src/score.rs", "core.rs"));
        assert!(paths_match("./src/foo.rs", "src/foo.rs"));
        assert!(!paths_match("src/foobar.rs", "foo.rs"));
    }
}
