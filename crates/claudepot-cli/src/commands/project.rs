use crate::AppContext;
use anyhow::Result;
use claudepot_core::paths;
use claudepot_core::project::{self, format_size};
use claudepot_core::project_journal;
use std::time::SystemTime;

/// Default journal nag threshold per spec §8 Q7.
const JOURNAL_NAG_THRESHOLD_SECS: u64 = 86_400; // 24h

fn journals_dir() -> std::path::PathBuf {
    paths::claude_config_dir()
        .join("claudepot")
        .join("journals")
}

fn locks_dir() -> std::path::PathBuf {
    paths::claude_config_dir().join("claudepot").join("locks")
}

fn snapshots_dir() -> std::path::PathBuf {
    paths::claude_config_dir()
        .join("claudepot")
        .join("snapshots")
}

/// Check whether a journal has been explicitly abandoned via a
/// sidecar `.abandoned.json` file.
fn abandoned_sidecar_exists(journal_path: &std::path::Path) -> bool {
    let Some((parent, stem)) = journal_path
        .parent()
        .zip(journal_path.file_stem().map(|s| s.to_string_lossy().to_string()))
    else {
        return false;
    };
    parent.join(format!("{stem}.abandoned.json")).exists()
}

/// Print a one-line banner if any pending journals exist. Used by
/// read-only subcommands (list, show) per spec §6 (gate rules).
fn warn_pending_journals_banner() {
    let Ok(pending) = project_journal::list_pending(&journals_dir()) else {
        return;
    };
    let non_abandoned: Vec<_> = pending
        .into_iter()
        .filter(|(path, _)| !abandoned_sidecar_exists(path))
        .collect();
    if !non_abandoned.is_empty() {
        eprintln!(
            "\u{26a0}  {} pending rename journal(s). Run `claudepot project repair` to resolve.",
            non_abandoned.len()
        );
    }
}

/// Hard gate for mutating subcommands. Returns an error if any journal
/// is pending (and not abandoned) unless the caller explicitly opts
/// out via `ignore_pending_journals`.
fn gate_on_pending_journals(ignore: bool) -> Result<()> {
    let pending: Vec<_> = project_journal::list_pending(&journals_dir())?
        .into_iter()
        .filter(|(path, _)| !abandoned_sidecar_exists(path))
        .collect();
    if pending.is_empty() || ignore {
        if ignore && !pending.is_empty() {
            eprintln!(
                "\u{26a0}  Ignoring {} pending rename journal(s) at your request.",
                pending.len()
            );
        }
        return Ok(());
    }
    eprintln!("Pending rename journals detected:");
    for (path, j) in &pending {
        eprintln!(
            "  {} — {} \u{2192} {} (started {}, phases: [{}])",
            path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            j.old_path,
            j.new_path,
            j.started_at,
            j.phases_completed.join(", ")
        );
    }
    anyhow::bail!(
        "refusing to proceed with pending rename journals. \
         Run `claudepot project repair` or pass --ignore-pending-journals."
    );
}

pub fn list(ctx: &AppContext) -> Result<()> {
    warn_pending_journals_banner();
    let config_dir = paths::claude_config_dir();
    let projects = project::list_projects(&config_dir)?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&projects)?);
        return Ok(());
    }

    if projects.is_empty() {
        println!("No CC project directories found.");
        return Ok(());
    }

    let mut orphan_count = 0;
    let mut total_size: u64 = 0;
    let mut orphan_size: u64 = 0;

    // Header
    println!(
        "  {:<50}  {:>8}  {:>6}  {:>9}  {:>10}  Status",
        "Path", "Sessions", "Memory", "Size", "Last used"
    );
    println!(
        "  {:<50}  {:>8}  {:>6}  {:>9}  {:>10}  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );

    for p in &projects {
        total_size += p.total_size_bytes;
        if p.is_orphan {
            orphan_count += 1;
            orphan_size += p.total_size_bytes;
        }

        let status = if p.is_orphan {
            "\u{26a0} orphan"
        } else {
            "\u{2713}"
        };
        let last_used = p
            .last_modified
            .map(format_relative_time)
            .unwrap_or_else(|| "unknown".to_string());

        // Truncate path for display (char-safe to avoid panic on multibyte)
        let display_path = if p.original_path.chars().count() > 50 {
            let tail: String = p
                .original_path
                .chars()
                .rev()
                .take(47)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("...{}", tail)
        } else {
            p.original_path.clone()
        };

        println!(
            "  {:<50}  {:>8}  {:>6}  {:>9}  {:>10}  {}",
            display_path,
            p.session_count,
            p.memory_file_count,
            format_size(p.total_size_bytes),
            last_used,
            status
        );
    }

    println!();
    if orphan_count > 0 {
        println!(
            "{} projects, {} total ({} orphans, {} reclaimable)",
            projects.len(),
            format_size(total_size),
            orphan_count,
            format_size(orphan_size)
        );
    } else {
        println!(
            "{} projects, {} total",
            projects.len(),
            format_size(total_size)
        );
    }

    Ok(())
}

pub fn show(ctx: &AppContext, path: &str) -> Result<()> {
    warn_pending_journals_banner();
    let config_dir = paths::claude_config_dir();
    let detail = match project::show_project(&config_dir, path) {
        Ok(d) => d,
        Err(claudepot_core::error::ProjectError::NotFound(p)) => {
            // Hint: scan known projects for a basename match — common
            // case after a rename where the user is still typing the
            // old path.
            let basename = std::path::Path::new(&p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let suggestions: Vec<String> = if basename.is_empty() {
                Vec::new()
            } else {
                project::list_projects(&config_dir)
                    .ok()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|info| {
                        info.original_path != p
                            && std::path::Path::new(&info.original_path)
                                .file_name()
                                .map(|s| {
                                    let s = s.to_string_lossy();
                                    s.starts_with(&basename) || basename.starts_with(s.as_ref())
                                })
                                .unwrap_or(false)
                    })
                    .map(|info| info.original_path)
                    .take(5)
                    .collect()
            };
            eprintln!("project not found: {p}");
            if !suggestions.is_empty() {
                eprintln!();
                eprintln!("Did you mean one of these (basename match)?");
                for s in &suggestions {
                    eprintln!("  {s}");
                }
            }
            anyhow::bail!("not found");
        }
        Err(e) => return Err(e.into()),
    };

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&detail)?);
        return Ok(());
    }

    println!("Project: {}", detail.info.original_path);
    println!(
        "  CC dir:    ~/.claude/projects/{}",
        detail.info.sanitized_name
    );
    println!("  Sessions:  {}", detail.info.session_count);
    println!(
        "  Memory:    {} file{}{}",
        detail.info.memory_file_count,
        if detail.info.memory_file_count == 1 {
            ""
        } else {
            "s"
        },
        if detail.memory_files.is_empty() {
            String::new()
        } else {
            format!(" ({})", detail.memory_files.join(", "))
        }
    );
    println!("  Size:      {}", format_size(detail.info.total_size_bytes));
    println!(
        "  Last used: {}",
        detail
            .info
            .last_modified
            .map(format_absolute_time)
            .unwrap_or_else(|| "unknown".to_string())
    );

    if detail.info.is_orphan {
        println!("  Status:    \u{26a0} orphan (source path does not exist)");
    }

    if !detail.sessions.is_empty() {
        println!();
        println!("  Sessions:");
        for s in &detail.sessions {
            let last = s
                .last_modified
                .map(format_absolute_time)
                .unwrap_or_else(|| "unknown".to_string());

            // Truncate session ID for display
            let display_id = if s.session_id.len() > 12 {
                format!("{}...", &s.session_id[..12])
            } else {
                s.session_id.clone()
            };

            println!(
                "    {}  {:>9}  {}",
                display_id,
                format_size(s.file_size),
                last
            );
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn move_project(
    ctx: &AppContext,
    old_path: &str,
    new_path: &str,
    no_move: bool,
    merge: bool,
    overwrite: bool,
    force: bool,
    dry_run: bool,
    ignore_pending_journals: bool,
) -> Result<()> {
    if !dry_run {
        gate_on_pending_journals(ignore_pending_journals)?;
    }
    let config_dir = paths::claude_config_dir();
    // `~/.claude.json` is the sibling config file to `~/.claude/`.
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    // Default snapshot location per spec §6.
    let snapshots_dir = Some(config_dir.join("claudepot").join("snapshots"));

    let args = project::MoveArgs {
        old_path: old_path.into(),
        new_path: new_path.into(),
        config_dir,
        claude_json_path,
        snapshots_dir,
        no_move,
        merge,
        overwrite,
        force,
        dry_run,
        ignore_pending_journals,
    };

    let result = project::move_project(&args, &claudepot_core::project_progress::NoopSink)?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if dry_run {
        // Dry run plan is in warnings[0]
        if let Some(plan) = result.warnings.first() {
            println!("{plan}");
        }
        return Ok(());
    }

    println!("Moving project: {} \u{2192} {}", old_path, new_path);
    println!();

    if result.actual_dir_moved {
        println!("  \u{2713} Moved directory on disk");
    }
    if result.cc_dir_renamed {
        println!("  \u{2713} Renamed CC project data");
        if let (Some(old_san), Some(new_san)) = (&result.old_sanitized, &result.new_sanitized) {
            println!("    {} \u{2192} {}", old_san, new_san);
        }
    }
    if result.history_lines_updated > 0 {
        println!(
            "  \u{2713} Updated {} history entries",
            result.history_lines_updated
        );
    }
    if result.jsonl_files_modified > 0 {
        println!(
            "  \u{2713} Rewrote {} cwd reference{} across {} session/subagent file{} (P6)",
            result.jsonl_lines_rewritten,
            if result.jsonl_lines_rewritten == 1 { "" } else { "s" },
            result.jsonl_files_modified,
            if result.jsonl_files_modified == 1 { "" } else { "s" },
        );
    }
    if !result.jsonl_errors.is_empty() {
        println!(
            "  \u{26a0} {} P6 file(s) failed:",
            result.jsonl_errors.len()
        );
        for (path, err) in &result.jsonl_errors {
            println!("    {:?} \u{2014} {}", path, err);
        }
    }
    if result.config_key_renamed {
        let suffix = if result.config_had_collision {
            format!(
                " (collision: {} key(s) merged old-wins; snapshot at {:?})",
                result.config_merged_keys.len(),
                result.config_snapshot_path.as_ref()
            )
        } else if result.config_nested_rewrites > 0 {
            format!(
                " ({} nested path string(s) rewritten)",
                result.config_nested_rewrites
            )
        } else {
            String::new()
        };
        println!("  \u{2713} ~/.claude.json projects map key migrated (P7){suffix}");
    }
    if result.memory_dir_moved {
        println!("  \u{2713} Auto-memory dir moved (P8)");
    } else if result.memory_git_root_changed {
        println!("  \u{2014} P8 skipped: git root changed but no auto-memory dir to move");
    }
    if result.project_settings_rewritten {
        println!("  \u{2713} Project-local .claude/settings.json autoMemoryDirectory rewritten (P9)");
    }

    for warning in &result.warnings {
        println!("  \u{26a0} {}", warning);
    }

    println!();
    println!("Done.");
    Ok(())
}

pub fn clean(ctx: &AppContext, dry_run: bool, ignore_pending_journals: bool) -> Result<()> {
    if !dry_run {
        gate_on_pending_journals(ignore_pending_journals)?;
    }
    let config_dir = paths::claude_config_dir();
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    let snaps = snapshots_dir();
    let locks = locks_dir();

    // Single flag drives the core: dry when explicitly requested OR
    // when the user hasn't confirmed. This makes the decision explicit
    // in one place (fixes the previous --json --yes path that returned
    // early on the dry preview and never deleted).
    let perform = !dry_run && ctx.yes;

    // Resolve user-managed protected paths (defaults + user deltas).
    // A read failure shouldn't block the clean — log and proceed
    // unprotected so a corrupt prefs file doesn't pin the user out
    // of cleanup. The protection is a safety net, not a gate.
    let protected = match claudepot_core::protected_paths::resolved_set(
        &paths::claudepot_data_dir(),
    ) {
        Ok(set) => set,
        Err(e) => {
            eprintln!("warning: protected-paths read failed: {e}");
            std::collections::HashSet::new()
        }
    };

    let (result, orphans) = project::clean_orphans_with_progress(
        &config_dir,
        claude_json_path.as_deref(),
        Some(snaps.as_path()),
        Some(locks.as_path()),
        &protected,
        !perform,
        &claudepot_core::project_progress::NoopSink,
    )?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.orphans_found == 0 && result.unreachable_skipped == 0 {
        println!("No orphaned projects found.");
        return Ok(());
    }

    let total_size: u64 = orphans.iter().map(|o| o.total_size_bytes).sum();

    if result.orphans_found > 0 {
        println!(
            "Found {} orphaned project{} ({}):",
            result.orphans_found,
            if result.orphans_found == 1 { "" } else { "s" },
            format_size(total_size)
        );
        println!();

        for o in &orphans {
            let last = o
                .last_modified
                .map(format_relative_time)
                .unwrap_or_else(|| "unknown".to_string());
            let tag = if o.is_empty { " (empty)" } else { "" };
            println!(
                "  {:<50}  {} session{}  {:>9}  {}{}",
                if o.original_path.chars().count() > 50 {
                    let tail: String = o
                        .original_path
                        .chars()
                        .rev()
                        .take(47)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("...{}", tail)
                } else {
                    o.original_path.clone()
                },
                o.session_count,
                if o.session_count == 1 { " " } else { "s" },
                format_size(o.total_size_bytes),
                last,
                tag
            );
        }
    }

    if result.unreachable_skipped > 0 {
        println!();
        println!(
            "\u{26a0}  Skipped {} project{} with unreachable source paths (unmounted volume or permission denied). Mount the source and re-run.",
            result.unreachable_skipped,
            if result.unreachable_skipped == 1 { "" } else { "s" }
        );
    }

    if !perform {
        if !dry_run && result.orphans_found > 0 {
            println!();
            println!(
                "Run with --yes to remove {} orphaned project{} ({}).",
                result.orphans_found,
                if result.orphans_found == 1 { "" } else { "s" },
                format_size(total_size)
            );
        }
        return Ok(());
    }

    // Summary of the actual deletion run.
    println!();
    println!(
        "\u{2713} Removed {} project{}, freed {}.",
        result.orphans_removed,
        if result.orphans_removed == 1 { "" } else { "s" },
        format_size(result.bytes_freed)
    );
    if result.orphans_skipped_live > 0 {
        println!(
            "\u{26a0}  Skipped {} project{} with a live CC session — quit Claude Code and re-run.",
            result.orphans_skipped_live,
            if result.orphans_skipped_live == 1 { "" } else { "s" }
        );
    }
    if result.claude_json_entries_removed > 0 {
        println!(
            "  \u{2713} Removed {} ~/.claude.json projects-map entr{}",
            result.claude_json_entries_removed,
            if result.claude_json_entries_removed == 1 { "y" } else { "ies" }
        );
    }
    if result.history_lines_removed > 0 {
        println!(
            "  \u{2713} Removed {} history.jsonl line{}",
            result.history_lines_removed,
            if result.history_lines_removed == 1 { "" } else { "s" }
        );
    }
    if result.claudepot_artifacts_removed > 0 {
        println!(
            "  \u{2713} Removed {} stale claudepot artifact{}",
            result.claudepot_artifacts_removed,
            if result.claudepot_artifacts_removed == 1 { "" } else { "s" }
        );
    }
    if !result.snapshot_paths.is_empty() {
        println!();
        println!("Recovery snapshots (kept for manual restore):");
        for p in &result.snapshot_paths {
            println!("  {:?}", p);
        }
    }

    Ok(())
}

fn format_relative_time(time: SystemTime) -> String {
    let elapsed = time.elapsed().unwrap_or_default();
    let secs = elapsed.as_secs();

    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn format_absolute_time(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

// ---------------------------------------------------------------------------
// project repair — thin dispatch over claudepot_core::project_repair
// ---------------------------------------------------------------------------

use claudepot_core::project_repair;

#[allow(clippy::too_many_arguments)]
pub fn repair(
    ctx: &AppContext,
    resume: bool,
    rollback: bool,
    abandon: bool,
    break_lock: Option<&str>,
    gc: bool,
    older_than: Option<u64>,
    id: Option<&str>,
    all: bool,
) -> Result<()> {
    let journals = journals_dir();
    let locks = locks_dir();
    let snaps = snapshots_dir();

    if let Some(path_hint) = break_lock {
        return handle_break_lock(ctx, path_hint, &locks, &journals);
    }

    if gc {
        return handle_gc(ctx, older_than.unwrap_or(90), &journals, &snaps);
    }

    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )?;
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
        .filter(|e| id.map(|want| e.id == want).unwrap_or(all))
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
        println!("      {} \u{2192} {}", e.journal.old_path, e.journal.new_path);
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
    println!(
        "Resolve with: --resume, --rollback, or --abandon (add --id <id> or --all)."
    );
    Ok(())
}

/// Core-facing helper: build the three path args needed by resume/rollback.
fn repair_paths() -> (std::path::PathBuf, Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let config_dir = paths::claude_config_dir();
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    let snapshots = Some(config_dir.join("claudepot").join("snapshots"));
    (config_dir, claude_json_path, snapshots)
}

fn handle_resume(ctx: &AppContext, entry: &project_repair::JournalEntry) -> Result<()> {
    if !ctx.yes {
        eprintln!("repair --resume will re-run:");
        eprintln!(
            "  claudepot project move '{}' '{}' {}{}{}",
            entry.journal.old_path,
            entry.journal.new_path,
            if entry.journal.flags.merge { "--merge " } else { "" },
            if entry.journal.flags.overwrite { "--overwrite " } else { "" },
            if entry.journal.flags.force { "--force" } else { "" },
        );
        eprintln!();
        eprintln!("Re-run with -y to confirm.");
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let (config_dir, claude_json, snapshots) = repair_paths();
    let result = project_repair::resume(entry, config_dir, claude_json, snapshots, &claudepot_core::project_progress::NoopSink)?;
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
            if entry.journal.flags.merge { "--merge " } else { "" },
            if entry.journal.flags.overwrite { "--overwrite " } else { "" },
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
    let (config_dir, claude_json, snapshots) = repair_paths();
    let result = project_repair::rollback(entry, config_dir, claude_json, snapshots, &claudepot_core::project_progress::NoopSink)?;
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
        println!("\u{2713} Marked abandoned. Audit trail kept at {:?}.", entry.path);
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
