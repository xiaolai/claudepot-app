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
