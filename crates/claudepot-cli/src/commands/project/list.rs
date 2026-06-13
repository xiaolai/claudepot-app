//! `list` / `show` verb group — read-only project browsing. Grouped
//! per the commands.md verb-group guidance: both render the same
//! project metadata (sessions / memory / size / last-used) and share
//! the pending-journal banner.
//!
//! Sub-module of `commands/project.rs`; see that file's header for
//! the layout rationale and the shared gate/formatting helpers.

use super::*;

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

fn format_absolute_time(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}
