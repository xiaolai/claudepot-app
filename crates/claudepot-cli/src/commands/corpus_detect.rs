//! `claudepot corpus detect` — run the deterministic detectors and
//! report what they found, without spending anything.
//!
//! This is the preview the plan's activation flow needs: a count of
//! real candidates *before* a model is invoked. Nothing here calls an
//! LLM; every number is a SQL read plus pure comparison.

use anyhow::{Context, Result};

use claudepot_core::corpus::detect::{self, Findings};
use claudepot_core::corpus::{self, CorpusIndex};

use crate::output::print_json;
use crate::AppContext;

fn open() -> Result<CorpusIndex> {
    CorpusIndex::open(&corpus::default_path()).context("open corpus.db")
}

pub fn detect_cmd(ctx: &AppContext, limit: usize) -> Result<()> {
    let idx = open()?;
    let f: Findings = detect::detect_all(&idx, limit)?;

    let resolved = f.resolved_incidents();
    let unresolved = f.incidents.len() - resolved;

    if ctx.json {
        return print_json(&serde_json::json!({
            "incidents": f.incidents.len(),
            "resolved": resolved,
            "unresolved": unresolved,
            "repetitions": f.repetitions.len(),
            "corrections": f.corrections.len(),
            "top_repetitions": f.repetitions.iter().take(10).collect::<Vec<_>>(),
        }));
    }

    println!(
        "{} incident(s) — {} with an observed recovery, {} unresolved.",
        f.incidents.len(),
        resolved,
        unresolved
    );
    println!(
        "{} repetition cluster(s), {} corroborated correction(s).",
        f.repetitions.len(),
        f.corrections.len()
    );

    if !f.repetitions.is_empty() {
        println!("\nMost repeated requests (automation candidates, not lessons):");
        for r in f.repetitions.iter().take(8) {
            let sample: String = r.sample.chars().take(60).collect();
            println!(
                "  {:>4}x  {:>2} proj  {}",
                r.count,
                r.projects,
                sample.replace('\n', " ")
            );
        }
    }

    // Resolved incidents are the highest-value distillation candidates:
    // a failure with an observed fix is the one shape that can support a
    // guard. Show the families they cluster into.
    if resolved > 0 {
        use std::collections::HashMap;
        let mut by_family: HashMap<&str, usize> = HashMap::new();
        for i in f.incidents.iter().filter(|i| i.is_resolved()) {
            *by_family.entry(i.family.as_str()).or_default() += 1;
        }
        let mut v: Vec<_> = by_family.into_iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("\nVerified recoveries by command family:");
        for (fam, n) in v.into_iter().take(8) {
            println!("  {n:>4}x  {fam}");
        }
    }
    Ok(())
}

/// `claudepot corpus interaction-demand` — plan §10.2.
///
/// Answers "how often does a human need to tell an agent something text
/// cannot carry?" by counting the *residue* of the text channel being
/// strained, not by counting rich questions agents never had a way to
/// ask. See `corpus::interaction_demand` for why that distinction is
/// the whole point of the detector.
pub fn interaction_demand_cmd(ctx: &AppContext, limit: usize) -> Result<()> {
    use claudepot_core::corpus::interaction_demand::{self, Signal};

    let idx = open()?;
    let report = interaction_demand::detect_demand(&idx, limit)?;

    let all = [
        Signal::ClarificationChain,
        Signal::PastedStructure,
        Signal::SpatialGesture,
        Signal::AmbiguousAnswer,
    ];

    if ctx.json {
        return print_json(&serde_json::json!({
            "examined": report.examined,
            "total": report.total(),
            "rate_per_1k": report.rate_per_1k(),
            "counts": all.iter().map(|s| (s.as_str(), report.count(*s)))
                .collect::<std::collections::BTreeMap<_, _>>(),
            "samples": report.signals.iter().take(20).map(|s| serde_json::json!({
                "signal": s.signal.as_str(),
                "session_id": s.session_id,
                "project_path": s.project_path,
                "turn_index": s.turn_index,
                "turns": s.turns,
                "sample": s.sample,
            })).collect::<Vec<_>>(),
        }));
    }

    if report.examined == 0 {
        // Render-if-nonzero: an empty corpus says so once rather than
        // printing a table of zeros that looks like a finding.
        eprintln!("corpus is empty — run `claudepot corpus index` first");
        return Ok(());
    }

    println!(
        "{} exchange(s) examined (harness plumbing excluded).",
        report.examined
    );
    for s in all {
        let n = report.count(s);
        if n == 0 {
            continue;
        }
        println!("  {:<22} {:>6}  — {}", s.as_str(), n, s.implication());
    }
    println!(
        "\n{:.2} signal(s) per 1,000 exchanges.",
        report.rate_per_1k()
    );
    println!(
        "\nThis is a LOWER bound: it only fires where someone pushed through\n\
         the limitation rather than abandoning the question. It does not\n\
         count rich questions agents never had a channel to ask."
    );
    Ok(())
}
