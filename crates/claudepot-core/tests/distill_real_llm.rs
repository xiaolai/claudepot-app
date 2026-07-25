//! Real-LLM end-to-end test for the manual distiller path.
//!
//! `shared_memory::distill::distill_transcript` spawns `claude -p` with
//! `--output-format json --json-schema <schema>` and feeds the stdout to
//! `proposal::parse_claims`. Every layer of that is unit-tested against
//! captured fixtures; this test is the one that proves the *live* CLI
//! still accepts those flags and still emits a shape the parser reads.
//!
//! It exists because the flags were the bug. The manual path shipped
//! without `--output-format`/`--json-schema` while the scheduled agent
//! had both, and no test compared them — a fixture-only suite would have
//! stayed green through the entire defect.
//!
//! ## Cost & gating
//!
//! One `claude -p` call on Haiku: a few cents inside a warm prompt cache,
//! ~$0.06 cold. Gated two ways, matching `templates_real_llm.rs`:
//!
//! 1. `#[ignore]` — excluded from a default `cargo test`.
//! 2. A real transcript must exist under `~/.claude/projects/`. Without
//!    one the test skips with a printed note rather than failing, so a
//!    CI runner with no CC history stays green.
//!
//! The run writes to a `TempDir` SessionIndex, never to the user's
//! `~/.claudepot/sessions.db`.
//!
//! Run with:
//!   cargo test -p claudepot-core --test distill_real_llm -- --ignored --nocapture

use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::distill::distill_transcript;
use claudepot_core::shared_memory::review;
use tempfile::TempDir;

/// A transcript big enough to hold a lesson but small enough to distill
/// cheaply. `None` (and a printed skip) when the machine has no CC
/// history.
fn sample_transcript(test_name: &str) -> Option<std::path::PathBuf> {
    let root = dirs::home_dir()?.join(".claude").join("projects");
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    for project in std::fs::read_dir(&root).ok()?.flatten() {
        let Ok(entries) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Ok(len) = e.metadata().map(|m| m.len()) else {
                continue;
            };
            // 20–80 KB: a real session, not a one-turn stub, and not a
            // multi-megabyte epic that costs real money to read.
            if (20_000..80_000).contains(&len) && best.as_ref().is_none_or(|(b, _)| len < *b) {
                best = Some((len, p));
            }
        }
    }
    match best {
        Some((_, p)) => Some(p),
        None => {
            eprintln!("{test_name}: skipping — no 20–80 KB transcript under ~/.claude/projects");
            None
        }
    }
}

/// The whole chain against the live CLI: flags accepted → CC runs →
/// stdout parses → rows land (or the harvest is legitimately empty).
///
/// The assertion is deliberately *not* "at least one claim". Most
/// sessions teach nothing, and the distiller returning `{"claims":[]}`
/// is correct behavior — asserting otherwise would make this test fail
/// on the model's judgment rather than on our contract. What must hold
/// is that the call succeeds and nothing was unreadable: a non-zero
/// `malformed_claims` means CC emitted a shape the parser could not
/// read, which is exactly the regression this guards.
#[test]
#[ignore = "spawns a real `claude -p` call; costs money"]
fn the_manual_distiller_path_works_against_the_live_cli() {
    let Some(transcript) =
        sample_transcript("the_manual_distiller_path_works_against_the_live_cli")
    else {
        return;
    };
    eprintln!("distilling: {}", transcript.display());

    let tmp = TempDir::new().unwrap();
    let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();

    let report = distill_transcript(
        &idx,
        "claude",
        "/tmp/distill-e2e",
        &transcript.to_string_lossy(),
        "test:distill-real-llm",
    )
    .expect("the live distiller path must succeed");

    eprintln!("report: {report:?}");

    assert_eq!(
        report.malformed_claims, 0,
        "CC emitted claim(s) the parser could not read — the schema \
         contract in distiller_flags() has drifted from CC's output"
    );

    // Whatever was proposed must be a real, reviewable row: inert until
    // a human accepts it. An empty harvest is fine and asserts nothing.
    let counts = review::counts(&idx, None).expect("counts");
    assert_eq!(
        counts.proposed, report.proposed as i64,
        "every proposal landed as a reviewable row"
    );
    assert_eq!(counts.accepted, 0, "nothing bypassed review");
    assert_eq!(counts.enforced, 0, "nothing compiled itself into a guard");
}
