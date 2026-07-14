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

use claudepot_core::agent::templates::{KNOWLEDGE_DISTILLER_MODEL, KNOWLEDGE_DISTILLER_PROMPT};
use claudepot_core::paths;
use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::proposal::{self, ProposalOrigin};
use claudepot_core::shared_memory::review::{self, ReviewState};

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
    // Stamp the commit the anchored files are at right now. That pair —
    // files + commit — is what lets us tell you later that this claim
    // may no longer be true.
    let commit = if no_anchor { None } else { head_commit() };
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
        match distill_one(&idx, &project, path) {
            Ok(r) => {
                total.proposed += r.proposed;
                total.skipped_duplicate += r.skipped_duplicate;
                total.skipped_low_confidence += r.skipped_low_confidence;
                total.skipped_too_long += r.skipped_too_long;
                total.skipped_empty += r.skipped_empty;
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
        }));
    }
    println!("\n{} lesson(s) filed for review.", total.proposed);
    if total.total_skipped() > 0 {
        // Never silently drop things. A harvest that says "0 proposed"
        // with no explanation looks broken.
        println!(
            "  ({} skipped: {} duplicate, {} low-confidence, {} over-long)",
            total.total_skipped(),
            total.skipped_duplicate,
            total.skipped_low_confidence,
            total.skipped_too_long
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

/// Run the distiller over one transcript and file whatever it finds.
fn distill_one(idx: &SessionIndex, project: &str, path: &str) -> Result<proposal::IngestReport> {
    let out = std::process::Command::new("claude")
        .arg("-p")
        .arg(format!(
            "{KNOWLEDGE_DISTILLER_PROMPT}\n\nThe transcript is at: {path}\n\n\
             Output ONLY a JSON object of the form {{\"claims\":[...]}}. No prose."
        ))
        .args(["--model", KNOWLEDGE_DISTILLER_MODEL])
        .args(["--allowedTools", "Read,Grep"])
        .env("CLAUDEPOT_EVENT_SESSION_PATH", path)
        .stdin(std::process::Stdio::null())
        .output()
        .context("spawn `claude -p` for the distiller")?;
    if !out.status.success() {
        bail!(
            "claude -p exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let claims = proposal::parse_claims(&raw).context("parse the distiller's output")?;

    let origin = ProposalOrigin {
        project_path: project,
        file_path: Some(path),
        exchange_id: None,
        created_by: "cli:lesson-harvest",
    };
    Ok(proposal::ingest_proposals(idx, &claims, &origin, now_ms())?)
}

fn head_commit() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn short(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

// ─── compile → guard ─────────────────────────────────────────────

use claudepot_core::shared_memory::guard::{self, GuardSpec};

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
    let spec = propose_guard(&lesson.content, &directive, &args.id)?;

    let script_path = repo_invariants_path()?;
    let script = std::fs::read_to_string(&script_path)
        .with_context(|| format!("read {}", script_path.display()))?;
    let spliced = guard::splice_into_script(&script, &spec).context("splice guard block")?;

    if !args.write {
        if ctx.json {
            return print_json(&serde_json::json!({ "proposed": spec, "block": spec.render() }));
        }
        println!("Proposed guard for lesson {}:\n", args.id);
        println!("{}", spec.render());
        println!("Run with --write to add it to {}.", script_path.display());
        println!("(It will be kept only if it does NOT fire on the current clean tree.)");
        return Ok(());
    }

    // Write, then PROVE it doesn't false-positive. A generated guard that
    // trips on the current clean tree is wrong by construction — the
    // lesson was already fixed, so its anti-pattern must be absent now. If
    // it fires, the pattern is bad: revert and refuse to keep it.
    std::fs::write(&script_path, &spliced)
        .with_context(|| format!("write {}", script_path.display()))?;
    let clean = run_invariants(&script_path)?;
    if !clean {
        std::fs::write(&script_path, &script).context("revert bad guard")?;
        bail!(
            "the generated guard fires on the current clean tree — its detect pattern is wrong \
             (it would block every push). Reverted. Pattern was: {}",
            spec.detect_regex
        );
    }

    let guard_ref = format!(
        "{}:{}",
        script_path
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
        script_path.display()
    );
    println!("Review the diff:  git diff {}", script_path.display());
    Ok(())
}

/// Ask the model to fill in a `GuardSpec` from a lesson. Structured
/// output; the model never emits shell.
fn propose_guard(claim: &str, directive: &str, lesson_id: &str) -> Result<GuardSpec> {
    let schema = r#"{
      "type":"object",
      "properties":{
        "slug":{"type":"string","description":"kebab-case id, e.g. no-bare-canonicalize"},
        "rationale":{"type":"string","description":"one line: what this enforces and why"},
        "detect_regex":{"type":"string","description":"grep -E regex whose PRESENCE is the violation. Must NOT match already-correct code."},
        "include_globs":{"type":"array","items":{"type":"string"},"description":"file globs, e.g. *.rs"},
        "allow_substrings":{"type":"array","items":{"type":"string"},"description":"path substrings that are legitimate exceptions"},
        "message":{"type":"string","description":"what to print when it fires; tell the reader what to do"},
        "compilable":{"type":"boolean","description":"false if this lesson cannot be a grep tripwire"}
      },
      "required":["slug","rationale","detect_regex","message","compilable"]
    }"#;
    let prompt = format!(
        "You turn an accepted engineering lesson into a grep-based CI tripwire.\n\n\
         LESSON: {claim}\nDIRECTIVE: {directive}\n\n\
         Produce a GuardSpec matching the schema. The detect_regex matches the ANTI-pattern \
         (its presence is the bug). It MUST NOT match already-correct code, because the codebase \
         is currently clean. If the lesson is about human judgment, prose, or anything a grep \
         cannot detect, set compilable=false and leave detect_regex empty. Output ONLY the JSON object.\n\n\
         Schema: {schema}"
    );
    let out = std::process::Command::new("claude")
        .arg("-p")
        .arg(prompt)
        .args(["--model", "claude-haiku-4-5"])
        .stdin(std::process::Stdio::null())
        .output()
        .context("spawn `claude -p` to propose a guard")?;
    if !out.status.success() {
        bail!(
            "claude -p failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let obj = first_json(&raw).context("model did not return a JSON object")?;
    // Models routinely emit regexes with backslashes that aren't valid
    // JSON escapes (`\.`, `\(`, `\d`), which serde rejects outright. This
    // is the single most common way a guard proposal fails to parse, and
    // it's a packaging problem, not a content one — repair it.
    let repaired = repair_json_escapes(obj);
    let v: serde_json::Value = serde_json::from_str(&repaired).context("parse guard proposal")?;

    if !v
        .get("compilable")
        .and_then(|c| c.as_bool())
        .unwrap_or(false)
    {
        bail!(
            "the model judged this lesson not compilable to a grep guard \
             (it needs human judgment or isn't a detectable pattern). Keep it as a directive or a note."
        );
    }
    Ok(GuardSpec {
        slug: sanitize_slug(
            v.get("slug")
                .and_then(|s| s.as_str())
                .unwrap_or("generated-guard"),
        ),
        rationale: v
            .get("rationale")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        detect_regex: v
            .get("detect_regex")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .context("model returned compilable=true but no detect_regex")?
            .to_string(),
        include_globs: str_array(&v, "include_globs"),
        allow_substrings: str_array(&v, "allow_substrings"),
        message: v
            .get("message")
            .and_then(|s| s.as_str())
            .unwrap_or("guard fired")
            .to_string(),
        source_lesson_id: lesson_id.to_string(),
    })
}

fn repo_invariants_path() -> Result<std::path::PathBuf> {
    let root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .context("not in a git repository — guards live in scripts/repo-invariants.sh")?;
    let p = std::path::Path::new(&root).join("scripts/repo-invariants.sh");
    if !p.exists() {
        bail!(
            "{} does not exist — guards are a repo-invariants.sh feature",
            p.display()
        );
    }
    Ok(p)
}

fn run_invariants(script: &std::path::Path) -> Result<bool> {
    let out = std::process::Command::new("bash")
        .arg(script)
        .output()
        .context("run repo-invariants.sh")?;
    Ok(out.status.success())
}

fn sanitize_slug(s: &str) -> String {
    let cleaned: String = s
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = cleaned.trim_matches('-').to_string();
    if slug.is_empty() {
        "generated-guard".to_string()
    } else {
        slug
    }
}

fn str_array(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// First balanced JSON object in a string (the model may wrap it in
/// prose). Reuses the same brace-counting discipline as the distiller.
fn first_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for (i, &c) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
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

/// Repair invalid JSON string escapes that models emit — chiefly a lone
/// backslash before a char that JSON doesn't recognize as an escape
/// (`\.`, `\(`, `\d` in a regex). Such a backslash is doubled so it
/// parses to a literal backslash, which is what the model meant.
///
/// Only touches backslashes INSIDE string literals; structural JSON is
/// left alone. A backslash that IS a valid escape (`\"`, `\\`, `\n`, …)
/// passes through untouched, as does a `\uXXXX` sequence.
fn repair_json_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut in_str = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if !in_str {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
            continue;
        }
        if c == '"' {
            in_str = false;
            out.push(c);
            continue;
        }
        if c == '\\' {
            match chars.peek() {
                // Valid JSON escapes — emit the pair verbatim.
                Some(&n) if matches!(n, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u') => {
                    out.push('\\');
                    out.push(n);
                    chars.next();
                }
                // Invalid escape (or trailing backslash): double it.
                _ => out.push_str("\\\\"),
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_a_regex_backslash_that_json_rejects() {
        let broken = r#"{"detect_regex":"\.canonicalize\(\)"}"#;
        // serde rejects the raw form...
        assert!(serde_json::from_str::<serde_json::Value>(broken).is_err());
        // ...and accepts the repaired form, preserving the regex.
        let fixed = repair_json_escapes(broken);
        let v: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["detect_regex"], r"\.canonicalize\(\)");
    }

    #[test]
    fn leaves_valid_escapes_and_structure_untouched() {
        let ok = r#"{"a":"line\nbreak","b":"quote\"here","c":"tab\tstop"}"#;
        let v: serde_json::Value = serde_json::from_str(&repair_json_escapes(ok)).unwrap();
        assert_eq!(v["a"], "line\nbreak");
        assert_eq!(v["b"], "quote\"here");
        assert_eq!(v["c"], "tab\tstop");
    }

    #[test]
    fn a_unicode_escape_survives() {
        let u = r#"{"x":"é"}"#;
        let v: serde_json::Value = serde_json::from_str(&repair_json_escapes(u)).unwrap();
        assert_eq!(v["x"], "é");
    }
}
