//! Ingest distilled claims into `memories` as **proposals**.
//!
//! # The seam
//!
//! This is the deterministic half of the harvester. The
//! knowledge-distiller agent emits JSON matching its schema; this
//! module parses it and writes rows. The model never calls a write
//! tool, and therefore never *forgets* to.
//!
//! That is not a stylistic preference. The MCP memory server shipped a
//! `claudepot_remember` tool, and the instruction snippet has told
//! agents to call it since v2 — yet for months the `memories` table on
//! a real machine sat at **zero rows** while thousands of exchanges
//! piled up. That is what motivated this module.
//!
//! The ratio since has stayed lopsided the same way. On that machine at
//! 9,670 indexed exchanges: 8 of 10 memories came from this
//! deterministic path (`cli:lesson-harvest` and
//! `agent:knowledge-distiller`), 2 from an agent electing to call
//! `claudepot_remember`. An agent that *may* persist knowledge
//! sometimes does — but far too rarely to build on. So the model's only
//! job is to return a value; persistence is Rust's.
//!
//! (Figures are a dated observation, not an invariant. They are here to
//! show the shape of the gap, and re-measuring is a `sqlite3` query
//! against `~/.claudepot/sessions.db` — not a reason to rewrite this
//! doc on every harvest.)
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
    /// Elements of the `claims` array that could not be parsed into a
    /// [`DistilledClaim`] and were dropped individually.
    ///
    /// A parse artifact, not part of the model's schema — hence
    /// `#[serde(skip)]`. It exists so that "the model returned garbage"
    /// and "the session taught us nothing" stop looking identical to the
    /// caller. Both used to surface as an empty harvest.
    #[serde(skip)]
    pub malformed: u32,
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
///
/// The optional fields additionally deserialize **leniently** — a model
/// that returns `files` as a bare string, `confidence` as `"85"`, or
/// `evidence` as an object gets coerced rather than rejected. `claim`
/// and `directive` stay strict, because they are the two fields whose
/// meaning cannot survive coercion: stringifying an object into a
/// "lesson" manufactures a claim a human is then asked to approve.
/// A claim that fails here is dropped alone and counted in
/// [`DistilledClaims::malformed`]; its siblings still land.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DistilledClaim {
    pub claim: String,
    pub directive: String,
    /// `pattern` when absent — the least load-bearing kind.
    #[serde(default, deserialize_with = "lenient_string")]
    pub kind: String,
    #[serde(default, deserialize_with = "lenient_string_list")]
    pub files: Vec<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub evidence: String,
    /// Absent confidence is treated as "did not meet the bar" rather
    /// than "maximally sure". Fail closed: an unrated claim should not
    /// outrank one the model actually vouched for.
    #[serde(default, deserialize_with = "lenient_i64")]
    pub confidence: i64,
}

/// Render a scalar as a string; anything structural becomes empty.
///
/// Deliberately narrower than goose's equivalent
/// (`context_mgmt::structured::stringify_lenient`, which flattens
/// objects into `"k: v; k: v"`). That is right for a compaction summary,
/// where degraded prose still helps the model continue. It is wrong
/// here: these values feed a durable row a human reviews, and a
/// flattened object reads like a sentence the distiller never wrote.
fn scalar_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn lenient_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(scalar_to_string(&value))
}

/// `files` as a bare string is the shape a model produces when it
/// over-applies "one path per lesson". Treat it as a one-element list
/// rather than losing the anchor — the anchor is what makes
/// invalidation possible later.
fn lenient_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(scalar_to_string)
            .filter(|s| !s.trim().is_empty())
            .collect(),
        serde_json::Value::Null => Vec::new(),
        other => {
            let s = scalar_to_string(&other);
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
    })
}

/// `confidence` as `"85"` or `85.0`. An unparseable value falls to 0,
/// which fails closed against [`MIN_CONFIDENCE`].
fn lenient_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64)),
        _ => None,
    }
    .unwrap_or(0))
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
    /// Elements of the `claims` array the parser could not read.
    ///
    /// Deliberately NOT part of [`total_skipped`](Self::total_skipped):
    /// the other counters are claims we understood and declined by
    /// policy, which is the harvester working. This one is the model or
    /// the schema misbehaving, and it should read as a defect signal,
    /// not as routine filtering.
    pub malformed_claims: u32,
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
/// The `Result` is retained for source compatibility with callers that
/// already `?` it. Nothing in this path errors any more: a body we
/// cannot recognize is an empty harvest, and an individual claim we
/// cannot parse is counted in [`DistilledClaims::malformed`] rather than
/// taking its siblings down with it.
pub fn parse_claims(raw: &str) -> Result<DistilledClaims, serde_json::Error> {
    Ok(best_payload(strip_markdown_fence(raw.trim()), 0).unwrap_or_default())
}

/// How deep to follow `result`-in-`result` nesting before giving up.
/// Real envelopes nest once; the bound only exists so a pathological
/// self-similar body cannot recurse without end.
const MAX_ENVELOPE_DEPTH: u8 = 4;

/// The best claims payload reachable from `text`, or `None` if no
/// candidate is a claims document at all.
///
/// Every balanced top-level `{…}` is a candidate, and we keep the one
/// with the MOST claims, tie-broken by the LAST occurrence. Taking the
/// *first* parseable object — the previous behavior — lets a schema
/// example echoed in the model's preamble ("I'll return
/// `{\"claims\": []}`…") shadow the real answer that follows it. Goose
/// hits the same hazard in `context_mgmt::structured::json_candidates`
/// and solves it by trying the last fence first; ranking by claim count
/// additionally survives the reverse ordering, where the answer comes
/// first and an illustration trails it.
///
/// Scanning for braces rather than parsing the whole body strictly also
/// means a JSON *array* of CC events is handled for free: each element
/// is its own candidate, so the `result` event is found among them.
fn best_payload(text: &str, depth: u8) -> Option<DistilledClaims> {
    let mut best: Option<DistilledClaims> = None;
    for candidate in balanced_objects(text) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        let Some(found) = payload_from_value(&v, depth) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|b| found.claims.len() >= b.claims.len())
        {
            best = Some(found);
        }
    }
    best
}

/// Recognize the payload in any of the shapes the harness produces.
/// `None` = "this value isn't one of them", not "it's empty".
fn payload_from_value(v: &serde_json::Value, depth: u8) -> Option<DistilledClaims> {
    // Shape 1: already the payload.
    if let Some(arr) = v.get("claims") {
        return Some(claims_from_array(arr));
    }
    if depth >= MAX_ENVELOPE_DEPTH {
        return None;
    }
    // Shape 1b: CC's `result` event carries `structured_output` — the
    // payload already parsed for us — whenever the run passed
    // `--json-schema`, which `distill::distiller_flags` now does.
    // Preferred over `result` below: same content, one less string
    // re-parse, and it is the field CC validated against the schema.
    // Verified against CC 2.1.220 output (see `real_run`).
    if let Some(structured) = v.get("structured_output") {
        if structured.get("claims").is_some() {
            return payload_from_value(structured, depth + 1);
        }
    }
    // Shape 2: an envelope with a `result` field.
    if let Some(inner) = v.get("result") {
        // 2a: result is the object itself.
        if inner.get("claims").is_some() {
            return payload_from_value(inner, depth + 1);
        }
        // 2b: result is a JSON *string* holding the object — possibly
        // fenced, possibly wrapped in prose of its own.
        if let Some(s) = inner.as_str() {
            return best_payload(strip_markdown_fence(s.trim()), depth + 1);
        }
    }
    None
}

/// Parse the `claims` array one element at a time.
///
/// The whole point of this function: `serde_json::from_value` over the
/// *array* rejects the entire batch when a single element is malformed.
/// That is the failure class behind the defaults on [`DistilledClaim`] —
/// a real Haiku run omitted `kind` and took every sibling claim with it.
/// Defaulting one field fixed one symptom; parsing per element fixes the
/// class.
fn claims_from_array(arr: &serde_json::Value) -> DistilledClaims {
    let mut out = DistilledClaims::default();
    let Some(items) = arr.as_array() else {
        // `claims` present but not an array. Still a claims document —
        // just an unusable one, so the caller stops looking.
        return out;
    };
    for item in items {
        match serde_json::from_value::<DistilledClaim>(item.clone()) {
            Ok(c) => out.claims.push(c),
            Err(_) => out.malformed += 1,
        }
    }
    out
}

/// Every balanced `{…}` in `s`, brace-counting outside of string
/// literals (so a `}` inside a claim's text doesn't end the object).
/// Nested objects are not returned separately — after a match, scanning
/// resumes past its close.
fn balanced_objects(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Unterminated from here means nothing later can balance either.
        let Some(end) = balanced_end(bytes, i) else {
            break;
        };
        if let Some(slice) = s.get(i..=end) {
            out.push(slice);
        }
        i = end + 1;
    }
    out
}

/// Index of the `}` closing the `{` at `start`, or `None` if unbalanced.
fn balanced_end(bytes: &[u8], start: usize) -> Option<usize> {
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
                    return Some(i);
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
    let mut report = IngestReport {
        malformed_claims: claims.malformed,
        ..IngestReport::default()
    };
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
        let claims = DistilledClaims {
            claims: vec![c],
            ..Default::default()
        };

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

    // ─── one bad claim must not sink the batch ──────────────────

    /// The regression this whole change exists for. Previously
    /// `serde_json::from_value` ran over the *array*, so a single
    /// unreadable element discarded every good claim beside it — and
    /// the caller could not tell that from "this session taught us
    /// nothing".
    #[test]
    fn a_malformed_claim_is_dropped_alone_not_with_its_siblings() {
        let raw = r#"{"claims":[
            {"claim":"a","directive":"d","kind":"fact","evidence":"e","confidence":90},
            {"claim":{"nested":"object"},"directive":"d","confidence":90},
            {"claim":"b","directive":"d","kind":"fact","evidence":"e","confidence":90}
        ]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 2, "both good claims survive");
        assert_eq!(p.claims[0].claim, "a");
        assert_eq!(p.claims[1].claim, "b");
        assert_eq!(p.malformed, 1);
    }

    #[test]
    fn a_claim_missing_a_required_field_is_counted_not_fatal() {
        // `directive` absent: not a lesson, but its siblings are.
        let raw = r#"{"claims":[
            {"claim":"only a statement","kind":"fact","confidence":90},
            {"claim":"a","directive":"d","kind":"fact","evidence":"e","confidence":90}
        ]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.malformed, 1);
    }

    #[test]
    fn the_malformed_count_reaches_the_ingest_report() {
        // The counter is worthless if it dies at the parse boundary.
        let (_t, idx) = idx();
        let claims = DistilledClaims {
            claims: vec![claim("a real lesson worth filing", 90)],
            malformed: 3,
        };
        let r = ingest_proposals(&idx, &claims, &origin(), 1).unwrap();
        assert_eq!(r.proposed, 1);
        assert_eq!(r.malformed_claims, 3);
        // A parse defect is not a policy skip — keep them distinct.
        assert_eq!(r.total_skipped(), 0);
    }

    // ─── lenient optional fields ────────────────────────────────

    #[test]
    fn files_given_as_a_bare_string_becomes_a_one_element_anchor() {
        // Losing this silently would cost the lesson its invalidation
        // anchor, which is the thing that makes it expire correctly.
        let raw = r#"{"claims":[{"claim":"a","directive":"d","kind":"fact",
                     "evidence":"e","confidence":90,"files":"src/parser.rs"}]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims[0].files, vec!["src/parser.rs"]);
        assert_eq!(p.malformed, 0);
    }

    #[test]
    fn confidence_given_as_a_string_is_coerced() {
        let raw = r#"{"claims":[{"claim":"a","directive":"d","kind":"fact",
                     "evidence":"e","confidence":"85"}]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims[0].confidence, 85);
    }

    #[test]
    fn an_unreadable_confidence_fails_closed_below_the_bar() {
        let raw = r#"{"claims":[{"claim":"a","directive":"d","kind":"fact",
                     "evidence":"e","confidence":"very sure"}]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims[0].confidence, 0);
        assert!(p.claims[0].confidence < MIN_CONFIDENCE);
    }

    #[test]
    fn a_structural_evidence_value_does_not_sink_the_claim() {
        // Coerced to empty rather than flattened into a fake sentence —
        // `evidence` is shown to a human deciding whether to accept.
        let raw = r#"{"claims":[{"claim":"a","directive":"d","kind":"fact",
                     "evidence":{"was":"a nested object"},"confidence":90}]}"#;
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].evidence, "");
        assert_eq!(p.malformed, 0);
    }

    // ─── candidate selection ────────────────────────────────────

    #[test]
    fn an_echoed_schema_example_does_not_shadow_the_real_answer() {
        // Taking the FIRST balanced object — the old behavior — returned
        // the empty example and reported a silent empty harvest.
        let raw = "I'll return {\"claims\": []} if nothing is found. Here is the result:\n\
                   {\"claims\":[{\"claim\":\"a\",\"directive\":\"d\",\"kind\":\"fact\",\
                   \"evidence\":\"e\",\"confidence\":90}]}";
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].claim, "a");
    }

    #[test]
    fn a_trailing_illustration_does_not_beat_the_real_answer() {
        // The reverse ordering: ranking by claim count (not just "last")
        // is what survives this one.
        let raw = "{\"claims\":[{\"claim\":\"a\",\"directive\":\"d\",\"kind\":\"fact\",\
                   \"evidence\":\"e\",\"confidence\":90}]}\n\
                   For reference the empty shape is {\"claims\": []}.";
        let p = parse_claims(raw).unwrap();
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].claim, "a");
    }

    #[test]
    fn a_result_event_inside_a_json_array_of_events_is_found() {
        // `claude -p --output-format json` is now used by the manual
        // distill path too; an array-of-events envelope must not read as
        // an empty harvest.
        let inner = r#"{"claims":[{"claim":"a","directive":"d","kind":"fact","evidence":"e","confidence":90}]}"#;
        let raw = serde_json::json!([
            {"type": "system", "subtype": "init"},
            {"type": "result", "result": inner},
        ])
        .to_string();
        let p = parse_claims(&raw).unwrap();
        assert_eq!(p.claims.len(), 1);
    }

    #[test]
    fn an_unterminated_object_is_not_repaired() {
        // Output cut off mid-JSON. Guessing at the missing tail would
        // file half a lesson as if it were whole.
        let p = parse_claims(r#"{"claims":[{"claim":"a","directive":"d"#).unwrap();
        assert!(p.claims.is_empty());
        assert_eq!(p.malformed, 0);
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

    /// The envelope `claude -p --output-format json --json-schema …`
    /// really produces, captured verbatim from CC **2.1.220** on
    /// 2026-07-25 — the flags `distill::distiller_flags` now sends.
    ///
    /// Trimmed to two of the fifteen events (a `system`/`status` and the
    /// terminal `result`); both are byte-for-byte as CC emitted them.
    /// The dropped events are `system`/`init`, the assistant turns, and
    /// a `rate_limit_event` — none of which the parser looks at, and one
    /// of which carries a multi-kilobyte thinking signature.
    ///
    /// Three properties of the real shape this locks down, none of which
    /// the pre-existing fixtures covered:
    ///   1. stdout is a JSON **array** of events, not one object;
    ///   2. the payload arrives as a *string* in `result`;
    ///   3. it also arrives pre-parsed in `structured_output`.
    const REAL_SCHEMA_ENVELOPE: &str = r#"[{"type":"system","subtype":"status","status":null,"permissionMode":"default","uuid":"71e0813a-792f-4432-abd2-c8c99aa0e348","session_id":"b5121469-85f5-4eae-8342-675212ad0f3f"},{"is_error":false,"duration_api_ms":6242,"num_turns":2,"stop_reason":"tool_use","session_id":"b5121469-85f5-4eae-8342-675212ad0f3f","total_cost_usd":0.064632,"subtype":"success","api_error_status":null,"result":"{\"claims\":[{\"claim\":\"the sky is blue\",\"directive\":\"Assume daylight.\",\"kind\":\"fact\",\"evidence\":\"observed\",\"confidence\":90}]}","structured_output":{"claims":[{"claim":"the sky is blue","directive":"Assume daylight.","kind":"fact","evidence":"observed","confidence":90}]},"type":"result","duration_ms":8320,"uuid":"4d50fd08-60f0-4d66-928f-717a101dd9e5"}]"#;

    #[test]
    fn the_real_schema_envelope_parses() {
        let p = parse_claims(REAL_SCHEMA_ENVELOPE).expect("real CC output must parse");
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].claim, "the sky is blue");
        assert_eq!(p.claims[0].kind, "fact");
        assert_eq!(p.claims[0].confidence, 90);
        assert_eq!(p.malformed, 0);
    }

    /// `structured_output` must be what we read — not the `result`
    /// string that sits beside it. Poison the string: if the assertion
    /// still sees the good claim, the preferred field won.
    #[test]
    fn structured_output_is_preferred_over_the_result_string() {
        let poisoned = REAL_SCHEMA_ENVELOPE.replace(
            r#"\"claim\":\"the sky is blue\""#,
            r#"\"claim\":\"STALE STRING COPY\""#,
        );
        assert!(poisoned.contains("STALE STRING COPY"), "fixture edited");
        let p = parse_claims(&poisoned).expect("must parse");
        assert_eq!(p.claims.len(), 1);
        assert_eq!(p.claims[0].claim, "the sky is blue");
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
