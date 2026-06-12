//! `clean` verb — remove orphaned project directories in bulk.
//!
//! Sub-module of `commands/project.rs`; see that file's header for
//! the layout rationale and the shared gate/formatting helpers.

use super::*;

pub fn clean(ctx: &AppContext, dry_run: bool, ignore_pending_journals: bool) -> Result<()> {
    if !dry_run {
        gate_on_pending_journals(ignore_pending_journals)?;
    }
    let config_dir = paths::claude_config_dir();
    let claude_json_path = paths::claude_json_path();
    let snaps = snapshots_dir();
    let locks = locks_dir();

    // Single flag drives the core: dry when explicitly requested OR
    // when the user hasn't confirmed. This makes the decision explicit
    // in one place (fixes the previous --json --yes path that returned
    // early on the dry preview and never deleted).
    let perform = !dry_run && ctx.yes;

    // Resolve user-managed protected paths (defaults + user deltas).
    // On read failure, fall back to the built-in DEFAULTS (audit fix:
    // an empty set would silently disable even `/`, `~`, `/Users` —
    // worse than blocking the clean. Built-in defaults are always
    // available since they're a compile-time constant).
    let protected =
        claudepot_core::protected_paths::resolved_set_or_defaults(&paths::claudepot_data_dir());

    let repair_root = paths::claudepot_repair_dir();
    let (result, orphans) = project::clean_orphans_with_progress(
        &config_dir,
        claude_json_path.as_deref(),
        Some(snaps.as_path()),
        Some(locks.as_path()),
        Some(repair_root.as_path()),
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
            if result.orphans_skipped_live == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    if result.claude_json_entries_removed > 0 {
        println!(
            "  \u{2713} Removed {} ~/.claude.json projects-map entr{}",
            result.claude_json_entries_removed,
            if result.claude_json_entries_removed == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }
    if result.history_lines_removed > 0 {
        println!(
            "  \u{2713} Removed {} history.jsonl line{}",
            result.history_lines_removed,
            if result.history_lines_removed == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    if result.claudepot_artifacts_removed > 0 {
        println!(
            "  \u{2713} Removed {} stale claudepot artifact{}",
            result.claudepot_artifacts_removed,
            if result.claudepot_artifacts_removed == 1 {
                ""
            } else {
                "s"
            }
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
