//! Ingest distilled claims into `memories` as **proposals**.
//!
//! # The seam
//!
//! This is the deterministic half of the harvester. The
//! knowledge-distiller agent emits JSON matching its schema; this
//! module parses it and writes rows. The model never calls a write
//! tool, and therefore never *forgets* to.
//!
//! That is not a stylistic preference. The MCP memory server has
//! shipped a `claudepot_remember` tool for months, the instruction
//! snippet tells agents to call it, and the `memories` table on a real
//! machine with 7,798 indexed exchanges contains **zero rows**. An
//! agent that *may* persist knowledge does not persist knowledge. So
//! the model's only job is to return a value; persistence is Rust's.
//!
//! # Proposals, not facts
//!
//! Everything written here lands as `review_state = 'proposed'`. It is
//! inert: no directive is compiled, no guard is emitted, nothing enters
//! an agent's context until a human accepts it. A wrong memory that
//! slips into context is worse than no memory, because it will be
//! trusted and it will be invisible.
//!
//! # What is deliberately dropped
//!
//! - **Low-confidence claims** (< [`MIN_CONFIDENCE`]). The distiller is
//!   told not to emit them; we enforce it rather than trust it.
//! - **Duplicates.** An identical claim already in the queue (or
//!   already accepted, or already *rejected*) is not re-filed. Without
//!   this, every settled session re-proposes the same lesson and the
//!   user re-rejects it forever — the queue becomes noise and the
//!   feature dies.
//! - **Anything that looks like copied transcript text.** The prompt
//!   forbids quoting; a claim is a *statement*, not an excerpt. Overly
//!   long content is a signal the model pasted rather than distilled.

use serde::{Deserialize, Serialize};

use crate::redaction::{apply as redact_apply, RedactionPolicy};
use crate::session_index::SessionIndex;
use crate::shared_memory::durable::{self, DurableError, MemoryKind, MemoryRecord};
use crate::shared_memory::recurrence;

/// Claims below this confidence are dropped without review. The
/// distiller's prompt says the same; belt and braces.
pub const MIN_CONFIDENCE: i64 = 60;

/// A claim longer than this was almost certainly pasted out of the
/// transcript rather than distilled from it. Drop it: the point of a
/// lesson is that it is shorter than the thing it was learned from.
pub const MAX_CLAIM_CHARS: usize = 600;

/// The distiller agent's `result.json` payload.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DistilledClaims {
    #[serde(default)]
    pub claims: Vec<DistilledClaim>,
}

/// One distilled lesson.
///
/// Only `claim` and `directive` are required — they are the two fields
/// with no sensible default, because a lesson with no statement or no
/// instruction is not a lesson. **Everything else defaults**, and that
/// is deliberate: a real Haiku run over a real transcript omitted `kind`
/// entirely, and a strict struct turned a perfectly good lesson into a
/// hard parse failure that took the other claims in the batch down with
/// it. Being strict about a field we can default is choosing to lose
/// data.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DistilledClaim {
    pub claim: String,
    pub directive: String,
    /// `pattern` when absent — the least load-bearing kind.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub evidence: String,
    /// Absent confidence is treated as "did not meet the bar" rather
    /// than "maximally sure". Fail closed: an unrated claim should not
    /// outrank one the model actually vouched for.
    #[serde(default)]
    pub confidence: i64,
}

/// Where a batch of claims came from. Provenance is denormalized onto
/// the memory row because `memory_links` does not survive a rebuild —
/// see `shared_memory::schema`.
#[derive(Debug, Clone)]
pub struct ProposalOrigin<'a> {
    pub project_path: &'a str,
    /// The transcript the lesson was learned from.
    pub file_path: Option<&'a str>,
    /// `<session_id>:<turn_index>`, when known.
    pub exchange_id: Option<&'a str>,
    /// Who produced it, e.g. `agent:knowledge-distiller`.
    pub created_by: &'a str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IngestReport {
    pub proposed: u32,
    pub skipped_low_confidence: u32,
    pub skipped_duplicate: u32,
    pub skipped_too_long: u32,
    pub skipped_empty: u32,
    /// New claims that matched an already-accepted/suspect lesson in this
    /// project — filed as pending recurrences for a human to confirm. This
    /// counts *detections*, independent of whether the claim was also filed
    /// as a fresh proposal or skipped as a duplicate.
    pub recurrences_detected: u32,
}

impl IngestReport {
    pub fn total_skipped(&self) -> u32 {
        self.skipped_low_confidence
            + self.skipped_duplicate
            + self.skipped_too_long
            + self.skipped_empty
    }
}

/// Parse a distiller run's `result.json` body.
///
/// Tolerant of every shape the harness actually produces, because a
/// parse failure here is indistinguishable from "the session taught us
/// nothing" — it fails *silently*, and a harvester that silently
/// harvests nothing is worse than one that crashes.
///
/// Observed in practice against a real Haiku run: the model returned
/// correct JSON wrapped in a ```json markdown fence. With
/// `--output-format json` + a `json_schema` it should not, but "should
/// not" is not a guarantee, and the cost of being wrong is a feature
/// that quietly does nothing.
pub fn parse_claims(raw: &str) -> Result<DistilledClaims, serde_json::Error> {
    let cleaned = strip_markdown_fence(raw.trim());

    // The strict path: the whole body is JSON.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
        if let Some(found) = from_value_shapes(&v)? {
            return Ok(found);
        }
    }

    // The forgiving path: the model wrapped its JSON in prose ("Based on
    // my examination of the transcript, here are the lessons: {…}").
    // Observed on a real run. Refusing to parse that is choosing to
    // throw away a correct answer because of its packaging.
    if let Some(obj) = first_json_object(cleaned) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(obj) {
            if let Some(found) = from_value_shapes(&v)? {
                return Ok(found);
            }
        }
    }

    // Nothing recognizable: an empty harvest, not an error. A distiller
    // that found no lessons is doing its job, and most sessions teach
    // nothing.
    Ok(DistilledClaims::default())
}

/// Recognize the payload in any of the shapes the harness produces.
/// `Ok(None)` = "this value isn't one of them", not "it's empty".
fn from_value_shapes(v: &serde_json::Value) -> Result<Option<DistilledClaims>, serde_json::Error> {
    // Shape 1: already the payload.
    if v.get("claims").is_some() {
        return serde_json::from_value(v.clone()).map(Some);
    }
    // Shape 2: an envelope with a `result` field.
    if let Some(inner) = v.get("result") {
        // 2a: result is the object itself.
        if inner.get("claims").is_some() {
            return serde_json::from_value(inner.clone()).map(Some);
        }
        // 2b: result is a JSON *string* holding the object — possibly
        // fenced, possibly wrapped in prose of its own.
        if let Some(s) = inner.as_str() {
            let s = strip_markdown_fence(s.trim());
            if let Ok(inner_v) = serde_json::from_str::<serde_json::Value>(s) {
                if inner_v.get("claims").is_some() {
                    return serde_json::from_value(inner_v).map(Some);
                }
            }
            if let Some(obj) = first_json_object(s) {
                let inner_v: serde_json::Value = serde_json::from_str(obj)?;
                if inner_v.get("claims").is_some() {
                    return serde_json::from_value(inner_v).map(Some);
                }
            }
        }
    }
    Ok(None)
}

/// The first balanced `{…}` in `s`, brace-counting outside of string
/// literals (so a `}` inside a claim's text doesn't end the object).
fn first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &c) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return s.get(start..=i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip a ```json … ``` wrapper if present. Returns the input
/// unchanged when there is no fence.
fn strip_markdown_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    // Drop the optional language tag on the opening fence line.
    let rest = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => return s,
    };
    rest.trim_end()
        .strip_suffix("```")
        .map(str::trim_end)
        .unwrap_or(rest)
}

/// Insert claims as proposals. Idempotent per claim: re-ingesting the
/// same distiller output is a no-op.
pub fn ingest_proposals(
    idx: &SessionIndex,
    claims: &DistilledClaims,
    origin: &ProposalOrigin<'_>,
    now_ms: i64,
) -> Result<IngestReport, DurableError> {
    let mut report = IngestReport::default();
    let policy = RedactionPolicy::default();

    // The lessons a new claim can recur against: this project's committed
    // (accepted/suspect) lessons. Fetched ONCE — every claim in the batch
    // shares `origin.project_path`, so re-querying per claim would be
    // wasted work. Empty on a project with no accepted lessons, in which
    // case detection is a no-op.
    let priors = recurrence::prior_lessons(idx, origin.project_path)?;

    for c in &claims.claims {
        let claim = c.claim.trim();
        let directive = c.directive.trim();
        if claim.is_empty() || directive.is_empty() {
            report.skipped_empty += 1;
            continue;
        }
        if c.confidence < MIN_CONFIDENCE {
            report.skipped_low_confidence += 1;
            continue;
        }
        if claim.chars().count() > MAX_CLAIM_CHARS {
            report.skipped_too_long += 1;
            continue;
        }

        // Defense in depth. The prompt forbids quoting the transcript,
        // but a prompt is a request, not a guarantee — and the one
        // thing we must never do is launder private content out of a
        // transcript and into a durable table that outlives it.
        let claim = redact_apply(claim, &policy);
        let directive = redact_apply(directive, &policy);
        let evidence = redact_apply(c.evidence.trim(), &policy);

        // Recurrence check BEFORE the dedup gate. This runs even when the
        // claim is an exact duplicate of an accepted lesson — an exact
        // re-derivation is the strongest possible recurrence signal, and
        // the dedup below would otherwise swallow it silently. `record`
        // is itself idempotent, so a re-harvest doesn't pile up events.
        if !priors.is_empty() {
            if let Some(m) = recurrence::detect_match(&c.files, &claim, &directive, &priors) {
                let filed = recurrence::record(
                    idx,
                    &recurrence::NewRecurrence {
                        matched_memory_id: &m.memory_id,
                        project_path: origin.project_path,
                        new_content: &claim,
                        new_exchange_id: origin.exchange_id,
                        new_file_path: origin.file_path,
                        detected_by: m.detected_by,
                    },
                    now_ms,
                )?;
                if filed.is_some() {
                    report.recurrences_detected += 1;
                }
            }
        }

        if is_already_known(idx, origin.project_path, &claim)? {
            report.skipped_duplicate += 1;
            continue;
        }

        let anchor = anchor_json(&c.files, &evidence);
        insert_proposal(
            idx,
            origin,
            &claim,
            &directive,
            parse_kind(&c.kind),
            c.confidence.clamp(0, 100),
            anchor.as_deref(),
            now_ms,
        )?;
        report.proposed += 1;
    }
    Ok(report)
}

/// Has this exact claim been seen before — in any review state?
///
/// Checking *every* state, not just `proposed`, is the point. A claim
/// the user already **rejected** must not come back: the distiller will
/// re-derive it from the same transcript on every future run, and a
/// queue that resurrects rejected items trains the user to stop looking
/// at it.
fn is_already_known(
    idx: &SessionIndex,
    project_path: &str,
    content: &str,
) -> Result<bool, DurableError> {
    match idx.db().query_row(
        "SELECT 1 FROM memories \
         WHERE content = ?1 AND (project_path = ?2 OR project_path IS NULL) LIMIT 1",
        rusqlite::params![content, project_path],
        |_| Ok(true),
    ) {
        Ok(found) => Ok(found),
        // Only "no such row" means "not a duplicate". A real SQL error
        // (locked DB, corruption) must NOT be swallowed as `false` — that
        // would file a duplicate proposal on every transient failure.
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(DurableError::from(e)),
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_proposal(
    idx: &SessionIndex,
    origin: &ProposalOrigin<'_>,
    claim: &str,
    directive: &str,
    kind: MemoryKind,
    confidence: i64,
    anchor: Option<&str>,
    _now_ms: i64,
) -> Result<MemoryRecord, DurableError> {
    // ONE atomic insert as `review_state = 'proposed'`. Going through
    // create_memory + a follow-up UPDATE would leave a crash window in
    // which the row is 'accepted' (create_memory's column default) and
    // has bypassed the human review gate. See durable::create_proposal.
    durable::create_proposal(
        idx,
        &durable::NewProposal {
            project_path: origin.project_path,
            kind,
            content: claim,
            directive,
            confidence,
            anchor_json: anchor,
            origin_exchange_id: origin.exchange_id,
            origin_file_path: origin.file_path,
            created_by: origin.created_by,
        },
    )
}

/// `{"files": [...], "evidence": "..."}`. The commit is stamped at
/// *acceptance*, not here: a proposal is anchored to the code it
/// describes, but it only becomes invalidatable once a human has
/// agreed it is true.
fn anchor_json(files: &[String], evidence: &str) -> Option<String> {
    if files.is_empty() && evidence.is_empty() {
        return None;
    }
    serde_json::to_string(&serde_json::json!({
        "files": files,
        "evidence": evidence,
    }))
    .ok()
}

fn parse_kind(raw: &str) -> MemoryKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "constraint" => MemoryKind::Constraint,
        "preference" => MemoryKind::Preference,
        "fact" => MemoryKind::Fact,
        // The distiller mines recurring shapes; when in doubt it is a
        // pattern, which is the least load-bearing of the kinds.
        _ => MemoryKind::Pattern,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn idx() -> (TempDir, SessionIndex) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        (tmp, idx)
    }

    fn origin<'a>() -> ProposalOrigin<'a> {
        ProposalOrigin {
            project_path: "/work/app",
            file_path: Some("/t/s.jsonl"),
            exchange_id: Some("s1:4"),
            created_by: "agent:knowledge-distiller",
        }
    }

    fn claim(text: &str, confidence: i64) -> DistilledClaim {
        DistilledClaim {
            claim: text.to_string(),
            directive: "Run scripts/preflight.sh before pushing.".to_string(),
            kind: "constraint".to_string(),
            files: vec!["scripts/preflight.sh".to_string()],
            evidence: "CI went red after a local run passed.".to_string(),
            confidence,
        }
    }

    #[test]
    fn an_ingested_claim_lands_as_proposed_never_accepted() {
        // The single most important assertion in this module. Anything
        // that lands `accepted` has bypassed the human.
        let (_t, idx) = idx();
        let claims = DistilledClaims {
            claims: vec![claim("preflight runs guards cargo test does not", 90)],
        };
        let r = ingest_proposals(&idx, &claims, &origin(), 1_000).unwrap();
        assert_eq!(r.proposed, 1);

        let (state, directive, exch): (String, Option<String>, Option<String>) = idx
            .db()
            .query_row(
                "SELECT review_state, directive, origin_exchange_id FROM memories",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "proposed");
        assert!(directive.unwrap().contains("preflight.sh"));
        assert_eq!(exch.as_deref(), Some("s1:4"), "provenance is denormalized");
    }

    #[test]
    fn a_rejected_claim_is_never_re_proposed() {
        // The distiller re-derives the same lesson from the same
        // transcript on every run. A queue that resurrects what the
        // user already threw away is a queue the user stops opening.
        let (_t, idx) = idx();
        let claims = DistilledClaims {
            claims: vec![claim("some lesson", 90)],
        };
        ingest_proposals(&idx, &claims, &origin(), 1_000).unwrap();
        idx.db()
            .execute("UPDATE memories SET review_state = 'rejected'", [])
            .unwrap();

        let r = ingest_proposals(&idx, &claims, &origin(), 2_000).unwrap();
        assert_eq!(r.proposed, 0);
        assert_eq!(r.skipped_duplicate, 1);

        let n: i64 = idx
            .db()
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "the rejected claim must not be re-filed");
    }

    #[test]
    fn re_ingesting_the_same_run_is_idempotent() {
        let (_t, idx) = idx();
        let claims = DistilledClaims {
            claims: vec![claim("a lesson", 90)],
        };
        ingest_proposals(&idx, &claims, &origin(), 1_000).unwrap();
        let r = ingest_proposals(&idx, &claims, &origin(), 1_001).unwrap();
        assert_eq!(r.proposed, 0);
        assert_eq!(r.skipped_duplicate, 1);
    }

    #[test]
    fn low_confidence_claims_are_dropped_not_queued() {
        let (_t, idx) = idx();
        let claims = DistilledClaims {
            claims: vec![claim("shaky", MIN_CONFIDENCE - 1)],
        };
        let r = ingest_proposals(&idx, &claims, &origin(), 1).unwrap();
        assert_eq!(r.proposed, 0);
        assert_eq!(r.skipped_low_confidence, 1);
    }

    #[test]
    fn a_claim_that_looks_pasted_is_dropped() {
        // A lesson is shorter than the thing it was learned from. A
        // 600+ char "claim" is the model pasting the transcript back.
        let (_t, idx) = idx();
        let claims = DistilledClaims {
            claims: vec![claim(&"x".repeat(MAX_CLAIM_CHARS + 1), 95)],
        };
        let r = ingest_proposals(&idx, &claims, &origin(), 1).unwrap();
        assert_eq!(r.proposed, 0);
        assert_eq!(r.skipped_too_long, 1);
    }

    #[test]
    fn a_secret_that_slips_into_a_claim_is_redacted_before_it_is_stored() {
        // The prompt forbids quoting the transcript. A prompt is a
        // request, not a guarantee — and a durable table outlives the
        // transcript it launders content out of.
        let (_t, idx) = idx();
        let mut c = claim("the key sk-ant-oat01-AAAABBBBCCCCDDDD unblocked it", 95);
        c.directive = "export TOKEN=sk-ant-oat01-AAAABBBBCCCCDDDD".to_string();
        let claims = DistilledClaims { claims: vec![c] };

        ingest_proposals(&idx, &claims, &origin(), 1).unwrap();
        let (content, directive): (String, String) = idx
            .db()
            .query_row("SELECT content, directive FROM memories", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(!content.contains("sk-ant-oat01-AAAABBBBCCCCDDDD"));
        assert!(!directive.contains("sk-ant-oat01-AAAABBBBCCCCDDDD"));
    }

    #[test]
    fn an_empty_harvest_is_a_success_not_an_error() {
        // Most sessions teach nothing. The distiller is told an empty
        // list is correct; ingest must agree, or every quiet session
        // looks like a failure.
        let (_t, idx) = idx();
        let r = ingest_proposals(&idx, &DistilledClaims::default(), &origin(), 1).unwrap();
        assert_eq!(r, IngestReport::default());
    }

    // ─── parsing the agent's actual output shape ────────────────

    #[test]
    fn parses_the_bare_payload() {
        let p = parse_claims(r#"{"claims":[]}"#).unwrap();
        assert!(p.claims.is_empty());
    }

    #[test]
    fn parses_the_claude_p_envelope_with_an_object_result() {
        let raw = r#"{"type":"result","result":{"claims":[{"claim":"c","directive":"d",
                     "kind":"pattern","evidence":"e","confidence":80}]}}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
    }

    #[test]
    fn parses_the_claude_p_envelope_with_a_stringified_result() {
        // `claude -p --output-format json` often hands back the model's
        // JSON as a *string* inside the envelope. Handle it here rather
        // than making every caller guess.
        let inner = r#"{"claims":[{"claim":"c","directive":"d","kind":"fact","evidence":"e","confidence":70}]}"#;
        let raw = serde_json::json!({ "type": "result", "result": inner }).to_string();
        let p = parse_claims(&raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].confidence, 70);
    }

    #[test]
    fn unrecognizable_output_is_an_empty_harvest_not_a_crash() {
        let p = parse_claims(r#"{"type":"result","result":"I could not read the file"}"#);
        assert!(p.is_err() || p.unwrap().claims.is_empty());
    }

    #[test]
    fn parses_json_wrapped_in_a_markdown_fence() {
        // Not hypothetical: a real Haiku run against a real 6.9 MB
        // transcript returned exactly this. A parse failure here is
        // indistinguishable from "nothing was learned" — it fails
        // silently, which is the worst way for a harvester to fail.
        let raw = "```json\n{\"claims\":[{\"claim\":\"c\",\"directive\":\"d\",\
                   \"kind\":\"constraint\",\"evidence\":\"e\",\"confidence\":92}]}\n```";
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].confidence, 92);
    }

    #[test]
    fn parses_a_fenced_payload_nested_inside_the_envelope() {
        let fenced = "```json\n{\"claims\":[]}\n```";
        let raw = serde_json::json!({ "type": "result", "result": fenced }).to_string();
        assert!(parse_claims(&raw).unwrap().claims.is_empty());
    }

    #[test]
    fn an_unfenced_payload_is_untouched() {
        assert_eq!(strip_markdown_fence(r#"{"claims":[]}"#), r#"{"claims":[]}"#);
    }
}

/// End-to-end against the **actual output of a real Haiku run**.
///
/// The payload below is copied verbatim from a distiller run against a
/// 6.9 MB transcript of this repo (markdown fence and all). It is the
/// only test here that proves the whole chain — model output → parse →
/// filter → proposal row — on something the model really produced,
/// rather than on a fixture written by the same person who wrote the
/// parser.
#[cfg(test)]
mod real_run {
    use super::*;
    use tempfile::TempDir;

    const REAL_HAIKU_OUTPUT: &str = r#"```json
{
  "claims": [
    {
      "claim": "Adding new error enum variants creates exhaustive-match violations in platform-specific code; Windows-gated match sites on macOS cannot be compile-tested locally and will fail on the CI runner.",
      "directive": "When adding a new error variant, audit all match sites (especially under `#[cfg(target_os = \"windows\")]`) before pushing.",
      "kind": "constraint",
      "files": ["crates/claudepot-core/src/desktop_backend/crypto.rs"],
      "evidence": "Windows CI build failed with a non-exhaustive pattern error after a batch added a variant to the error enum.",
      "confidence": 92
    }
  ]
}
```"#;

    #[test]
    fn a_real_distiller_run_becomes_a_reviewable_proposal() {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();

        let claims = parse_claims(REAL_HAIKU_OUTPUT).expect("real output must parse");
        assert_eq!(claims.claims.len(), 1);

        let origin = ProposalOrigin {
            project_path: "/work/app",
            file_path: Some("/t/b1adfd71.jsonl"),
            exchange_id: None,
            created_by: "agent:knowledge-distiller",
        };
        let report = ingest_proposals(&idx, &claims, &origin, 1_700_000_000_000).unwrap();
        assert_eq!(report.proposed, 1);

        let (state, kind, directive, anchor): (String, String, String, String) = idx
            .db()
            .query_row(
                "SELECT review_state, kind, directive, anchor_json FROM memories",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();

        // Inert until a human says otherwise.
        assert_eq!(state, "proposed");
        // The model's `kind` survived the round trip.
        assert_eq!(kind, "constraint");
        // The directive is imperative and names something concrete —
        // this is the ETH finding made mechanical.
        assert!(directive.starts_with("When adding"));
        // Anchored to a file, so Phase 3 can invalidate it when that
        // file changes.
        assert!(anchor.contains("crypto.rs"));
    }
}

/// Regression tests for shapes a REAL distiller run actually produced.
///
/// Every case here was a live failure, not a hypothetical. The harvest
/// of three real transcripts failed 3/3 on its first run: two returned
/// JSON wrapped in prose, one omitted `kind`. A strict parser turned
/// correct lessons into nothing at all — and, worse, did it *silently
/// enough* to look like "this session taught us nothing".
#[cfg(test)]
mod observed_failures {
    use super::*;

    #[test]
    fn json_wrapped_in_prose_is_still_parsed() {
        let raw = "Based on my examination of the transcript, I found evidence of \
                   specific failures. Here are the durable lessons:\n\n\
                   {\"claims\":[{\"claim\":\"c\",\"directive\":\"d\",\"kind\":\"constraint\",\
                   \"evidence\":\"e\",\"confidence\":92}]}\n\nLet me know if you want more.";
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].confidence, 92);
    }

    #[test]
    fn a_claim_missing_kind_defaults_instead_of_failing_the_whole_batch() {
        // The real cost of strictness: one claim without `kind` took
        // every other claim in the same run down with it.
        let raw = r#"{"claims":[
            {"claim":"a","directive":"d1","evidence":"e","confidence":80},
            {"claim":"b","directive":"d2","kind":"constraint","evidence":"e","confidence":90}
        ]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 2, "one weak field must not lose the batch");
        assert_eq!(p.claims[0].kind, "");
    }

    #[test]
    fn a_claim_without_confidence_fails_closed() {
        // Absent confidence must not read as "maximally sure". An
        // unrated claim should not outrank one the model vouched for.
        let raw = r#"{"claims":[{"claim":"c","directive":"d"}]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims[0].confidence, 0);
        assert!(p.claims[0].confidence < MIN_CONFIDENCE);
    }

    #[test]
    fn a_brace_inside_a_claim_does_not_truncate_the_object() {
        // Brace-counting has to respect string literals, or a lesson
        // that mentions `#[cfg(...)]` or a JSON snippet cuts its own
        // object short and the parse silently loses everything after it.
        let raw = "here you go: {\"claims\":[{\"claim\":\"use match { arm => x }\",\
                   \"directive\":\"d\",\"kind\":\"pattern\",\"evidence\":\"e\",\
                   \"confidence\":75}]}";
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert!(p.claims[0].claim.contains("arm => x"));
    }

    #[test]
    fn prose_with_no_json_at_all_is_an_empty_harvest() {
        let p = parse_claims("I could not read that transcript.").unwrap();
        assert!(p.claims.is_empty());
    }
}
