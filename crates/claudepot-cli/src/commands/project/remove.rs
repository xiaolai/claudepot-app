//! `remove` verb — single-target trash with manifest snapshot.
//!
//! Sub-module of `commands/project.rs`; see that file's header for
//! the layout rationale and the shared gate/formatting helpers.

use super::*;

use claudepot_core::project_remove::{
    remove_project as core_remove_project, remove_project_preview, RemoveArgs,
};

/// Build the args bundle from the user's input plus the standard
/// claudepot path layout. Pulled out so dry-run and execute share
/// exactly the same resolution path.
fn build_remove_args<'a>(
    target: &'a str,
    config_dir: &'a std::path::Path,
    claude_json: &'a std::path::Path,
    history: &'a std::path::Path,
    snapshots: &'a std::path::Path,
    locks: &'a std::path::Path,
    data_dir: &'a std::path::Path,
) -> RemoveArgs<'a> {
    RemoveArgs {
        config_dir,
        claude_json_path: Some(claude_json),
        history_path: Some(history),
        snapshots_dir: snapshots,
        locks_dir: locks,
        data_dir,
        target,
    }
}

/// Print the three-block disclosure (Removing / Not touching /
/// Recoverable until). Same shape the GUI modal renders, so the CLI
/// and GUI agree on what the user is being asked to confirm.
fn print_remove_disclosure(
    preview: &claudepot_core::project_remove::RemovePreview,
    config_dir: &std::path::Path,
) {
    println!();
    println!("Removing:");
    if preview.artifact_dir_present {
        let cc_dir = config_dir.join("projects").join(&preview.slug);
        println!("  {}", cc_dir.display());
    } else {
        // Config-only project: no artifact dir to name. Say so, or the
        // disclosure points at a path that isn't there.
        println!("  (no session directory — config entries only)");
    }
    let mut details = Vec::new();
    if preview.session_count > 0 {
        details.push(format!(
            "{} session{}",
            preview.session_count,
            if preview.session_count == 1 { "" } else { "s" }
        ));
    }
    if preview.bytes > 0 {
        details.push(format_size(preview.bytes));
    }
    // Render-if-nonzero: a config-only project has no mtime, and
    // "last touched unknown" is noise.
    if let Some(last) = preview.last_modified.map(format_relative_time) {
        details.push(format!("last touched {last}"));
    }
    if preview.claude_json_entry_present {
        details.push("with .claude.json entry".to_string());
    }
    if preview.history_lines_count > 0 {
        details.push(format!(
            "{} history line{}",
            preview.history_lines_count,
            if preview.history_lines_count == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    println!("  {}", details.join(" \u{00b7} "));

    println!();
    println!("Not touching:");
    if let Some(orig) = preview.original_path.as_deref() {
        println!("  {}  (your actual project files)", orig);
    } else {
        println!("  (the underlying cwd, whichever it was)");
    }

    println!();
    if preview.artifact_dir_present {
        let cutoff = chrono::Utc::now() + chrono::Duration::days(30);
        println!(
            "Recoverable until: {} (30 days), via `claudepot project trash restore <id>`",
            cutoff.format("%Y-%m-%d")
        );
    } else {
        // No artifact dir means no trash entry — promising a
        // `trash restore <id>` here would be a lie. The batch
        // helpers' snapshot is the only way back.
        println!("Nothing to trash — a snapshot of the stripped entries is written to");
        println!("  {}", snapshots_dir().display());
    }
}

pub fn remove(ctx: &AppContext, target: &str, dry_run: bool) -> Result<()> {
    if !dry_run {
        gate_on_pending_journals(false)?;
    }
    let config_dir = paths::claude_config_dir();
    let claude_json =
        paths::claude_json_path().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let history = config_dir.join("history.jsonl");
    let snapshots = snapshots_dir();
    let locks = locks_dir();
    let data_dir = paths::claudepot_data_dir();

    let args = build_remove_args(
        target,
        &config_dir,
        &claude_json,
        &history,
        &snapshots,
        &locks,
        &data_dir,
    );
    let preview = remove_project_preview(&args)?;

    if preview.has_live_session {
        eprintln!(
            "\u{26a0}  Live CC session detected for {}. Close it before removing.",
            preview.original_path.as_deref().unwrap_or(&preview.slug)
        );
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "error": "live_session",
                    "slug": preview.slug,
                    "original_path": preview.original_path
                })
            );
        }
        anyhow::bail!("refusing to remove project with a live session");
    }

    if ctx.json && (dry_run || !ctx.yes) {
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    if !ctx.yes || dry_run {
        print_remove_disclosure(&preview, &config_dir);
        if !dry_run {
            println!();
            if preview.artifact_dir_present {
                println!("Re-run with -y to confirm. (The project moves to recoverable trash.)");
            } else {
                // Config-only: nothing is trashed, so don't promise a
                // recoverable-trash entry that won't exist.
                println!(
                    "Re-run with -y to confirm. (Config entries are stripped; a snapshot is kept.)"
                );
            }
        }
        return Ok(());
    }

    let result = core_remove_project(&args)?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    match result.trash_id.as_deref() {
        Some(id) => println!(
            "\u{2713} Trashed {} ({}). Restore with `claudepot project trash restore {}`.",
            result.slug,
            format_size(result.bytes),
            id
        ),
        // Config-only removal: nothing was trashed, so there is no
        // restore id to hand back.
        None => println!(
            "\u{2713} Removed config entries for {} (no session directory existed).",
            result.slug
        ),
    }
    if result.claude_json_entry_removed {
        println!("  · pruned ~/.claude.json entry");
    }
    if result.history_lines_removed > 0 {
        println!(
            "  · pruned {} history line{}",
            result.history_lines_removed,
            if result.history_lines_removed == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    Ok(())
}
