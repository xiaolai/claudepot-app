//! `repair` verb — thin dispatch over `claudepot_core::project_repair`
//! for pending / failed rename journals (resume, rollback, abandon,
//! break-lock, gc).
//!
//! Sub-module of `commands/project.rs`; see that file's header for
//! the layout rationale and the shared gate helpers.

use super::*;

use claudepot_core::project_repair;

/// Default journal nag threshold per spec §8 Q7.
const JOURNAL_NAG_THRESHOLD_SECS: u64 = 86_400; // 24h

/// Flag bundle for `claudepot project repair`, flattened into the
/// `ProjectAction::Repair` variant in `main.rs` (the in-tree
/// `ExportArgs` pattern). Field docs are the user-visible `--help`
/// text.
#[derive(Debug, clap::Args)]
pub struct RepairArgs {
    /// Finish remaining phases for a journal (id optional, use --all to target every one)
    #[arg(long)]
    pub resume: bool,
    /// Reverse completed phases and restore snapshots
    #[arg(long, conflicts_with = "resume")]
    pub rollback: bool,
    /// Mark a journal as abandoned (keeps audit trail, suppresses nags)
    #[arg(long, conflicts_with_all = ["resume", "rollback"])]
    pub abandon: bool,
    /// Force-release a lock file whose staleness detection refuses auto-break
    #[arg(long)]
    pub break_lock: Option<String>,
    /// Clean up abandoned journals and expired snapshots
    #[arg(long)]
    pub gc: bool,
    /// For --gc: how many days old before cleanup (default 90)
    #[arg(long, requires = "gc")]
    pub older_than: Option<u64>,
    /// Target journal id (filename without extension). If absent,
    /// --resume/--rollback/--abandon require --all.
    #[arg(long)]
    pub id: Option<String>,
    /// Apply to all matching journals
    #[arg(long)]
    pub all: bool,
}

pub fn repair(ctx: &AppContext, args: RepairArgs) -> Result<()> {
    let RepairArgs {
        resume,
        rollback,
        abandon,
        break_lock,
        gc,
        older_than,
        id,
        all,
    } = args;
    let journals = journals_dir();
    let locks = locks_dir();
    let snaps = snapshots_dir();

    if let Some(path_hint) = break_lock.as_deref() {
        return handle_break_lock(ctx, path_hint, &locks, &journals);
    }

    if gc {
        return handle_gc(ctx, older_than.unwrap_or(90), &journals, &snaps);
    }

    let entries =
        project_repair::list_pending_with_status(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)?;
    if entries.is_empty() {
        if ctx.json {
            println!("[]");
        } else {
            println!("No pending rename journals.");
        }
        return Ok(());
    }

    if !resume && !rollback && !abandon {
        return list_journals(ctx, &entries);
    }

    let target_set: Vec<_> = entries
        .iter()
        .filter(|e| id.as_deref().map(|want| e.id == want).unwrap_or(all))
        .cloned()
        .collect();

    if target_set.is_empty() {
        anyhow::bail!(
            "no journal matched. Pass --id <id> or --all. \
             Use `claudepot project repair` (no flags) to list."
        );
    }

    for entry in &target_set {
        if resume {
            handle_resume(ctx, entry)?;
        } else if rollback {
            handle_rollback(ctx, entry)?;
        } else if abandon {
            handle_abandon(ctx, entry)?;
        }
    }
    Ok(())
}

fn list_journals(ctx: &AppContext, entries: &[project_repair::JournalEntry]) -> Result<()> {
    if ctx.json {
        let rendered: Vec<_> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "path": e.path,
                    "status": e.status.tag(),
                    "old_path": e.journal.old_path,
                    "new_path": e.journal.new_path,
                    "started_at": e.journal.started_at,
                    "phases_completed": e.journal.phases_completed,
                    "last_error": e.journal.last_error,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rendered)?);
        return Ok(());
    }

    println!("Pending rename journals ({}):", entries.len());
    println!();
    for e in entries {
        println!("  [{}] {}", e.status.tag(), e.id);
        println!(
            "      {} \u{2192} {}",
            e.journal.old_path, e.journal.new_path
        );
        println!(
            "      started {}, phases [{}]",
            e.journal.started_at,
            e.journal.phases_completed.join(", ")
        );
        if let Some(err) = &e.journal.last_error {
            println!("      last error: {}", err);
        }
    }
    println!();
    println!("Resolve with: --resume, --rollback, or --abandon (add --id <id> or --all).");
    Ok(())
}

/// Core-facing helper: build the path args needed by resume/rollback.
/// Returns (config_dir, claude_json, snapshots_dir, claudepot_state_dir).
/// `claudepot_state_dir` MUST flow through so the resumed/rolled-back
/// move's lock + journal + snapshot tree stays attached to the
/// original repair tree (audit B3 fix).
fn repair_paths() -> (
    std::path::PathBuf,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
) {
    let config_dir = paths::claude_config_dir();
    let claude_json_path = paths::claude_json_path();
    let state_root = paths::claudepot_repair_dir();
    let snapshots = Some(state_root.join("snapshots"));
    (config_dir, claude_json_path, snapshots, Some(state_root))
}

fn handle_resume(ctx: &AppContext, entry: &project_repair::JournalEntry) -> Result<()> {
    if !ctx.yes {
        eprintln!("repair --resume will re-run:");
        eprintln!(
            "  claudepot project move '{}' '{}' {}{}{}",
            entry.journal.old_path,
            entry.journal.new_path,
            if entry.journal.flags.merge {
                "--merge "
            } else {
                ""
            },
            if entry.journal.flags.overwrite {
                "--overwrite "
            } else {
                ""
            },
            if entry.journal.flags.force {
                "--force"
            } else {
                ""
            },
        );
        eprintln!();
        eprintln!("Re-run with -y to confirm.");
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let (config_dir, claude_json, snapshots, state_root) = repair_paths();
    let result = project_repair::resume(
        entry,
        config_dir,
        claude_json,
        snapshots,
        state_root,
        &claudepot_core::project_progress::NoopSink,
    )?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("\u{2713} Resumed and completed successfully.");
    }
    Ok(())
}

fn handle_rollback(ctx: &AppContext, entry: &project_repair::JournalEntry) -> Result<()> {
    if !ctx.yes {
        eprintln!("repair --rollback will re-run:");
        eprintln!(
            "  claudepot project move '{}' '{}' {}{}",
            entry.journal.new_path,
            entry.journal.old_path,
            if entry.journal.flags.merge {
                "--merge "
            } else {
                ""
            },
            if entry.journal.flags.overwrite {
                "--overwrite "
            } else {
                ""
            },
        );
        if !entry.journal.snapshot_paths.is_empty() {
            eprintln!();
            eprintln!(
                "Snapshots from destructive phases (inspect before \
                 continuing if you want to preserve any):"
            );
            for s in &entry.journal.snapshot_paths {
                eprintln!("  {:?}", s);
            }
        }
        eprintln!();
        eprintln!("Re-run with -y to confirm.");
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let (config_dir, claude_json, snapshots, state_root) = repair_paths();
    let result = project_repair::rollback(
        entry,
        config_dir,
        claude_json,
        snapshots,
        state_root,
        &claudepot_core::project_progress::NoopSink,
    )?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("\u{2713} Rolled back successfully.");
        if !entry.journal.snapshot_paths.is_empty() {
            println!();
            println!("Snapshots of destructive-phase targets remain at:");
            for s in &entry.journal.snapshot_paths {
                println!("  {:?}", s);
            }
        }
    }
    Ok(())
}

fn handle_abandon(ctx: &AppContext, entry: &project_repair::JournalEntry) -> Result<()> {
    if !ctx.yes {
        eprintln!(
            "About to abandon journal {:?}. Future runs will no longer \
             nag about it. Re-run with -y to confirm.",
            entry.path
        );
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let sidecar = project_repair::abandon(entry)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "action": "abandoned",
                "journal": entry.path,
                "sidecar": sidecar,
            })
        );
    } else {
        println!(
            "\u{2713} Marked abandoned. Audit trail kept at {:?}.",
            entry.path
        );
        println!("   Sidecar: {:?}", sidecar);
    }
    Ok(())
}

fn handle_break_lock(
    ctx: &AppContext,
    project_hint: &str,
    locks_dir: &std::path::Path,
    journals_dir: &std::path::Path,
) -> Result<()> {
    let lock_path = project_repair::resolve_lock_file(locks_dir, project_hint)
        .ok_or_else(|| anyhow::anyhow!("no lock file found for '{project_hint}'"))?;

    if !ctx.yes {
        eprintln!(
            "About to break lock {:?}. This may corrupt CC state if \
             another claudepot is actively renaming. Re-run with -y to confirm.",
            lock_path
        );
        anyhow::bail!("aborted (run with -y to confirm)");
    }

    let broken = project_repair::break_lock_with_audit(&lock_path, journals_dir)?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "prior_lock": broken.prior,
                "audit_path": broken.audit_path,
            })
        );
    } else {
        println!("\u{2713} Broken lock {:?}", lock_path);
        println!(
            "   pid={}  host={}  started={}",
            broken.prior.pid, broken.prior.hostname, broken.prior.start_iso8601
        );
        println!("   audit \u{2192} {:?}", broken.audit_path);
    }
    Ok(())
}

fn handle_gc(
    ctx: &AppContext,
    older_than_days: u64,
    journals_dir: &std::path::Path,
    snapshots_dir: &std::path::Path,
) -> Result<()> {
    let dry_run = !ctx.yes;
    let result = project_repair::gc(journals_dir, snapshots_dir, older_than_days, dry_run)?;

    // Audit M3: `--json` must produce structured output on EVERY path
    // including dry-run. Previously the dry-run branch printed plain
    // text regardless of the flag, so `gc --json` (without -y) returned
    // unparseable output — breaking scripted callers that use the
    // dry-run mode as a "what would happen" preview.
    if ctx.json {
        let payload = if dry_run {
            serde_json::json!({
                "dry_run": true,
                "would_remove": result
                    .would_remove
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
            })
        } else {
            serde_json::json!({
                "dry_run": false,
                "removed_journals": result.removed_journals,
                "removed_snapshots": result.removed_snapshots,
                "bytes_freed": result.bytes_freed,
            })
        };
        println!("{payload}");
        return Ok(());
    }

    if dry_run {
        for p in &result.would_remove {
            println!("would gc {:?}", p);
        }
        println!();
        println!("Dry run. Re-run with -y to perform cleanup.");
    } else {
        println!(
            "\u{2713} Removed {} journal(s), {} snapshot(s), freed {}.",
            result.removed_journals,
            result.removed_snapshots,
            format_size(result.bytes_freed)
        );
    }

    Ok(())
}
