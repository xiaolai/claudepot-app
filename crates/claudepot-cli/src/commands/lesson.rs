//! The `lesson` verb group — the knowledge compiler's CLI surface.
//!
//! `harvest` mines settled sessions for lessons; `list` shows the
//! triage queue; `accept` / `reject` are the human's entire job.
//!
//! # Why this is not under the `memory` noun
//!
//! `claudepot memory` already means "the CLAUDE.md files in a project".
//! These verbs operate on rows in `sessions.db`'s `memories` table —
//! distilled claims with a review state. Same word, different thing;
//! overloading the noun would make both harder to explain.
//!
//! # Why the CLI can do the whole loop
//!
//! Triage has to cost seconds or it does not happen. A user already in a
//! terminal should not have to open a GUI to say yes or no to four
//! claims. The GUI surface is the same data.

use anyhow::{bail, Context, Result};

use claudepot_core::agent::templates::KNOWLEDGE_DISTILLER_MODEL;
use claudepot_core::paths;
use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::review::{self, ReviewState};
use claudepot_core::shared_memory::{compile, distill, git, proposal};

use crate::output::print_json;
use crate::AppContext;

/// Cost per distilled session, in USD. Haiku, ~1 large transcript in,
/// a few hundred tokens out. Used only to warn before a bulk spend.
const EST_COST_PER_SESSION_USD: f64 = 0.04;

fn open_index() -> Result<SessionIndex> {
    let db = paths::claudepot_data_dir().join("sessions.db");
    SessionIndex::open(&db).context("open sessions.db")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ─── list ────────────────────────────────────────────────────────

pub fn list_cmd(
    ctx: &AppContext,
    project: Option<&str>,
    state: Option<&str>,
    limit: u32,
) -> Result<()> {
    let idx = open_index()?;
    let state = match state {
        Some(s) => Some(ReviewState::parse(s).with_context(|| {
            format!("unknown state {s:?} (proposed|accepted|rejected|suspect)")
        })?),
        // The queue is what you came for.
        None => Some(ReviewState::Proposed),
    };
    let rows = review::list(&idx, project, state, limit)?;

    if ctx.json {
        return print_json(&rows);
    }
    if rows.is_empty() {
        println!("Nothing to review.");
        return Ok(());
    }
    for r in &rows {
        let conf = r.confidence.map(|c| format!(" · {c}%")).unwrap_or_default();
        println!("\n\x1b[1m{}\x1b[0m  [{}{}]", r.content, r.kind, conf);
        if let Some(d) = &r.directive {
            println!("  → {d}");
        }
        if let Some(reason) = &r.suspect_reason {
            println!("  \x1b[33m! {reason}\x1b[0m");
        }
        if let Some(a) = &r.anchor_json {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(a) {
                if let Some(e) = v.get("evidence").and_then(|x| x.as_str()) {
                    println!("  because: {e}");
                }
            }
        }
        println!("  \x1b[2m{}\x1b[0m", r.id);
    }
    println!(
        "\n{} to review. Accept: claudepot lesson accept <id>   Reject: claudepot lesson reject <id>",
        rows.len()
    );
    Ok(())
}

// ─── accept / reject ─────────────────────────────────────────────

pub fn accept_cmd(ctx: &AppContext, id: &str, no_anchor: bool) -> Result<()> {
    let idx = open_index()?;
    // Resolve the lesson first so we anchor to ITS project's HEAD, not
    // whatever repo the CLI happens to be run from. Accepting a lesson
    // that belongs to another project while cd'd elsewhere would stamp
    // the wrong commit (or none), silently breaking invalidation.
    let lesson = review::list(&idx, None, Some(ReviewState::Proposed), 1000)?
        .into_iter()
        .chain(review::list(&idx, None, Some(ReviewState::Suspect), 1000)?)
        .find(|r| r.id == id)
        .with_context(|| format!("no lesson awaiting review with id {id}"))?;

    // Stamp the commit the anchored files are at right now, in the
    // lesson's own project. That pair — files + commit — is what lets us
    // tell you later that this claim may no longer be true.
    let commit = if no_anchor {
        None
    } else {
        lesson
            .project_path
            .as_deref()
            .and_then(|p| git::head_commit(std::path::Path::new(p)))
    };
    let ok = review::accept(&idx, id, commit.as_deref(), now_ms())?;
    if !ok {
        bail!("no such lesson: {id}");
    }
    if ctx.json {
        return print_json(&serde_json::json!({ "accepted": id, "anchor_commit": commit }));
    }
    match &commit {
        Some(sha) => println!(
            "Accepted. Anchored at {}. If the files it depends on change, \
             it comes back for re-review.",
            &sha[..sha.len().min(8)]
        ),
        None => println!("Accepted (no anchor — it will never be re-checked)."),
    }
    Ok(())
}

pub fn reject_cmd(ctx: &AppContext, id: &str) -> Result<()> {
    let idx = open_index()?;
    if !review::reject(&idx, id, now_ms())? {
        bail!("no such lesson: {id}");
    }
    if ctx.json {
        return print_json(&serde_json::json!({ "rejected": id }));
    }
    // Say the non-obvious part: rejection is remembered, not discarded.
    println!("Rejected. It won't be proposed again.");
    Ok(())
}

// ─── counts ──────────────────────────────────────────────────────

pub fn status_cmd(ctx: &AppContext, project: Option<&str>) -> Result<()> {
    let idx = open_index()?;
    let c = review::counts(&idx, project)?;
    if ctx.json {
        return print_json(&c);
    }
    println!("  {:>4}  to review", c.proposed);
    println!(
        "  {:>4}  accepted ({} enforced by a check)",
        c.accepted, c.enforced
    );
    if c.suspect > 0 {
        println!(
            "  {:>4}  \x1b[33msuspect — the code they relied on changed\x1b[0m",
            c.suspect
        );
    }
    if c.rejected > 0 {
        println!("  {:>4}  rejected", c.rejected);
    }
    Ok(())
}

// ─── harvest ─────────────────────────────────────────────────────

pub struct HarvestArgs {
    pub project: Option<String>,
    pub limit: u32,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn harvest_cmd(ctx: &AppContext, args: HarvestArgs) -> Result<()> {
    let idx = open_index()?;
    let project = match args.project {
        Some(p) => p,
        None => std::env::current_dir()?.to_string_lossy().into_owned(),
    };

    // Only sessions we have not already distilled. Re-mining a
    // transcript costs money and yields duplicates that ingest throws
    // away — so don't pay for them twice.
    let targets = review::undistilled_sessions(&idx, &project, args.limit)?;
    if targets.is_empty() {
        if ctx.json {
            return print_json(&serde_json::json!({ "harvested": 0, "reason": "nothing new" }));
        }
        println!("No new sessions to harvest in {project}.");
        return Ok(());
    }

    let cost = targets.len() as f64 * EST_COST_PER_SESSION_USD;
    if args.dry_run {
        if ctx.json {
            return print_json(&serde_json::json!({
                "would_harvest": targets.len(),
                "est_cost_usd": cost,
            }));
        }
        println!(
            "Would distill {} session(s) — roughly ${:.2}. Re-run without --dry-run.",
            targets.len(),
            cost
        );
        return Ok(());
    }
    if !args.yes && !ctx.yes {
        // Spending the user's money is the one thing that must never be
        // a surprise.
        println!(
            "About to distill {} session(s) with {} — roughly ${:.2}.",
            targets.len(),
            KNOWLEDGE_DISTILLER_MODEL,
            cost
        );
        println!("Re-run with --yes to proceed, or --dry-run to see the list.");
        return Ok(());
    }

    let mut total = proposal::IngestReport::default();
    let mut failed = 0u32;
    for (i, path) in targets.iter().enumerate() {
        if !ctx.quiet {
            eprintln!("[{}/{}] {}", i + 1, targets.len(), short(path));
        }
        match distill::distill_transcript(&idx, "claude", &project, path, "cli:lesson-harvest")
            .map_err(anyhow::Error::from)
        {
            Ok(r) => {
                total.proposed += r.proposed;
                total.skipped_duplicate += r.skipped_duplicate;
                total.skipped_low_confidence += r.skipped_low_confidence;
                total.skipped_too_long += r.skipped_too_long;
                total.skipped_empty += r.skipped_empty;
                total.recurrences_detected += r.recurrences_detected;
            }
            Err(e) => {
                // One bad transcript must not abort a 100-session
                // harvest. Report it and move on.
                failed += 1;
                eprintln!("      failed: {e:#}");
            }
        }
    }

    if ctx.json {
        return print_json(&serde_json::json!({
            "sessions": targets.len(),
            "failed": failed,
            "proposed": total.proposed,
            "skipped": total.total_skipped(),
            "recurrences_detected": total.recurrences_detected,
        }));
    }
    println!("\n{} lesson(s) filed for review.", total.proposed);
    if total.total_skipped() > 0 {
        // Never silently drop things. A harvest that says "0 proposed"
        // with no explanation looks broken.
        println!(
            "  ({} skipped: {} duplicate, {} low-confidence, {} over-long, {} empty)",
            total.total_skipped(),
            total.skipped_duplicate,
            total.skipped_low_confidence,
            total.skipped_too_long,
            total.skipped_empty
        );
    }
    if total.recurrences_detected > 0 {
        // A recurrence is the failure the compiler exists to prevent —
        // never bury it in the skipped line. Confirm them in the GUI's
        // Review tab (or wherever the recurrence surface lives).
        println!(
            "  ⟳ {} recurrence(s) detected — a class you already learned recurred. \
             Confirm in Review.",
            total.recurrences_detected
        );
    }
    if failed > 0 {
        println!("  ({failed} session(s) failed — see above)");
    }
    if total.proposed > 0 {
        println!("\nReview them:  claudepot lesson list");
    }
    Ok(())
}

fn short(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

// ─── compile → guard ─────────────────────────────────────────────

pub struct CompileArgs {
    pub id: String,
    /// Write the guard into scripts/repo-invariants.sh. Without this, the
    /// proposed block is printed for review and nothing is changed.
    pub write: bool,
}

pub fn compile_cmd(ctx: &AppContext, args: CompileArgs) -> Result<()> {
    let idx = open_index()?;
    // Only an accepted lesson can be compiled. Compiling a proposal would
    // put an unreviewed claim into a binding check — the exact thing the
    // review gate exists to prevent.
    let rows = review::list(&idx, None, Some(ReviewState::Accepted), 500)?;
    let lesson = rows
        .iter()
        .find(|r| r.id == args.id)
        .with_context(|| format!("no accepted lesson with id {}", args.id))?;

    let directive = lesson.directive.clone().unwrap_or_default();
    if directive.is_empty() {
        bail!("this lesson has no directive to compile");
    }

    // The model proposes the guard as structured fields — it never writes
    // shell. A model that could emit arbitrary shell could emit `rm -rf`;
    // a model that fills a struct cannot.
    let spec = compile::propose_guard("claude", &lesson.content, &directive, &args.id)?;

    // The guard belongs in the LESSON's project repo, not whatever repo
    // the CLI is run from — a lesson mined in project B must not have its
    // guard written into project A's repo-invariants.sh. Staging locates
    // that repo's scripts/repo-invariants.sh and splices in memory;
    // nothing is written until `install`.
    let project = lesson
        .project_path
        .clone()
        .context("this lesson has no project_path; cannot locate its repo-invariants.sh")?;
    let staged = compile::stage_guard(&project, &spec)?;

    if !args.write {
        if ctx.json {
            return print_json(&serde_json::json!({ "proposed": spec, "block": spec.render() }));
        }
        println!("Proposed guard for lesson {}:\n", args.id);
        println!("{}", spec.render());
        println!(
            "Run with --write to add it to {}.",
            staged.script_path.display()
        );
        println!("(It will be kept only if it does NOT fire on the current clean tree.)");
        return Ok(());
    }

    // The write→verify→revert transaction: baseline the script, write the
    // guard, and keep it only if the clean tree stays clean.
    staged.install()?;

    let guard_ref = format!(
        "{}:{}",
        staged
            .script_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo-invariants.sh"),
        spec.slug
    );
    review::mark_compiled(&idx, &args.id, "guard", &guard_ref, now_ms())?;

    if ctx.json {
        return print_json(&serde_json::json!({ "compiled": args.id, "guard_ref": guard_ref }));
    }
    println!(
        "Guard added to {} and verified clean.",
        staged.script_path.display()
    );
    println!(
        "Review the diff:  git diff {}",
        staged.script_path.display()
    );
    Ok(())
}
