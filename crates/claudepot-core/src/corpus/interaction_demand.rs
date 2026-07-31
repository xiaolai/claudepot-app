//! Does a human ever need to answer an agent with something text
//! cannot carry?
//!
//! This detector decides plan §10.2 — whether the interaction half of
//! boards (durable waiters, approval manifests, trusted chrome) is worth
//! its complexity and risk.
//!
//! # What it deliberately does NOT measure
//!
//! The obvious detector counts how often agents asked a *rich* question
//! — brushed region, threshold off a distribution, twelve fields at
//! once. That test is **selection-biased and must not be used.** Agents
//! in this corpus had exactly one channel: text. An agent that can only
//! ask yes/no questions will be found to have asked yes/no questions,
//! so a low count is what you observe whether or not the demand exists.
//!
//! What is observable without that bias is the *residue* — places where
//! the text channel was used and visibly strained:
//!
//! - [`Signal::ClarificationChain`] — several short round trips to
//!   settle one parameter. The channel worked, but at N× the turns.
//! - [`Signal::PastedStructure`] — a human hand-serializing a table or
//!   a coordinate list into a chat message.
//! - [`Signal::SpatialGesture`] — "the spike around 3.5", "the second
//!   cluster". A human pointing at something they could not click.
//! - [`Signal::AmbiguousAnswer`] — a terse answer immediately followed
//!   by a correction, i.e. the agent guessed wrong from an
//!   underspecified reply.
//!
//! Each is a *lower* bound on demand: it only fires where someone
//! pushed through the limitation rather than giving up on the question.
//!
//! # Vocabulary
//!
//! Nothing here is a **recurrence**. That word has a precise,
//! human-confirmed meaning in `shared_memory::recurrence`, and diluting
//! it breaks the one honest signal the knowledge base has. These are
//! *demand signals*.

use std::collections::HashMap;

use super::normalize::is_harness_synthetic;
use crate::corpus::{CorpusError, CorpusIndex};
use crate::session_live::redact::redact_secrets;

/// A turn short enough to be an answer rather than a request.
const TERSE_ANSWER_CHARS: usize = 40;

/// How many consecutive terse user turns make a clarification *chain*
/// rather than ordinary back-and-forth.
const MIN_CHAIN_LEN: usize = 3;

/// Rows of pipe-or-comma-separated data before a message counts as
/// pasted structure rather than prose that happens to contain a comma.
const MIN_STRUCTURED_LINES: usize = 3;

/// Which strain the text channel showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    ClarificationChain,
    PastedStructure,
    SpatialGesture,
    AmbiguousAnswer,
}

impl Signal {
    pub fn as_str(self) -> &'static str {
        match self {
            Signal::ClarificationChain => "clarification_chain",
            Signal::PastedStructure => "pasted_structure",
            Signal::SpatialGesture => "spatial_gesture",
            Signal::AmbiguousAnswer => "ambiguous_answer",
        }
    }

    /// What a hit implies about a richer input channel.
    pub fn implication(self) -> &'static str {
        match self {
            Signal::ClarificationChain => "one parameter cost several round trips",
            Signal::PastedStructure => "a human hand-serialized structure into chat",
            Signal::SpatialGesture => "a human pointed at something they could not click",
            Signal::AmbiguousAnswer => "a terse answer was misread and corrected",
        }
    }
}

/// One observed strain, with enough context to go read the transcript.
#[derive(Debug, Clone)]
pub struct DemandSignal {
    pub signal: Signal,
    pub session_id: String,
    pub project_path: String,
    pub turn_index: i64,
    /// Redacted excerpt. A transcript excerpt reaches stdout and
    /// `--json`; this is exactly where a pasted token would sit.
    pub sample: String,
    /// For a chain, how many turns it took. 1 for point signals.
    pub turns: usize,
}

/// Counts per signal plus the sampled hits.
#[derive(Debug, Default)]
pub struct DemandReport {
    pub signals: Vec<DemandSignal>,
    pub counts: HashMap<&'static str, usize>,
    /// Exchanges examined after harness filtering — the denominator.
    pub examined: usize,
}

impl DemandReport {
    pub fn count(&self, s: Signal) -> usize {
        self.counts.get(s.as_str()).copied().unwrap_or(0)
    }

    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }

    /// Hits per 1,000 examined exchanges.
    ///
    /// The rate, not the raw count, is what the §10.2 decision turns on
    /// — a big corpus makes any absolute number look impressive.
    pub fn rate_per_1k(&self) -> f64 {
        if self.examined == 0 {
            return 0.0;
        }
        self.total() as f64 * 1000.0 / self.examined as f64
    }
}

/// Phrases that point at a position in something visual. Deliberately
/// narrow — "around" alone matches far too much ordinary prose.
const SPATIAL_MARKERS: &[&str] = &[
    "the spike",
    "that spike",
    "the dip",
    "that dip",
    "the peak",
    "that peak",
    "the cluster",
    "second cluster",
    "first cluster",
    "the outlier",
    "that outlier",
    "the bump",
    "the plateau",
    "top left",
    "top right",
    "bottom left",
    "bottom right",
    "the tail end",
    "the flat part",
    "the steep part",
];

/// A spatial gesture needs a *location* as well as a landmark, or
/// "the peak" is just a noun.
const LOCATION_HINTS: &[&str] = &[
    " around ",
    " near ",
    " at about ",
    " at x",
    " at y",
    " just after ",
    " just before ",
    " between ",
    " right after ",
    " right before ",
];

/// True when the text points at a position in a visual rather than
/// naming a thing.
pub fn is_spatial_gesture(text: &str) -> bool {
    let lower = text.to_lowercase();
    let Some(at) = SPATIAL_MARKERS
        .iter()
        .find_map(|m| lower.find(m).map(|i| i + m.len()))
    else {
        return false;
    };

    // The location must sit **right after** the landmark. A bare
    // `" at "` anywhere in the message was the original rule and it
    // matched arXiv abstracts and bot-office submissions — any long
    // document contains both a landmark word and the word "at"
    // somewhere.
    let tail: String = lower[at..].chars().take(24).collect();
    LOCATION_HINTS.iter().any(|h| tail.starts_with(h.trim_end()))
        || tail.starts_with(" at ")
        // "the spike at 3.5" / "the dip 12s in" — a number immediately
        // after the landmark is a location.
        || tail
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
}

/// Tokens that mark a line as source code or serialized data rather
/// than a hand-typed table.
const CODE_MARKERS: &[&str] = &[
    "{", "}", "();", "=>", "->", "::", "def ", "fn ", "let ", "const ", "import ", "from ",
    "class ", "return ", "public ", "func ", "#include", "</", "/>", "&&", "||", "$(",
];

/// Strip fenced code blocks. Pasted code is the dominant false positive
/// for [`is_pasted_structure`] — in this corpus it outnumbered real
/// hand-typed tables by roughly ten to one before this ran.
fn strip_fenced(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn looks_like_code(line: &str) -> bool {
    let l = line.trim();
    l.ends_with(';')
        || l.ends_with('{')
        || l.ends_with(',') && l.starts_with('"')
        || CODE_MARKERS.iter().any(|m| l.contains(m))
}

/// True when a message carries hand-serialized tabular data.
///
/// # The false-positive problem this exists to survive
///
/// The naive version — "three lines sharing a comma count" — matched
/// pasted code, JSON, log output, and import lists, and reported 12.8%
/// of every exchange in the reference corpus as a hand-typed table.
/// That number was not a finding, it was the detector failing.
///
/// So: fenced blocks are stripped first, code-shaped lines are excluded
/// from the count, and a JSON- or log-shaped message is rejected
/// outright. What survives is much closer to someone actually typing a
/// table into chat — which is the thing plan §10.2 is trying to count.
pub fn is_pasted_structure(text: &str) -> bool {
    let unfenced = strip_fenced(text);
    let lines: Vec<&str> = unfenced
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < MIN_STRUCTURED_LINES {
        return false;
    }

    // A message that opens as JSON or an array is serialized data a
    // tool produced, not a table a human typed.
    if lines[0].starts_with('{') || lines[0].starts_with('[') {
        return false;
    }

    let candidates: Vec<&&str> = lines.iter().filter(|l| !looks_like_code(l)).collect();
    if candidates.len() < MIN_STRUCTURED_LINES {
        return false;
    }

    // Two or more pipes on a line is a table row; one pipe is a shell
    // command, which is why the threshold is not 1.
    let pipe_rows = candidates
        .iter()
        .filter(|l| l.matches('|').count() >= 2)
        .count();

    // CSV-ish: several lines sharing an identical field count. Equal
    // counts are what separates a table from prose that happens to use
    // commas.
    let mut by_commas: HashMap<usize, usize> = HashMap::new();
    for l in &candidates {
        let n = l.matches(',').count();
        if n >= 1 {
            *by_commas.entry(n).or_default() += 1;
        }
    }
    let csv_rows = by_commas.values().copied().max().unwrap_or(0);

    let table_rows = pipe_rows.max(csv_rows);
    if table_rows < MIN_STRUCTURED_LINES {
        return false;
    }

    // **The message must be MOSTLY the table.**
    //
    // This is the discriminator that the line-count rules could not
    // supply, and it was found by reading the hits rather than by
    // reasoning: at 10.7% of all exchanges, nearly every match was an
    // agent *prompt template* or a bot-office submission — long
    // structured prose whose `Tags: a, b, c` lines tripped the comma
    // rule. None of them was a human serializing data for lack of a
    // better channel.
    //
    // A human who types a table into chat produces a message that is
    // largely that table. A template has one buried in paragraphs. The
    // ratio separates them; no amount of per-line cleverness does.
    table_rows * 2 >= lines.len()
}

/// True when a user turn is short enough to be an answer to a question
/// rather than a new request.
pub fn is_terse_answer(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.chars().count() <= TERSE_ANSWER_CHARS
}

/// Markers that open a correction of the agent's last interpretation.
const CORRECTION_OPENERS: &[&str] = &[
    "no,",
    "no ",
    "not that",
    "i meant",
    "i mean",
    "wrong",
    "that's not",
    "thats not",
    "actually,",
    "nope",
    "other one",
    "the other",
];

pub fn opens_with_correction(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    CORRECTION_OPENERS.iter().any(|m| lower.starts_with(m))
}

struct Turn {
    session_id: String,
    project_path: String,
    turn_index: i64,
    text: String,
}

/// Scan the corpus for the four demand signals.
///
/// `limit` caps the sampled signals returned; counts are complete
/// regardless, because the §10.2 decision needs the rate and a
/// truncated numerator would understate it.
pub fn detect_demand(idx: &CorpusIndex, limit: usize) -> Result<DemandReport, CorpusError> {
    let db = idx.conn();
    let mut stmt = db.prepare(
        "SELECT e.file_path, e.session_id, s.project_path, e.turn_index, e.user_text \
           FROM corpus_exchanges e \
           JOIN corpus_sessions s ON s.session_id = e.session_id \
          WHERE length(e.user_text) BETWEEN 2 AND 4000 \
          ORDER BY e.file_path, e.turn_index",
    )?;

    let mut by_file: HashMap<String, Vec<Turn>> = HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Turn {
                session_id: r.get(1)?,
                project_path: r.get(2)?,
                turn_index: r.get(3)?,
                text: r.get(4)?,
            },
        ))
    })?;
    for row in rows {
        let (file, t) = row?;
        // Harness plumbing first, always. CC injects
        // `<local-command-caveat>`, `<command-name>`, `<bash-stdout>`
        // and `[Request interrupted by user]`; before filtering, those
        // dominate every text-shaped count in this corpus.
        if is_harness_synthetic(&t.text) {
            continue;
        }
        by_file.entry(file).or_default().push(t);
    }

    let mut report = DemandReport::default();
    let bump = |report: &mut DemandReport, s: Signal| {
        *report.counts.entry(s.as_str()).or_default() += 1;
    };

    for turns in by_file.values() {
        report.examined += turns.len();

        let mut i = 0usize;
        while i < turns.len() {
            let t = &turns[i];

            // A run of consecutive terse turns is one chain, counted
            // once — counting each turn would inflate a single
            // negotiation into several signals.
            let mut run = 0usize;
            while i + run < turns.len() && is_terse_answer(&turns[i + run].text) {
                run += 1;
            }
            if run >= MIN_CHAIN_LEN {
                bump(&mut report, Signal::ClarificationChain);
                if report.signals.len() < limit {
                    report.signals.push(DemandSignal {
                        signal: Signal::ClarificationChain,
                        session_id: t.session_id.clone(),
                        project_path: t.project_path.clone(),
                        turn_index: t.turn_index,
                        sample: redact_secrets(&t.text),
                        turns: run,
                    });
                }
                i += run;
                continue;
            }

            let mut hit = None;
            if is_pasted_structure(&t.text) {
                hit = Some(Signal::PastedStructure);
            } else if is_spatial_gesture(&t.text) {
                hit = Some(Signal::SpatialGesture);
            } else if i > 0 && is_terse_answer(&turns[i - 1].text) && opens_with_correction(&t.text)
            {
                // The *previous* turn was the terse answer; this one
                // corrects the agent's reading of it.
                hit = Some(Signal::AmbiguousAnswer);
            }

            if let Some(s) = hit {
                bump(&mut report, s);
                if report.signals.len() < limit {
                    report.signals.push(DemandSignal {
                        signal: s,
                        session_id: t.session_id.clone(),
                        project_path: t.project_path.clone(),
                        turn_index: t.turn_index,
                        sample: redact_secrets(&t.text),
                        turns: 1,
                    });
                }
            }
            i += 1;
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_landmark_is_not_a_spatial_gesture() {
        // "the peak" is a noun until it carries a location.
        assert!(!is_spatial_gesture("the peak performance was good"));
        assert!(!is_spatial_gesture("we hit the plateau of the S curve"));
    }

    #[test]
    fn a_landmark_plus_a_location_is_a_spatial_gesture() {
        assert!(is_spatial_gesture("look at the spike around 3.5"));
        assert!(is_spatial_gesture("the second cluster near the origin"));
        assert!(is_spatial_gesture("the dip at x=12"));
        assert!(is_spatial_gesture("the outlier just after the restart"));
    }

    #[test]
    fn ordinary_prose_is_not_a_spatial_gesture() {
        assert!(!is_spatial_gesture("please run the tests around noon"));
        assert!(!is_spatial_gesture("refactor the parser"));
    }

    #[test]
    fn a_single_comma_line_is_not_pasted_structure() {
        assert!(!is_pasted_structure("hello, world"));
        assert!(!is_pasted_structure("run a, then b"));
    }

    #[test]
    fn a_pipe_table_is_pasted_structure() {
        let t = "| name | cost |\n| a | 1 |\n| b | 2 |\n| c | 3 |";
        assert!(is_pasted_structure(t));
    }

    #[test]
    fn consistent_csv_lines_are_pasted_structure() {
        let t = "opus,5,25\nsonnet,3,15\nhaiku,1,5";
        assert!(is_pasted_structure(t));
    }

    #[test]
    fn fenced_code_is_not_pasted_structure() {
        // The dominant false positive: pasted code outnumbered real
        // hand-typed tables ~10:1 in the reference corpus.
        let t = "here:\n```python\na, b, c = 1, 2, 3\nx, y, z = 4, 5, 6\np, q, r = 7, 8, 9\n```";
        assert!(!is_pasted_structure(t));
    }

    #[test]
    fn unfenced_code_lines_are_excluded_from_the_count() {
        let t = "import os, sys\nfrom a import b, c\nconst x = {a, b};";
        assert!(!is_pasted_structure(t));
    }

    #[test]
    fn a_json_payload_is_not_a_hand_typed_table() {
        let t = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}";
        assert!(!is_pasted_structure(t));
    }

    #[test]
    fn a_real_table_still_survives_the_exclusions() {
        // The exclusions must not swallow the signal they exist to
        // isolate.
        assert!(is_pasted_structure("opus,5,25\nsonnet,3,15\nhaiku,1,5"));
        assert!(is_pasted_structure(
            "| name | cost |\n| a | 1 |\n| b | 2 |\n| c | 3 |"
        ));
    }

    #[test]
    fn ragged_prose_with_commas_is_not_pasted_structure() {
        // Different comma counts per line -> prose, not a table.
        let t = "first, we do this\nthen the second, third, and fourth steps\nfinally done";
        assert!(!is_pasted_structure(t));
    }

    #[test]
    fn terse_answers_are_bounded_by_length() {
        assert!(is_terse_answer("yes"));
        assert!(is_terse_answer("the second one"));
        assert!(!is_terse_answer(&"x".repeat(TERSE_ANSWER_CHARS + 1)));
        assert!(!is_terse_answer("   "));
    }

    #[test]
    fn correction_openers_are_matched_at_the_start_only() {
        assert!(opens_with_correction("no, the other one"));
        assert!(opens_with_correction("I meant the second"));
        assert!(opens_with_correction("actually, use bar"));
        // "wrong" mid-sentence is a description, not a correction.
        assert!(!opens_with_correction("that produced the wrong output"));
    }

    #[test]
    fn signal_names_are_not_the_word_recurrence() {
        // `recurrence` has a precise human-confirmed meaning in
        // shared_memory; diluting it breaks the one honest signal the
        // knowledge base has.
        for s in [
            Signal::ClarificationChain,
            Signal::PastedStructure,
            Signal::SpatialGesture,
            Signal::AmbiguousAnswer,
        ] {
            assert!(!s.as_str().contains("recurrence"));
            assert!(!s.implication().contains("recurrence"));
        }
    }

    #[test]
    fn rate_is_zero_on_an_empty_corpus_rather_than_nan() {
        let r = DemandReport::default();
        assert_eq!(r.rate_per_1k(), 0.0);
        assert_eq!(r.total(), 0);
    }

    #[test]
    fn rate_is_per_thousand_examined() {
        let mut r = DemandReport {
            examined: 2000,
            ..Default::default()
        };
        r.counts.insert(Signal::SpatialGesture.as_str(), 4);
        assert_eq!(r.rate_per_1k(), 2.0);
    }
}
