//! Turning a raw first user prompt into a one-line title.
//!
//! `first_user_prompt` is whatever the user typed, verbatim — and what
//! people type into Claude Code is markdown. A real session on this
//! machine opens with a `## User Input` heading, a fenced path, and a
//! `- **Plan files**` bullet, all of which reached the card as
//! characters:
//!
//! ```text
//! ## User Input … <fence>text … dev-docs/remote-panel-plan.md 3 …
//! <fence> … Arguments are space-separated: … - **Plan files** — …
//! ```
//!
//! (The fences are spelled out rather than shown, because a nested code
//! fence in a doc comment is a rustdoc failure — which is how this
//! paragraph first broke `cargo test`.)
//!
//! A title is one line of prose; markdown belongs in the body.
//!
//! ## Stripped, not rendered
//!
//! A title has no room for a heading, a list or a code block, so the
//! marks come **out** rather than being turned into elements. That is
//! the opposite of what the transcript body does, and deliberately so.
//!
//! ## What is deliberately left alone
//!
//! **Every underscore, including `__bold__`.** The first version of this
//! stripped paired `__…__` on the reasoning that a pair is unambiguous.
//! It is not: the vectors caught `__init__ never runs` becoming
//! `init never runs`, which is exactly the corruption this module claims
//! to avoid, in the module that claims it. `__bold__` is rare in a
//! prompt and Python dunders are not, so the underscore rule is gone
//! rather than narrowed — a narrower version would still be a rule about
//! a character that means something else here.
//!
//! **Single `*`.** Same reasoning: `*.log`, `2 * 3`. Paired `**…**` is
//! removed because nothing else spells it, and one stray asterisk reads
//! far better than a mangled glob.
//!
//! ## Two implementations, locked together
//!
//! The panel gets its title from this function through the HTTP DTO;
//! the desktop derives its own in `src/sections/sessions/format.ts`,
//! because it renders `SessionRow` directly over IPC and adding a
//! derived column to the cache would mean a schema change for a
//! presentation concern.
//!
//! So the rule exists twice, and the two are pinned by
//! `crates/claudepot-core/testdata/session-title-vectors.json` — both
//! run those vectors. **Change one, change the other, add a vector.**
//! Same arrangement as `PriceBook::resolve` and `src/costs.ts`.

use once_cell::sync::Lazy;
use regex::Regex;

/// Claude Code's own reference placeholders, embedded directly in the
/// prompt text: `[Image #3]`, `[Pasted text #1 +42 lines]`. Kept
/// byte-compatible with CC's `parseReferences`, and stripped because
/// they leak an internal encoding into the UI.
static CC_REF_PLACEHOLDER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[(?:Pasted text|Image|\.\.\.Truncated text) #\d+(?: \+\d+ lines)?\.*\]")
        .expect("static regex")
});

/// Leading block scaffolding, peeled one layer at a time so a stack
/// like `> ## heading` unwinds.
static LEADING_FENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^```[A-Za-z0-9_+-]*[ \t]*\r?\n?").expect("static regex"));
static LEADING_HEADING: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^#{1,6}\s+").expect("static regex"));
static LEADING_QUOTE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^>\s*").expect("static regex"));
static LEADING_BULLET: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?:[-*+]|\d+\.)[ \t]+").expect("static regex"));

/// An image before a link — `![alt](url)` must not be read as a link
/// preceded by a stray `!`.
static IMAGE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"!\[([^\]]*)\]\([^)]*\)").expect("static regex"));
static LINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("static regex"));
static AUTOLINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<((?:https?|mailto):[^>\s]+)>").expect("static regex"));

/// Paired asterisks only. See the module docs on why underscores are
/// left alone entirely.
static BOLD_STAR: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*([^*]+)\*\*").expect("static regex"));
static STRIKE: Lazy<Regex> = Lazy::new(|| Regex::new(r"~~([^~]+)~~").expect("static regex"));

/// Any remaining fence, wherever it sits. The leading pass only reaches
/// the first one; a prompt that opens with prose and then fences a path
/// keeps its closing delimiter otherwise.
static ANY_FENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"```[A-Za-z0-9_+-]*").expect("static regex"));

/// Inline code: the backticks go, the content stays. A prompt naming a
/// file in backticks should read as the file name.
static INLINE_CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`([^`]*)`").expect("static regex"));

static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("static regex"));

/// One line of prose, or `None` when nothing readable is left.
///
/// `None` rather than an empty string: a caller has to choose a
/// fallback — a session id, a subsession hint — and an empty title
/// rendered as a blank row is the failure this return type prevents.
pub fn derive(raw: &str) -> Option<String> {
    let cleaned = CC_REF_PLACEHOLDER.replace_all(raw, "").into_owned();

    // Scaffolding is peeled **per line**, before the lines are joined.
    //
    // The first version peeled only the head of the whole string, so a
    // prompt whose list started on line five kept its `- ` markers —
    // `Arguments are space-separated: - Plan files`. Doing it after the
    // join is not an option either: a mid-sentence `-` ("well - it
    // depends") is a hyphen, and only its position at the start of a
    // line makes it a bullet. That information exists here and nowhere
    // later.
    let mut lines: Vec<String> = Vec::new();
    for line in cleaned.lines() {
        let mut l = line.to_string();
        loop {
            let before = l.clone();
            l = l.trim_start().to_string();
            // A fence before a heading, so "```md" then "# Title"
            // unwinds in that order.
            l = LEADING_FENCE.replace(&l, "").into_owned();
            l = LEADING_HEADING.replace(&l, "").into_owned();
            l = LEADING_QUOTE.replace(&l, "").into_owned();
            l = LEADING_BULLET.replace(&l, "").into_owned();
            if l == before {
                break;
            }
        }
        lines.push(l);
    }
    let mut s = lines.join("\n");

    // Inline marks, everywhere. Images before links, or `![alt](url)`
    // reads as a link behind a stray `!`.
    s = IMAGE.replace_all(&s, "$1").into_owned();
    s = LINK.replace_all(&s, "$1").into_owned();
    s = AUTOLINK.replace_all(&s, "$1").into_owned();
    s = BOLD_STAR.replace_all(&s, "$1").into_owned();
    s = STRIKE.replace_all(&s, "$1").into_owned();
    s = ANY_FENCE.replace_all(&s, " ").into_owned();
    s = INLINE_CODE.replace_all(&s, "$1").into_owned();

    // One line. A prompt is often a paragraph; a card row is not.
    s = WHITESPACE.replace_all(&s, " ").trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Vector {
        name: String,
        raw: String,
        want: Option<String>,
    }

    /// The shared vectors, which `src/sections/sessions/format.ts` also
    /// runs. A change here that is not mirrored there is a drift between
    /// what the panel shows and what the desktop shows for the same
    /// session.
    #[test]
    fn shared_vectors() {
        let raw = include_str!("../../testdata/session-title-vectors.json");
        let vectors: Vec<Vector> = serde_json::from_str(raw).expect("vectors parse");
        assert!(
            vectors.len() >= 20,
            "the vector file lost most of its cases"
        );
        for v in vectors {
            assert_eq!(
                derive(&v.raw).as_deref(),
                v.want.as_deref(),
                "vector {:?}",
                v.name
            );
        }
    }

    #[test]
    fn the_real_prompt_that_prompted_this() {
        // Copied off this machine. Every mark in it was on screen.
        let raw = "## User Input\n\n```text\ndev-docs/remote-panel-plan.md 3\n```\n\nArguments are space-separated:\n\n- **Plan files** — one or more paths.";
        assert_eq!(
            derive(raw).as_deref(),
            Some(
                "User Input dev-docs/remote-panel-plan.md 3 Arguments are space-separated: \
                 Plan files — one or more paths."
            )
        );
    }

    #[test]
    fn nothing_readable_is_none_not_empty() {
        assert_eq!(derive(""), None);
        assert_eq!(derive("   \n\n  "), None);
        assert_eq!(derive("```"), None);
        assert_eq!(derive("## "), None);
        assert_eq!(derive("[Image #1]"), None);
    }

    #[test]
    fn identifiers_and_globs_survive_intact() {
        // The reason single `*` and `_` are left alone. Each of these is
        // a plausible opening for a prompt, and each would be mangled by
        // a stripper that treated them as emphasis.
        for s in [
            "some_var_name is wrong",
            "__init__ never runs",
            "delete *.log files",
            "why is 2 * 3 * 4 slow",
            "check C:\\Users\\a_b\\c",
            "the file is a_b_c.txt",
        ] {
            assert_eq!(derive(s).as_deref(), Some(s), "mangled: {s}");
        }
    }
}
