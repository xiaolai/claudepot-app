use crate::AppContext;
use anyhow::Result;
use claudepot_core::paths;
use claudepot_core::project::{self, format_size};
use claudepot_core::{project_journal, project_lock};
use std::time::{SystemTime, UNIX_EPOCH};

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
    let detail = project::show_project(&config_dir, path)?;

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

    let result = project::move_project(&args, &|_, _| {})?;

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
    let (result, orphans) = project::clean_orphans(&config_dir, dry_run || !ctx.yes)?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.orphans_found == 0 {
        println!("No orphaned projects found.");
        return Ok(());
    }

    let total_size: u64 = orphans.iter().map(|o| o.total_size_bytes).sum();

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
        println!(
            "  {:<50}  {} session{}  {:>9}  {}",
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
            last
        );
    }

    if dry_run || !ctx.yes {
        if !dry_run {
            // Need confirmation
            println!();
            println!(
                "Run with --yes to remove {} orphaned project{} ({}).",
                result.orphans_found,
                if result.orphans_found == 1 { "" } else { "s" },
                format_size(total_size)
            );
        }
    } else {
        // Actually remove (re-run without dry_run)
        let (real_result, _) = project::clean_orphans(&config_dir, false)?;
        println!();
        println!(
            "\u{2713} Removed {} project{}, freed {}.",
            real_result.orphans_removed,
            if real_result.orphans_removed == 1 {
                ""
            } else {
                "s"
            },
            format_size(real_result.bytes_freed)
        );
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
// project repair
// ---------------------------------------------------------------------------

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

    // --break-lock is independent of journal operations.
    if let Some(path_hint) = break_lock {
        return handle_break_lock(ctx, path_hint, &locks);
    }

    // --gc is independent of journal resume/rollback/abandon.
    if gc {
        return handle_gc(ctx, older_than.unwrap_or(90), &journals, &snaps);
    }

    let pending = project_journal::list_pending(&journals)?;
    if pending.is_empty() {
        if ctx.json {
            println!("[]");
        } else {
            println!("No pending rename journals.");
        }
        return Ok(());
    }

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if !resume && !rollback && !abandon {
        // List-only mode.
        return list_journals(ctx, &pending, &locks, now_unix);
    }

    // Filter by id if provided.
    let target_set: Vec<_> = pending
        .iter()
        .filter(|(path, _)| {
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_string())
            else {
                return false;
            };
            id.map(|want| stem == want).unwrap_or(all)
        })
        .cloned()
        .collect();

    if target_set.is_empty() {
        anyhow::bail!(
            "no journal matched. Pass --id <id> or --all. \
             Use `claudepot project repair` (no flags) to list."
        );
    }

    for (path, journal) in &target_set {
        if resume {
            handle_resume(ctx, path, journal)?;
        } else if rollback {
            handle_rollback(ctx, path, journal, &snaps)?;
        } else if abandon {
            handle_abandon(ctx, path)?;
        }
    }
    Ok(())
}

fn list_journals(
    ctx: &AppContext,
    pending: &[(std::path::PathBuf, project_journal::Journal)],
    locks_dir: &std::path::Path,
    now_unix: u64,
) -> Result<()> {
    let items: Vec<_> = pending
        .iter()
        .map(|(path, j)| {
            let lock_path = locks_dir.join(format!("{}.lock", j.old_san));
            let lock_live = match project_lock::read_lock(&lock_path) {
                Ok(l) => project_lock::is_live(&l),
                Err(_) => false,
            };
            let status = project_journal::classify(
                path,
                j,
                lock_live,
                now_unix,
                JOURNAL_NAG_THRESHOLD_SECS,
            );
            (path.clone(), j.clone(), status)
        })
        .collect();

    if ctx.json {
        let rendered: Vec<_> = items
            .iter()
            .map(|(p, j, s)| {
                serde_json::json!({
                    "id": p.file_stem().map(|s| s.to_string_lossy()),
                    "path": p,
                    "status": s.tag(),
                    "old_path": j.old_path,
                    "new_path": j.new_path,
                    "started_at": j.started_at,
                    "phases_completed": j.phases_completed,
                    "last_error": j.last_error,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rendered)?);
        return Ok(());
    }

    println!("Pending rename journals ({}):", items.len());
    println!();
    for (path, j, status) in items {
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        println!("  [{}] {}", status.tag(), id);
        println!("      {} \u{2192} {}", j.old_path, j.new_path);
        println!(
            "      started {}, phases [{}]",
            j.started_at,
            j.phases_completed.join(", ")
        );
        if let Some(err) = &j.last_error {
            println!("      last error: {}", err);
        }
    }
    println!();
    println!(
        "Resolve with: --resume, --rollback, or --abandon (add --id <id> or --all)."
    );
    Ok(())
}

fn handle_resume(
    ctx: &AppContext,
    path: &std::path::Path,
    journal: &project_journal::Journal,
) -> Result<()> {
    // Phases are idempotent (spec §6): re-running the original move
    // with the same flags finishes any uncompleted phases and, on
    // success, deletes the journal. The only wrinkle is the journal
    // itself: we must delete the pre-existing pending journal first
    // so the gate doesn't refuse the re-run.
    if !ctx.yes {
        eprintln!("repair --resume will re-run:");
        eprintln!(
            "  claudepot project move '{}' '{}' {}{}{}",
            journal.old_path,
            journal.new_path,
            if journal.flags.merge { "--merge " } else { "" },
            if journal.flags.overwrite { "--overwrite " } else { "" },
            if journal.flags.force { "--force" } else { "" },
        );
        eprintln!();
        eprintln!("Re-run with -y to confirm.");
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    // Mark the original journal as abandoned (superseded by repair)
    // instead of deleting it — preserves the audit trail and makes
    // the gate skip it on the re-run.
    let _ = project_journal::mark_abandoned(path);

    let config_dir = paths::claude_config_dir();
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    let snapshots_dir = Some(config_dir.join("claudepot").join("snapshots"));
    let args = claudepot_core::project::MoveArgs {
        old_path: journal.old_path.clone().into(),
        new_path: journal.new_path.clone().into(),
        config_dir,
        claude_json_path,
        snapshots_dir,
        no_move: journal.flags.no_move,
        merge: journal.flags.merge,
        overwrite: journal.flags.overwrite,
        force: journal.flags.force,
        dry_run: false,
        ignore_pending_journals: true, // original is now abandoned
    };
    let result = claudepot_core::project::move_project(&args, &|_, _| {})?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("\u{2713} Resumed and completed successfully.");
    }
    Ok(())
}

fn handle_rollback(
    ctx: &AppContext,
    path: &std::path::Path,
    journal: &project_journal::Journal,
    _snaps_dir: &std::path::Path,
) -> Result<()> {
    // Rollback runs a reverse move (new → old). Any snapshots taken
    // by destructive phases are reported for manual inspection —
    // automatic snapshot restoration is out of scope for v2 because
    // it crosses filesystem boundaries (restoring a removed CC dir
    // tree needs copy + delete + journal coordination of its own).
    if !ctx.yes {
        eprintln!("repair --rollback will re-run:");
        eprintln!(
            "  claudepot project move '{}' '{}' {}{}",
            journal.new_path,
            journal.old_path,
            if journal.flags.merge { "--merge " } else { "" },
            if journal.flags.overwrite { "--overwrite " } else { "" },
        );
        if !journal.snapshot_paths.is_empty() {
            eprintln!();
            eprintln!(
                "Snapshots from destructive phases (inspect before \
                 continuing if you want to preserve any):"
            );
            for s in &journal.snapshot_paths {
                eprintln!("  {:?}", s);
            }
        }
        eprintln!();
        eprintln!("Re-run with -y to confirm.");
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let _ = project_journal::mark_abandoned(path);

    let config_dir = paths::claude_config_dir();
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    let snapshots_dir = Some(config_dir.join("claudepot").join("snapshots"));
    let args = claudepot_core::project::MoveArgs {
        old_path: journal.new_path.clone().into(),
        new_path: journal.old_path.clone().into(),
        config_dir,
        claude_json_path,
        snapshots_dir,
        no_move: journal.flags.no_move,
        merge: journal.flags.merge,
        overwrite: journal.flags.overwrite,
        force: journal.flags.force,
        dry_run: false,
        ignore_pending_journals: true,
    };
    let result = claudepot_core::project::move_project(&args, &|_, _| {})?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("\u{2713} Rolled back successfully.");
        if !journal.snapshot_paths.is_empty() {
            println!();
            println!("Snapshots of destructive-phase targets remain at:");
            for s in &journal.snapshot_paths {
                println!("  {:?}", s);
            }
        }
    }
    Ok(())
}

fn handle_abandon(ctx: &AppContext, path: &std::path::Path) -> Result<()> {
    if !ctx.yes {
        eprintln!(
            "About to abandon journal {:?}. Future runs will no longer \
             nag about it. Re-run with -y to confirm.",
            path
        );
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let sidecar = project_journal::mark_abandoned(path)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "action": "abandoned",
                "journal": path,
                "sidecar": sidecar,
            })
        );
    } else {
        println!("\u{2713} Marked abandoned. Audit trail kept at {:?}.", path);
        println!("   Sidecar: {:?}", sidecar);
    }
    Ok(())
}

fn handle_break_lock(
    ctx: &AppContext,
    project_hint: &str,
    locks_dir: &std::path::Path,
) -> Result<()> {
    // `project_hint` may be a sanitized dir name or a project path.
    // Resolve to a lock filename.
    let san = claudepot_core::project::sanitize_path(project_hint);
    let lock_file = locks_dir.join(format!("{san}.lock"));
    if !lock_file.exists() {
        // Maybe the user passed the bare sanitized name directly.
        let alt = locks_dir.join(format!("{project_hint}.lock"));
        if alt.exists() {
            return do_break_lock(ctx, &alt);
        }
        anyhow::bail!("no lock file found for '{project_hint}'");
    }
    do_break_lock(ctx, &lock_file)
}

fn do_break_lock(ctx: &AppContext, lock_path: &std::path::Path) -> Result<()> {
    if !ctx.yes {
        eprintln!(
            "About to break lock {:?}. This may corrupt CC state if \
             another claudepot is actively renaming. Re-run with -y to confirm.",
            lock_path
        );
        anyhow::bail!("aborted (run with -y to confirm)");
    }
    let prior = project_lock::break_lock(lock_path)?;

    // Audit-log the manual break per spec §5.1.
    let journals = journals_dir();
    let _ = std::fs::create_dir_all(&journals);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let audit_path = journals.join(format!("broken-lock-{ts}.json"));
    let body = serde_json::json!({
        "broken_at": chrono::Utc::now().to_rfc3339(),
        "reason": "manual --break-lock",
        "prior_lock": prior,
        "broken_by_pid": std::process::id(),
        "lock_path": lock_path,
    });
    let _ = std::fs::write(
        &audit_path,
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string()),
    );

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "prior_lock": prior,
                "audit_path": audit_path,
            })
        );
    } else {
        println!("\u{2713} Broken lock {:?}", lock_path);
        println!(
            "   pid={}  host={}  started={}",
            prior.pid, prior.hostname, prior.start_iso8601
        );
        println!("   audit \u{2192} {:?}", audit_path);
    }
    Ok(())
}

fn handle_gc(
    ctx: &AppContext,
    older_than_days: u64,
    journals_dir: &std::path::Path,
    snapshots_dir: &std::path::Path,
) -> Result<()> {
    let cutoff_secs = older_than_days.saturating_mul(86400);
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut removed_journals = 0usize;
    let mut removed_snaps = 0usize;
    let mut freed_bytes: u64 = 0;

    // Abandoned journals (only those with the .abandoned.json sidecar).
    if journals_dir.exists() {
        for entry in std::fs::read_dir(journals_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".abandoned.json") {
                continue;
            }
            let base = name.trim_end_matches(".abandoned.json");
            let journal_path = journals_dir.join(format!("{base}.json"));
            let meta = entry.metadata()?;
            let age = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| now_unix.saturating_sub(d.as_secs()))
                .unwrap_or(0);
            if age >= cutoff_secs {
                if ctx.yes {
                    freed_bytes += meta.len();
                    std::fs::remove_file(entry.path()).ok();
                    if journal_path.exists() {
                        freed_bytes += std::fs::metadata(&journal_path)
                            .map(|m| m.len())
                            .unwrap_or(0);
                        std::fs::remove_file(&journal_path).ok();
                    }
                    removed_journals += 1;
                } else {
                    println!(
                        "would gc journal {:?} (age {}d)",
                        journal_path,
                        age / 86400
                    );
                }
            }
        }
    }

    // Snapshots older than cutoff.
    if snapshots_dir.exists() {
        for entry in std::fs::read_dir(snapshots_dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            let age = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| now_unix.saturating_sub(d.as_secs()))
                .unwrap_or(0);
            if age >= cutoff_secs {
                if ctx.yes {
                    freed_bytes += meta.len();
                    std::fs::remove_file(entry.path()).ok();
                    removed_snaps += 1;
                } else {
                    println!(
                        "would gc snapshot {:?} (age {}d)",
                        entry.path(),
                        age / 86400
                    );
                }
            }
        }
    }

    if !ctx.yes {
        println!();
        println!("Dry run. Re-run with -y to perform cleanup.");
    } else if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "removed_journals": removed_journals,
                "removed_snapshots": removed_snaps,
                "bytes_freed": freed_bytes,
            })
        );
    } else {
        println!(
            "\u{2713} Removed {} journal(s), {} snapshot(s), freed {}.",
            removed_journals,
            removed_snaps,
            format_size(freed_bytes)
        );
    }

    Ok(())
}
