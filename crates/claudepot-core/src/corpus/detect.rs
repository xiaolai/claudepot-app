//! Tiers 1–3 — deterministic candidate detection over the corpus.
//!
//! No model calls. Everything here is a SQL read plus pure comparison,
//! and the output is deliberately **not lessons**: it is ranked, evidence-
//! bearing candidates that a model (Tier 4) or a human classifies later.
//!
//! # Precision is the whole problem
//!
//! Scale is not the difficulty. On a corpus a fraction of this size, the
//! naive Tier-3 join — "an error, then any later success of the same
//! tool in the same session" — produced **358,043 pairs**. Useless. Every
//! constraint below exists to cut that:
//!
//! - same session *and* same physical file;
//! - same **command family**, not merely the same tool
//!   ([`super::normalize::command_family`] — a failed `cargo test` is not
//!   recovered by a successful `cd`);
//! - the **first** success of that family after the failure, not any;
//! - bounded by [`MAX_RECOVERY_TURN_GAP`], because a success forty turns
//!   later is a different piece of work.
//!
//! # Vocabulary
//!
//! Nothing here is a "recurrence" — that word has a precise, human-
//! confirmed meaning in [`crate::shared_memory::recurrence`] (an
//! *accepted lesson's* failure class happening again) and diluting it
//! would break the one honest signal the knowledge base has. Repetition
//! is a *repetition cluster*; a failure with no observed recovery is
//! `unresolved`, never "abandoned" or "failed" — a session that simply
//! ends tells us nothing about why.

use super::normalize::{
    command_family, error_signature, fingerprint, is_harness_synthetic, jaccard, normalize_prompt,
};
use super::{CorpusError, CorpusIndex};
use crate::session_live::redact::redact_secrets;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How far after a failure a success still counts as its recovery.
/// Beyond this the pair is coincidence, not repair.
pub const MAX_RECOVERY_TURN_GAP: i64 = 12;

/// Minimum occurrences before a repetition cluster is worth surfacing.
pub const MIN_REPETITION_COUNT: usize = 5;

/// Minimum distinct projects for a repetition cluster to read as a
/// *reusable* pattern rather than one project's local habit.
pub const MIN_REPETITION_PROJECTS: usize = 2;

/// Similarity at which two user turns count as "asked again".
pub const REPEAT_INTENT_THRESHOLD: f64 = 0.6;

/// How far back a correction looks for the intent it is correcting.
pub const CORRECTION_LOOKBACK_TURNS: i64 = 3;

/// What kind of evidence backs a candidate. Mirrors the evidence grades
/// the Review surface will show, so the UI never has to invent one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// A failure followed by an observed success of the same family.
    VerifiedRecovery,
    /// A user correction, corroborated by a failure or a repeated ask.
    UserCorrection,
    /// The same normalized request, many times, across projects.
    RepeatedIncident,
}

/// A failure and, when one was observed, its recovery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    pub session_id: String,
    pub project_path: String,
    pub file_path: String,
    pub family: String,
    pub error_signature: String,
    pub failure_turn: i64,
    /// `None` when no qualifying success followed. Reported as
    /// `unresolved`, never as "abandoned".
    pub recovery_turn: Option<i64>,
    pub tool_name: String,
    /// The user turn that preceded the failure, if any — the intent.
    pub intent: Option<String>,
}

impl Incident {
    pub fn is_resolved(&self) -> bool {
        self.recovery_turn.is_some()
    }
}

/// The same normalized request, repeated. **Not** a lesson: this says
/// "you did this a lot", which is an automation or instruction
/// candidate, and says nothing about whether it went well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepetitionCluster {
    pub normalized: String,
    /// A representative example, for display — the original wording
    /// rather than the normalized form, but **redacted**.
    ///
    /// This crosses an emission boundary: `claudepot corpus detect`
    /// prints it to stdout and serializes it under `--json`. A prompt
    /// that repeats often enough to cluster is exactly the kind a user
    /// pastes a token into, so it is redacted at construction rather
    /// than trusted to every future caller (invariant R9,
    /// `scripts/repo-invariants.sh`).
    pub sample: String,
    pub count: usize,
    pub projects: usize,
    pub sessions: usize,
}

/// A user correction with something corroborating it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Correction {
    pub session_id: String,
    pub project_path: String,
    pub turn_index: i64,
    /// The correction as written, **redacted** — same emission-boundary
    /// reasoning as [`RepetitionCluster::sample`].
    pub text: String,
    /// Why this survived the marker filter.
    pub corroboration: Corroboration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corroboration {
    /// A tool call failed in the turns just before the correction.
    PrecedingFailure,
    /// The user had asked for materially the same thing just before.
    RepeatedIntent,
}

/// Everything one detection pass found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Findings {
    pub incidents: Vec<Incident>,
    pub repetitions: Vec<RepetitionCluster>,
    pub corrections: Vec<Correction>,
}

impl Findings {
    pub fn resolved_incidents(&self) -> usize {
        self.incidents.iter().filter(|i| i.is_resolved()).count()
    }
}

// ─── Tier 3: failure → recovery ──────────────────────────────────────

#[derive(Debug)]
struct RawCall {
    file_path: String,
    session_id: String,
    project_path: String,
    turn_index: i64,
    ordinal: i64,
    tool_name: String,
    input: Option<String>,
    result: Option<String>,
    is_error: bool,
}

/// Detect failure→recovery incidents.
///
/// One pass over tool calls ordered by (file, turn, ordinal). For each
/// failure, scan forward for the first success sharing its command
/// family within [`MAX_RECOVERY_TURN_GAP`] turns of the same file.
pub fn detect_incidents(idx: &CorpusIndex, limit: usize) -> Result<Vec<Incident>, CorpusError> {
    let db = idx.conn();
    let mut stmt = db.prepare(
        "SELECT tc.file_path, e.session_id, s.project_path, e.turn_index, tc.ordinal, \
                tc.tool_name, tc.tool_input_json, tc.tool_result_text, tc.is_error \
           FROM corpus_tool_calls tc \
           JOIN corpus_exchanges e ON e.id = tc.exchange_id \
           JOIN corpus_sessions  s ON s.session_id = e.session_id \
          ORDER BY tc.file_path, e.turn_index, tc.ordinal",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RawCall {
            file_path: r.get(0)?,
            session_id: r.get(1)?,
            project_path: r.get(2)?,
            turn_index: r.get(3)?,
            ordinal: r.get(4)?,
            tool_name: r.get(5)?,
            input: r.get(6)?,
            result: r.get(7)?,
            is_error: r.get::<_, i64>(8)? != 0,
        })
    })?;

    // Group by physical file — the unit a recovery must happen within.
    let mut by_file: HashMap<String, Vec<RawCall>> = HashMap::new();
    for row in rows {
        let c = row?;
        by_file.entry(c.file_path.clone()).or_default().push(c);
    }

    let mut out = Vec::new();
    for calls in by_file.values() {
        for (i, fail) in calls.iter().enumerate() {
            if !fail.is_error {
                continue;
            }
            let Some(sig) = fail.result.as_deref().and_then(error_signature) else {
                // Unclassifiable failure. Dropping it beats grouping
                // every such failure under one empty key.
                continue;
            };
            let family = command_family(&fail.tool_name, fail.input.as_deref());

            // First success of the same family, within the window.
            let recovery = calls[i + 1..]
                .iter()
                .take_while(|c| c.turn_index - fail.turn_index <= MAX_RECOVERY_TURN_GAP)
                .find(|c| !c.is_error && command_family(&c.tool_name, c.input.as_deref()) == family)
                .map(|c| c.turn_index);

            out.push(Incident {
                session_id: fail.session_id.clone(),
                project_path: fail.project_path.clone(),
                file_path: fail.file_path.clone(),
                family,
                error_signature: sig,
                failure_turn: fail.turn_index,
                recovery_turn: recovery,
                tool_name: fail.tool_name.clone(),
                intent: None,
            });
            if out.len() >= limit {
                return Ok(out);
            }
        }
        let _ = calls.first().map(|c| (&c.ordinal, &c.session_id));
    }
    Ok(out)
}

// ─── Tier 1: repetition clusters ─────────────────────────────────────

/// Cluster user turns by normalized form.
///
/// Exact-normalized equality, not MinHash: at this corpus size it is
/// cheap and has no false positives, and a false "you did this 40 times"
/// is worse than a missed near-duplicate. Near-duplicate matching is a
/// refinement, not a prerequisite.
pub fn detect_repetitions(
    idx: &CorpusIndex,
    min_count: usize,
    min_projects: usize,
) -> Result<Vec<RepetitionCluster>, CorpusError> {
    let db = idx.conn();
    let mut stmt = db.prepare(
        "SELECT e.user_text, s.project_path, e.session_id \
           FROM corpus_exchanges e \
           JOIN corpus_sessions s ON s.session_id = e.session_id \
          WHERE length(e.user_text) BETWEEN 4 AND 400",
    )?;
    struct Acc {
        sample: String,
        count: usize,
        projects: std::collections::HashSet<String>,
        sessions: std::collections::HashSet<String>,
    }
    let mut acc: HashMap<u64, Acc> = HashMap::new();
    let mut norms: HashMap<u64, String> = HashMap::new();

    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (text, project, session) = row?;
        // Harness plumbing is not a repeated *request*. Skipping it here
        // rather than at display time keeps the counts honest for every
        // consumer, not just the one that prints them.
        if is_harness_synthetic(&text) {
            continue;
        }
        let norm = normalize_prompt(&text);
        if norm.is_empty() {
            continue;
        }
        let key = fingerprint(&norm);
        norms.entry(key).or_insert_with(|| norm.clone());
        let e = acc.entry(key).or_insert_with(|| Acc {
            // Redacted here, not at print time: this string reaches
            // stdout and `--json`, and a prompt repeated often enough
            // to cluster is exactly where a pasted token would sit.
            sample: redact_secrets(&text),
            count: 0,
            projects: Default::default(),
            sessions: Default::default(),
        });
        e.count += 1;
        e.projects.insert(project);
        e.sessions.insert(session);
    }

    let mut out: Vec<RepetitionCluster> = acc
        .into_iter()
        .filter(|(_, a)| a.count >= min_count && a.projects.len() >= min_projects)
        .map(|(k, a)| RepetitionCluster {
            normalized: norms.get(&k).cloned().unwrap_or_default(),
            sample: a.sample,
            count: a.count,
            projects: a.projects.len(),
            sessions: a.sessions.len(),
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(b.projects.cmp(&a.projects)));
    Ok(out)
}

// ─── Tier 2: corroborated corrections ────────────────────────────────

/// Markers that *may* open a correction. A marker alone is not enough —
/// "no errors" is not a correction — which is why every hit must clear
/// [`corroborate`] before it survives.
const CORRECTION_MARKERS: &[&str] = &[
    "no,",
    "no.",
    "nope",
    "wrong",
    "actually",
    "instead",
    "don't",
    "do not",
    "stop",
    "revert",
    "undo",
    "that's not",
    "not like",
    "you forgot",
    "you missed",
    "still broken",
    "still failing",
    "i said",
    "as i said",
    "不对",
    "不是",
    "应该",
    "重来",
    "回退",
    "别这样",
];

fn opens_with_marker(text: &str) -> bool {
    let head: String = text.trim().to_lowercase().chars().take(60).collect();
    CORRECTION_MARKERS
        .iter()
        .any(|m| head.starts_with(m) || head.contains(m))
}

/// Detect user corrections that something else corroborates.
pub fn detect_corrections(idx: &CorpusIndex, limit: usize) -> Result<Vec<Correction>, CorpusError> {
    let db = idx.conn();
    let mut stmt = db.prepare(
        "SELECT e.file_path, e.session_id, s.project_path, e.turn_index, e.user_text, \
                (SELECT COUNT(*) FROM corpus_tool_calls tc \
                  WHERE tc.exchange_id = e.id AND tc.is_error = 1) \
           FROM corpus_exchanges e \
           JOIN corpus_sessions s ON s.session_id = e.session_id \
          WHERE length(e.user_text) BETWEEN 3 AND 600 \
          ORDER BY e.file_path, e.turn_index",
    )?;
    struct Turn {
        session_id: String,
        project_path: String,
        turn_index: i64,
        text: String,
        errors: i64,
    }
    impl TurnLike for Turn {
        fn turn_index(&self) -> i64 {
            self.turn_index
        }
        fn text(&self) -> &str {
            &self.text
        }
        fn errors(&self) -> i64 {
            self.errors
        }
    }
    let mut by_file: HashMap<String, Vec<Turn>> = HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Turn {
                session_id: r.get(1)?,
                project_path: r.get(2)?,
                turn_index: r.get(3)?,
                text: r.get(4)?,
                errors: r.get(5)?,
            },
        ))
    })?;
    for row in rows {
        let (file, t) = row?;
        by_file.entry(file).or_default().push(t);
    }

    let mut out = Vec::new();
    for turns in by_file.values() {
        for (i, t) in turns.iter().enumerate() {
            if is_harness_synthetic(&t.text) || !opens_with_marker(&t.text) {
                continue;
            }
            let Some(why) = corroborate(turns, i) else {
                continue;
            };
            out.push(Correction {
                session_id: t.session_id.clone(),
                project_path: t.project_path.clone(),
                turn_index: t.turn_index,
                text: redact_secrets(&t.text),
                corroboration: why,
            });
            if out.len() >= limit {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// A marker survives only with corroboration: either something actually
/// failed just before it, or the user is asking again for materially
/// what they already asked for. Without this the marker list matches
/// "no errors", "don't worry", and every other incidental use.
fn corroborate<T>(turns: &[T], i: usize) -> Option<Corroboration>
where
    T: TurnLike,
{
    let here = &turns[i];
    let lookback = CORRECTION_LOOKBACK_TURNS;

    // (a) a failing tool call in this turn or the ones just before it.
    for prev in turns[..=i].iter().rev() {
        if here.turn_index() - prev.turn_index() > lookback {
            break;
        }
        if prev.errors() > 0 {
            return Some(Corroboration::PrecedingFailure);
        }
    }

    // (b) the same intent, asked again.
    let norm_here = normalize_prompt(here.text());
    for prev in turns[..i].iter().rev() {
        if here.turn_index() - prev.turn_index() > lookback {
            break;
        }
        if jaccard(&norm_here, &normalize_prompt(prev.text())) >= REPEAT_INTENT_THRESHOLD {
            return Some(Corroboration::RepeatedIntent);
        }
    }
    None
}

/// Lets [`corroborate`] be tested without a database.
trait TurnLike {
    fn turn_index(&self) -> i64;
    fn text(&self) -> &str;
    fn errors(&self) -> i64;
}

// ─── the pass ────────────────────────────────────────────────────────

/// Run every tier. `limit` caps each detector independently so one
/// pathological corpus cannot starve the others.
pub fn detect_all(idx: &CorpusIndex, limit: usize) -> Result<Findings, CorpusError> {
    Ok(Findings {
        incidents: detect_incidents(idx, limit)?,
        repetitions: detect_repetitions(idx, MIN_REPETITION_COUNT, MIN_REPETITION_PROJECTS)?,
        corrections: detect_corrections(idx, limit)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T {
        turn: i64,
        text: String,
        errors: i64,
    }
    impl TurnLike for T {
        fn turn_index(&self) -> i64 {
            self.turn
        }
        fn text(&self) -> &str {
            &self.text
        }
        fn errors(&self) -> i64 {
            self.errors
        }
    }
    fn t(turn: i64, text: &str, errors: i64) -> T {
        T {
            turn,
            text: text.into(),
            errors,
        }
    }

    // ─── corroboration ──────────────────────────────────────────────

    /// The precision rule. "no errors" opens with a marker and is not a
    /// correction; without corroboration the marker list is noise.
    #[test]
    fn a_marker_without_corroboration_is_rejected() {
        let turns = vec![t(0, "run the tests", 0), t(1, "no, errors are fine", 0)];
        assert!(corroborate(&turns, 1).is_none());
    }

    #[test]
    fn a_preceding_failure_corroborates() {
        let turns = vec![t(0, "run the tests", 1), t(1, "no, use cargo nextest", 0)];
        assert_eq!(
            corroborate(&turns, 1),
            Some(Corroboration::PrecedingFailure)
        );
    }

    #[test]
    fn asking_again_corroborates() {
        let turns = vec![
            t(0, "run the workspace tests please", 0),
            t(1, "no, run the workspace tests please", 0),
        ];
        assert_eq!(corroborate(&turns, 1), Some(Corroboration::RepeatedIntent));
    }

    /// A failure long before the correction is not its cause.
    #[test]
    fn corroboration_respects_the_lookback_window() {
        let turns = vec![t(0, "build", 1), t(50, "no, do it differently", 0)];
        assert!(corroborate(&turns, 1).is_none());
    }

    #[test]
    fn markers_match_chinese_corrections_too() {
        assert!(opens_with_marker("不对，应该用 pnpm"));
        assert!(opens_with_marker("No, use pnpm instead"));
        assert!(!opens_with_marker("looks good, ship it"));
    }

    // ─── incident detection, end to end ─────────────────────────────

    use crate::corpus::{CorpusIndex, LOCAL_HOST};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write(root: &Path, slug: &str, sid: &str, lines: &[String]) {
        let dir = root.join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{sid}.jsonl")),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();
    }
    fn user(text: &str, ts: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{text}"}},"timestamp":"{ts}","cwd":"/repo/foo"}}"#
        )
    }
    fn tool_use(id: &str, name: &str, cmd: &str, ts: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{{"command":"{cmd}"}}}}]}},"timestamp":"{ts}","cwd":"/repo/foo"}}"#
        )
    }
    fn tool_res(id: &str, text: &str, err: bool, ts: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{id}","content":"{text}","is_error":{err}}}]}},"timestamp":"{ts}","cwd":"/repo/foo"}}"#
        )
    }

    fn indexed(lines: &[String]) -> (CorpusIndex, TempDir) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        write(&root, "-repo-foo", "S1", lines);
        let c = CorpusIndex::in_memory().unwrap();
        c.index_root(LOCAL_HOST, &root, 1).unwrap();
        (c, tmp)
    }

    #[test]
    fn a_failure_then_a_same_family_success_is_a_resolved_incident() {
        let (c, _t) = indexed(&[
            user("build it", "2026-04-10T10:00:00Z"),
            tool_use("t1", "Bash", "cargo test", "2026-04-10T10:00:01Z"),
            tool_res("t1", "Exit code 1", true, "2026-04-10T10:00:02Z"),
            user("fix it", "2026-04-10T10:00:03Z"),
            tool_use("t2", "Bash", "cargo test", "2026-04-10T10:00:04Z"),
            tool_res("t2", "ok", false, "2026-04-10T10:00:05Z"),
        ]);
        let inc = detect_incidents(&c, 100).unwrap();
        assert_eq!(inc.len(), 1, "one failure, one incident");
        assert!(
            inc[0].is_resolved(),
            "the later cargo success is its recovery"
        );
        assert_eq!(inc[0].family, "bash:cargo");
        assert!(inc[0].error_signature.contains("exit code 1"));
    }

    /// The constraint that kills the 358k-pair naive join: a success of
    /// a *different* command is not a recovery.
    #[test]
    fn an_unrelated_success_does_not_count_as_recovery() {
        let (c, _t) = indexed(&[
            user("build it", "2026-04-10T10:00:00Z"),
            tool_use("t1", "Bash", "cargo test", "2026-04-10T10:00:01Z"),
            tool_res("t1", "Exit code 1", true, "2026-04-10T10:00:02Z"),
            tool_use("t2", "Bash", "ls -la", "2026-04-10T10:00:03Z"),
            tool_res("t2", "ok", false, "2026-04-10T10:00:04Z"),
        ]);
        let inc = detect_incidents(&c, 100).unwrap();
        assert_eq!(inc.len(), 1);
        assert!(
            !inc[0].is_resolved(),
            "an `ls` success must not resolve a `cargo` failure"
        );
    }

    /// And a failure that never recovers is `unresolved` — not
    /// "abandoned", which the transcript cannot support.
    #[test]
    fn a_failure_with_no_success_is_unresolved_not_abandoned() {
        let (c, _t) = indexed(&[
            user("build it", "2026-04-10T10:00:00Z"),
            tool_use("t1", "Bash", "cargo test", "2026-04-10T10:00:01Z"),
            tool_res("t1", "Exit code 1", true, "2026-04-10T10:00:02Z"),
        ]);
        let inc = detect_incidents(&c, 100).unwrap();
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].recovery_turn, None);
    }

    #[test]
    fn repetition_needs_both_a_count_and_more_than_one_project() {
        let (c, _t) = indexed(&[
            user("check status", "2026-04-10T10:00:00Z"),
            user("check status", "2026-04-10T10:01:00Z"),
            user("check status", "2026-04-10T10:02:00Z"),
        ]);
        // One project only — must not surface however often it repeats.
        assert!(detect_repetitions(&c, 2, 2).unwrap().is_empty());
        // Same corpus, single-project threshold — now it does.
        let r = detect_repetitions(&c, 2, 1).unwrap();
        assert!(r.iter().any(|x| x.normalized.contains("check status")));
    }

    #[test]
    fn a_repetition_cluster_keeps_an_unnormalized_sample() {
        let (c, _t) = indexed(&[
            user("Fix The Build", "2026-04-10T10:00:00Z"),
            user("Fix The Build", "2026-04-10T10:01:00Z"),
        ]);
        let r = detect_repetitions(&c, 2, 1).unwrap();
        let cluster = r.iter().find(|x| x.count >= 2).unwrap();
        assert_eq!(
            cluster.sample, "Fix The Build",
            "display copy is the original"
        );
        assert_eq!(cluster.normalized, "fix the build");
    }

    /// Regression: harness plumbing dominated the first real run's
    /// repetition clusters. It must not reach them at all.
    #[test]
    fn harness_turns_never_form_a_repetition_cluster() {
        let (c, _t) = indexed(&[
            user("<command-name>/exit</command-name>", "2026-04-10T10:00:00Z"),
            user("<command-name>/exit</command-name>", "2026-04-10T10:01:00Z"),
            user("<command-name>/exit</command-name>", "2026-04-10T10:02:00Z"),
            user("commit and push", "2026-04-10T10:03:00Z"),
            user("commit and push", "2026-04-10T10:04:00Z"),
        ]);
        let r = detect_repetitions(&c, 2, 1).unwrap();
        assert!(
            !r.iter().any(|x| x.sample.contains("command-name")),
            "harness turns must not cluster: {r:?}"
        );
        assert!(
            r.iter().any(|x| x.sample.contains("commit and push")),
            "but real repeated requests still must"
        );
    }

    /// Regression: `corpus detect` prints these to stdout and
    /// serializes them under `--json`, so a token pasted into a prompt
    /// that repeats would have landed on the terminal. CI's R9 tripwire
    /// caught this on the feature's first run.
    #[test]
    fn emitted_samples_and_corrections_are_redacted() {
        let secret = "sk-ant-oat01-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let (c, _t) = indexed(&[
            user(
                &format!("use token {secret} please"),
                "2026-04-10T10:00:00Z",
            ),
            user(
                &format!("use token {secret} please"),
                "2026-04-10T10:01:00Z",
            ),
            tool_use("t1", "Bash", "cargo test", "2026-04-10T10:02:00Z"),
            tool_res("t1", "Exit code 1", true, "2026-04-10T10:03:00Z"),
            user(&format!("no, not {secret}"), "2026-04-10T10:04:00Z"),
        ]);

        for r in detect_repetitions(&c, 2, 1).unwrap() {
            assert!(
                !r.sample.contains(secret),
                "repetition sample leaked a secret: {}",
                r.sample
            );
        }
        let corrections = detect_corrections(&c, 100).unwrap();
        assert!(
            !corrections.is_empty(),
            "precondition: a correction is found"
        );
        for corr in corrections {
            assert!(
                !corr.text.contains(secret),
                "correction text leaked a secret: {}",
                corr.text
            );
        }
    }

    #[test]
    fn detect_all_runs_every_tier_without_erroring() {
        let (c, _t) = indexed(&[
            user("build it", "2026-04-10T10:00:00Z"),
            tool_use("t1", "Bash", "cargo test", "2026-04-10T10:00:01Z"),
            tool_res("t1", "Exit code 1", true, "2026-04-10T10:00:02Z"),
            user("no, use nextest", "2026-04-10T10:00:03Z"),
        ]);
        let f = detect_all(&c, 100).unwrap();
        assert_eq!(f.incidents.len(), 1);
        assert_eq!(f.resolved_incidents(), 0);
        assert_eq!(f.corrections.len(), 1, "corroborated by the failure above");
        assert_eq!(
            f.corrections[0].corroboration,
            Corroboration::PrecedingFailure
        );
    }

    #[test]
    fn an_empty_corpus_yields_nothing_rather_than_erroring() {
        let c = CorpusIndex::in_memory().unwrap();
        let f = detect_all(&c, 100).unwrap();
        assert!(f.incidents.is_empty() && f.repetitions.is_empty() && f.corrections.is_empty());
    }
}
