//! `trash` verb group — list / restore / empty the recoverable
//! project trash. Grouped per the commands.md verb-group guidance:
//! all three operate on the same trash store.
//!
//! Sub-module of `commands/project.rs`; see that file's header for
//! the layout rationale and the shared formatting helpers.

use super::*;

use claudepot_core::project_trash::{self, ProjectTrashFilter};

pub fn trash_list(ctx: &AppContext) -> Result<()> {
    let data_dir = paths::claudepot_data_dir();
    let listing = project_trash::list(&data_dir, ProjectTrashFilter::default())
        .context("read project trash")?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&listing)?);
        return Ok(());
    }

    if listing.entries.is_empty() {
        println!("No trashed projects.");
        return Ok(());
    }

    println!(
        "{} trashed project{} ({}):",
        listing.entries.len(),
        if listing.entries.len() == 1 { "" } else { "s" },
        format_size(listing.total_bytes)
    );
    println!();
    for e in &listing.entries {
        let when = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(e.ts_ms)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let path_label = e.original_path.as_deref().unwrap_or("(unknown cwd)");
        println!(
            "  {}  {}  {}  {} session{}  {}",
            e.id,
            when,
            format_size(e.bytes),
            e.session_count,
            if e.session_count == 1 { "" } else { "s" },
            path_label
        );
    }
    Ok(())
}

pub fn trash_restore(ctx: &AppContext, entry_id: &str) -> Result<()> {
    let data_dir = paths::claudepot_data_dir();
    let config_dir = paths::claude_config_dir();
    let claude_json = paths::claude_json_path();
    let history = config_dir.join("history.jsonl");

    let report = project_trash::restore(
        &data_dir,
        entry_id,
        &config_dir,
        claude_json.as_deref(),
        Some(&history),
    )
    .context("restore project from trash")?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("\u{2713} Restored {}", report.restored_dir.display());
    if report.claude_json_restored {
        println!("  · re-inserted ~/.claude.json entry");
    }
    if report.history_lines_restored > 0 {
        println!(
            "  · restored {} history line{}",
            report.history_lines_restored,
            if report.history_lines_restored == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    Ok(())
}

pub fn trash_empty(ctx: &AppContext, older_than_days: Option<u64>) -> Result<()> {
    let data_dir = paths::claudepot_data_dir();
    let filter = ProjectTrashFilter {
        older_than: older_than_days
            .map(|d| std::time::Duration::from_secs(d.saturating_mul(86_400))),
    };
    // Preview first so the user knows what they're about to lose.
    let listing = project_trash::list(&data_dir, filter.clone())?;
    if listing.entries.is_empty() {
        println!("No trash to empty.");
        return Ok(());
    }

    if !ctx.yes {
        println!(
            "{} trashed project{} ({}) match the filter.",
            listing.entries.len(),
            if listing.entries.len() == 1 { "" } else { "s" },
            format_size(listing.total_bytes)
        );
        println!("Re-run with -y to permanently delete.");
        return Ok(());
    }

    let freed = project_trash::empty(&data_dir, filter)?;
    if ctx.json {
        println!("{}", serde_json::json!({"bytes_freed": freed}));
    } else {
        println!("\u{2713} Emptied trash, freed {}.", format_size(freed));
    }
    Ok(())
}
