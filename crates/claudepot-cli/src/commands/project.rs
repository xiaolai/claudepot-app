use anyhow::Result;
use crate::AppContext;
use claudepot_core::paths;
use claudepot_core::project::{self, format_size};
use std::time::SystemTime;

pub fn list(ctx: &AppContext) -> Result<()> {
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
        "  {:<50}  {:>8}  {:>6}  {:>9}  {:>10}  {}",
        "Path", "Sessions", "Memory", "Size", "Last used", "Status"
    );
    println!(
        "  {:<50}  {:>8}  {:>6}  {:>9}  {:>10}  {}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
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
            .map(|t| format_relative_time(t))
            .unwrap_or_else(|| "unknown".to_string());

        // Truncate path for display
        let display_path = if p.original_path.len() > 50 {
            format!("...{}", &p.original_path[p.original_path.len() - 47..])
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
        if detail.info.memory_file_count == 1 { "" } else { "s" },
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
            .map(|t| format_absolute_time(t))
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
                .map(|t| format_absolute_time(t))
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

pub fn move_project(
    ctx: &AppContext,
    old_path: &str,
    new_path: &str,
    no_move: bool,
    merge: bool,
    overwrite: bool,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let config_dir = paths::claude_config_dir();

    let args = project::MoveArgs {
        old_path: old_path.into(),
        new_path: new_path.into(),
        config_dir,
        no_move,
        merge,
        overwrite,
        force,
        dry_run,
    };

    let result = project::move_project(&args)?;

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
        let old_san = project::sanitize_path(
            &std::fs::canonicalize(new_path)
                .unwrap_or_else(|_| new_path.into())
                .to_string_lossy()
                .replace(new_path, old_path),
        );
        let new_san = project::sanitize_path(
            &std::fs::canonicalize(new_path)
                .unwrap_or_else(|_| new_path.into())
                .to_string_lossy(),
        );
        println!("  \u{2713} Renamed CC project data");
        println!("    {} \u{2192} {}", old_san, new_san);
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

pub fn clean(ctx: &AppContext, dry_run: bool) -> Result<()> {
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
            .map(|t| format_relative_time(t))
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "  {:<50}  {} session{}  {:>9}  {}",
            if o.original_path.len() > 50 {
                format!("...{}", &o.original_path[o.original_path.len() - 47..])
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
            if real_result.orphans_removed == 1 { "" } else { "s" },
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
