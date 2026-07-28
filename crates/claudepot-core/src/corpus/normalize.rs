//! Tier 0 — turn raw transcript text into something comparable.
//!
//! Everything downstream groups by equality or near-equality, and raw
//! transcript text never repeats exactly: it carries absolute paths,
//! uuids, hashes, timestamps, line numbers, ports and durations that
//! differ on every run. Normalizing produces a *comparison* copy; the
//! display copy is always the original.
//!
//! # Two normalizers, on purpose
//!
//! [`normalize_prompt`] is aggressive — it exists to make "run the
//! tests" and "run the tests" cluster, so it flattens numbers wholesale.
//!
//! [`error_signature`] is not. An exit code is the most discriminating
//! part of a failure, so small integers survive; what gets stripped is
//! the *payload*. That distinction is load-bearing: normalizing
//! `Exit code 1` and `Exit code 143` together would merge "the command
//! failed" with "the command timed out".
//!
//! # Privacy
//!
//! Tool output is arbitrary stdout. On the reference machine it
//! includes bank statements. [`error_signature`] therefore takes only
//! the **first line** — where `Exit code N`, `Traceback…`,
//! `error[E0433]` live — runs it through the shared redactor, and caps
//! it. A signature is a grouping key, never a quote.

use crate::session_live::redact::redact_secrets;
use regex::Regex;
use std::sync::OnceLock;

/// Longest signature we keep. Past this it is payload, not shape.
const SIGNATURE_CAP: usize = 96;

fn re_uuid() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap()
    })
}

fn re_hash() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{7,}\b").unwrap())
}

/// Absolute paths, POSIX and Windows. Deliberately not anchored to `/`
/// alone — `rules/paths.md`: a Windows drive letter or UNC prefix is
/// just as much an absolute path, and leaving those un-normalized would
/// make the same operation cluster differently per platform.
fn re_path() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?:[A-Za-z]:\\|\\\\|/)[^\s'\x22`,;:()\[\]{}]{2,}").unwrap())
}

fn re_ts() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Case-insensitive because callers lowercase first, so the ISO `T`
    // arrives as `t`. No trailing `\b`: an ISO stamp is frequently
    // butted against following text, and a word boundary would then
    // fail and leave the whole stamp unnormalized.
    R.get_or_init(|| {
        Regex::new(r"(?i)\b\d{4}-\d{2}-\d{2}(?:[t ]\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?)?z?").unwrap()
    })
}

fn re_num() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\b\d+\b").unwrap())
}

/// Big numbers only — keeps exit codes and small counts, drops byte
/// sizes, pids, ports, durations and line offsets.
fn re_bignum() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\b\d{4,}\b").unwrap())
}

fn re_ws() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s+").unwrap())
}

/// Aggressive comparison form for clustering user prompts.
///
/// Order matters: timestamps before numbers (or the number rule eats
/// their digits first and the timestamp shape is lost), uuids and
/// hashes before paths (a path can contain both).
pub fn normalize_prompt(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = re_ts().replace_all(&s, " <ts> ");
    let s = re_uuid().replace_all(&s, " <uuid> ");
    let s = re_path().replace_all(&s, " <path> ");
    let s = re_hash().replace_all(&s, " <hash> ");
    let s = re_num().replace_all(&s, " <n> ");
    re_ws().replace_all(&s, " ").trim().to_string()
}

/// Stable 64-bit key for a normalized string.
pub fn fingerprint(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Grouping key for a tool failure.
///
/// First line only, redacted, lightly normalized, capped. Returns
/// `None` for output with no usable first line — an unclassifiable
/// failure is better dropped than grouped under an empty key, which
/// would merge every such failure into one enormous fake cluster.
pub fn error_signature(tool_result_text: &str) -> Option<String> {
    let first = tool_result_text.lines().find(|l| !l.trim().is_empty())?;
    let first = redact_secrets(first);
    let s = first.trim().to_lowercase();
    let s = re_ts().replace_all(&s, "<ts>");
    let s = re_uuid().replace_all(&s, "<uuid>");
    let s = re_path().replace_all(&s, "<path>");
    let s = re_hash().replace_all(&s, "<hash>");
    // Small integers survive: `exit code 1` and `exit code 143` are
    // different failures.
    let s = re_bignum().replace_all(&s, "<n>");
    let s = re_ws().replace_all(&s, " ").trim().to_string();
    if s.is_empty() {
        return None;
    }
    Some(s.chars().take(SIGNATURE_CAP).collect())
}

/// Shell words that prefix a command without being the command, and
/// which take **no** argument — scanning continues inside the segment.
const SHELL_PREFIX: &[&str] = &["sudo", "env", "time", "nohup", "exec", "then", "do", "!"];

/// Shell words that consume the rest of their segment. `cd /some/path`
/// must skip the *whole* segment: skipping only the word leaves the
/// path, whose basename then becomes the "command" — so
/// `cd "/Users/joker/repo"; cargo test` would report `repo` instead of
/// `cargo`, and pair a failed build with a successful directory change.
const SHELL_SEGMENT_CONSUMERS: &[&str] = &["cd", "source", ".", "pushd", "popd", "export"];

/// Text CC injects into the transcript that the user never typed.
///
/// Found empirically: the first detector run over the real corpus
/// returned `<local-command-caveat>…` 1,258 times across 69 projects as
/// its single largest "repeated request", followed by `/exit`, `/clear`
/// and `[Request interrupted by user]`. All of it is harness plumbing.
/// Clustering it is not merely useless — it buries the real signal
/// under synthetic turns that no automation could ever remove.
const HARNESS_MARKERS: &[&str] = &[
    "<local-command-",
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<system-reminder>",
    "[request interrupted by user",
    "caveat: the messages below were generated",
    "<user-prompt-submit-hook>",
    // Second pass over the real corpus surfaced this one: the echoed
    // output of a `!`-prefixed shell command, 98 times across 23
    // projects.
    "<bash-stdout>",
    "<bash-stderr>",
    "<bash-input>",
];

/// Whether this turn is harness plumbing rather than user intent.
///
/// Checked against a lowercased prefix: these markers open the turn, and
/// scanning the whole body would reject any turn that merely *quotes*
/// one — including a user asking about them.
pub fn is_harness_synthetic(text: &str) -> bool {
    let head: String = text.trim_start().to_lowercase().chars().take(200).collect();
    HARNESS_MARKERS.iter().any(|m| head.starts_with(m))
}

/// What "the same operation" means for pairing a failure with its
/// recovery.
///
/// For `Bash` this is the first *meaningful* command word, which is not
/// the first word: real commands routinely look like
/// `cd "<some dir>"; echo "…"; cat notes.md`, so a naive first-token
/// split yields `cd` for a large share of the corpus and would pair a
/// failed `cargo test` with a successful `cd`. Segments are split on
/// `;`, `&&`, `||`, `|` and newlines, leading `VAR=value` assignments
/// and [`SHELL_NOISE`] are skipped, and the result is reduced to a
/// basename so `/usr/bin/git` and `git` agree.
///
/// For file-taking tools the family is the file's basename — a failed
/// `Edit` on `foo.rs` is recovered by a successful `Edit` on `foo.rs`,
/// not by one on an unrelated file.
///
/// Everything else falls back to the tool name.
pub fn command_family(tool_name: &str, tool_input_json: Option<&str>) -> String {
    let input: Option<serde_json::Value> =
        tool_input_json.and_then(|j| serde_json::from_str(j).ok());

    match tool_name {
        "Bash" => {
            let cmd = input
                .as_ref()
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("bash:{}", first_meaningful_word(cmd))
        }
        "Edit" | "Write" | "Read" | "NotebookEdit" => {
            let path = input
                .as_ref()
                .and_then(|v| v.get("file_path").or_else(|| v.get("path")))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{}:{}", tool_name.to_lowercase(), basename(path))
        }
        other => other.to_lowercase(),
    }
}

fn first_meaningful_word(cmd: &str) -> String {
    'segments: for segment in cmd.split(['\n', ';', '|', '&']) {
        for word in segment.split_whitespace() {
            let w = word.trim_matches(|c| c == '(' || c == ')' || c == '"' || c == '\'');
            if w.is_empty() {
                continue;
            }
            // `VAR=value` prefix — not the command.
            if w.contains('=') && !w.starts_with('-') {
                continue;
            }
            if w.starts_with('-') {
                continue;
            }
            // A comment: everything after it in this segment is prose,
            // not a command. Without this the corpus reports `bash:#`
            // as a top command family — 266 "verified recoveries" that
            // are two comment lines either side of unrelated work.
            if w.starts_with('#') {
                continue 'segments;
            }
            let b = basename(w);
            if SHELL_SEGMENT_CONSUMERS.contains(&b.as_str()) {
                // Its argument is not a command. Abandon the segment.
                continue 'segments;
            }
            if SHELL_PREFIX.contains(&b.as_str()) {
                continue;
            }
            if !b.is_empty() {
                return b;
            }
        }
    }
    String::new()
}

/// Last path segment, separator-agnostic (`rules/paths.md` — never
/// hardcode `/`).
fn basename(p: &str) -> String {
    p.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .trim_matches('"')
        .to_string()
}

/// Token-set Jaccard over normalized text. Used where "did the user ask
/// for the same thing again" needs to tolerate rewording, rather than
/// demanding an exact fingerprint match.
pub fn jaccard(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let sa: HashSet<&str> = a.split(' ').filter(|t| !t.is_empty()).collect();
    let sb: HashSet<&str> = b.split(' ').filter(|t| !t.is_empty()).collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    inter / union
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_uuids_and_hashes_normalize_away() {
        let a = normalize_prompt("fix /Users/joker/repo/src/main.rs at line 42");
        let b = normalize_prompt("fix /Users/other/proj/src/main.rs at line 7");
        assert_eq!(a, b, "the same request about different paths must cluster");
    }

    /// `rules/paths.md`: a Windows path is a path.
    #[test]
    fn windows_and_unc_paths_normalize_too() {
        let unix = normalize_prompt("read /Users/joker/x/y.rs");
        let win = normalize_prompt(r"read C:\Users\joker\x\y.rs");
        let unc = normalize_prompt(r"read \\server\share\x\y.rs");
        assert_eq!(unix, win);
        assert_eq!(unix, unc);
    }

    #[test]
    fn timestamps_survive_as_a_shape_not_digits() {
        let s = normalize_prompt("failed at 2026-04-10T10:00:00Z");
        assert!(s.contains("<ts>"), "got {s}");
    }

    // ─── error signatures ───────────────────────────────────────────

    /// The whole point: different exit codes are different failures.
    #[test]
    fn exit_codes_are_preserved() {
        let a = error_signature("Exit code 1\nsome stdout").unwrap();
        let b = error_signature("Exit code 143\nCommand timed out").unwrap();
        assert_ne!(a, b, "a failure and a timeout must not merge");
        assert!(a.contains('1'));
    }

    /// Payload after the first line is noise — and on a real machine it
    /// contains bank statements.
    #[test]
    fn only_the_first_line_becomes_the_signature() {
        let sig = error_signature(
            "Exit code 1\n===== statement =====\naccount 6222021234567890 balance 12345",
        )
        .unwrap();
        assert!(!sig.contains("6222021234567890"));
        assert!(!sig.contains("balance"));
        assert!(sig.starts_with("exit code 1"));
    }

    #[test]
    fn the_same_failure_from_different_paths_shares_a_signature() {
        let a = error_signature("error: could not compile /Users/a/x.rs").unwrap();
        let b = error_signature("error: could not compile /Users/b/y.rs").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn empty_or_blank_output_has_no_signature() {
        assert!(error_signature("").is_none());
        assert!(error_signature("   \n  \n").is_none());
    }

    #[test]
    fn signatures_are_capped() {
        let long = format!("error: {}", "x".repeat(500));
        assert!(error_signature(&long).unwrap().chars().count() <= SIGNATURE_CAP);
    }

    // ─── command families ───────────────────────────────────────────

    /// The precision fix. Real Bash commands in this corpus start with
    /// `cd "…"`; taking the first token would pair a failed build with
    /// a successful directory change.
    #[test]
    fn bash_family_skips_cd_and_env_prefixes() {
        let j = r#"{"command":"cd \"/Users/joker/repo\"; cargo test --workspace"}"#;
        assert_eq!(command_family("Bash", Some(j)), "bash:cargo");

        let j2 = r#"{"command":"RUST_LOG=debug cargo build"}"#;
        assert_eq!(command_family("Bash", Some(j2)), "bash:cargo");

        let j3 = r#"{"command":"sudo /usr/bin/git status"}"#;
        assert_eq!(command_family("Bash", Some(j3)), "bash:git");
    }

    #[test]
    fn bash_family_reduces_absolute_commands_to_basenames() {
        let j = r#"{"command":"/opt/homebrew/bin/pnpm test"}"#;
        assert_eq!(command_family("Bash", Some(j)), "bash:pnpm");
    }

    #[test]
    fn file_tools_are_keyed_by_basename() {
        let j = r#"{"file_path":"/Users/joker/repo/src/main.rs"}"#;
        assert_eq!(command_family("Edit", Some(j)), "edit:main.rs");
        // Same file, different checkout — still the same family.
        let j2 = r#"{"file_path":"/tmp/other/src/main.rs"}"#;
        assert_eq!(command_family("Edit", Some(j2)), "edit:main.rs");
    }

    /// Regression from the first real run: `bash:#` came back as a top
    /// command family with 266 "verified recoveries" — comment lines
    /// paired either side of unrelated work.
    #[test]
    fn a_comment_is_not_a_command_family() {
        // `r##"…"##` because the payload contains `"#`, which would
        // close a single-hash raw string mid-literal.
        let j = r##"{"command":"# rebuild everything\ncargo build"}"##;
        assert_eq!(command_family("Bash", Some(j)), "bash:cargo");

        let only_comment = r##"{"command":"# just a note"}"##;
        assert_eq!(command_family("Bash", Some(only_comment)), "bash:");
    }

    // ─── harness plumbing ───────────────────────────────────────────

    /// Regression from the first real run: the single largest
    /// "repeated request" in the corpus was `<local-command-caveat>…`,
    /// 1,258 times across 69 projects. The user never typed any of it.
    #[test]
    fn harness_injected_turns_are_recognised() {
        for s in [
            "<local-command-caveat>Caveat: The messages below were generated…",
            "<command-name>/exit</command-name>",
            "<command-message>clear</command-message>",
            "[Request interrupted by user]",
            "<system-reminder>\nSomething\n</system-reminder>",
            "Caveat: The messages below were generated by the user while running",
        ] {
            assert!(is_harness_synthetic(s), "should be synthetic: {s}");
        }
    }

    /// ...but a real prompt that merely *mentions* one is still a real
    /// prompt. Matching anywhere in the body would silently drop it.
    #[test]
    fn a_user_asking_about_harness_text_is_not_synthetic() {
        assert!(!is_harness_synthetic(
            "why does <system-reminder> keep showing up in my transcripts?"
        ));
        assert!(!is_harness_synthetic("commit and push"));
        assert!(!is_harness_synthetic(""));
    }

    #[test]
    fn unknown_tools_fall_back_to_their_name() {
        assert_eq!(command_family("WebFetch", None), "webfetch");
    }

    #[test]
    fn malformed_input_json_does_not_panic() {
        assert_eq!(command_family("Bash", Some("not json")), "bash:");
        assert_eq!(command_family("Bash", None), "bash:");
    }

    #[test]
    fn jaccard_scores_overlap() {
        assert!(jaccard("run the tests", "run the tests") > 0.99);
        // {run,the} shared of {run,the,tests,test,suite} = exactly 0.4.
        assert!(jaccard("run the tests", "run the test suite") >= 0.4);
        assert!(jaccard("run the tests", "deploy to prod") < 0.2);
        assert_eq!(jaccard("", "x"), 0.0);
    }
}
