//! `move` verb — move/rename a project and migrate CC state. The
//! file is named `rename.rs` because `move` is a Rust keyword.
//!
//! Sub-module of `commands/project.rs`; see that file's header for
//! the layout rationale and the shared gate helpers.

use super::*;

/// Flag bundle for `claudepot project move`, flattened into the
/// `ProjectAction::Move` variant in `main.rs` (the in-tree
/// `ExportArgs` pattern). Field docs are the user-visible `--help`
/// text.
#[derive(Debug, clap::Args)]
pub struct MoveArgs {
    /// Current project path
    pub old_path: String,
    /// New project path
    pub new_path: String,
    /// Only update CC state, don't move the actual directory
    #[arg(long)]
    pub no_move: bool,
    /// Merge CC data if target already has sessions
    #[arg(long, conflicts_with = "overwrite")]
    pub merge: bool,
    /// Overwrite CC data at target
    #[arg(long, conflicts_with = "merge")]
    pub overwrite: bool,
    /// Proceed even if Claude is running in the directory
    #[arg(long)]
    pub force: bool,
    /// Show what would happen without making changes
    #[arg(long)]
    pub dry_run: bool,
    /// Proceed despite unresolved pending rename journals (last-resort)
    #[arg(long)]
    pub ignore_pending_journals: bool,
}

pub fn move_project(ctx: &AppContext, args: MoveArgs) -> Result<()> {
    let MoveArgs {
        old_path,
        new_path,
        no_move,
        merge,
        overwrite,
        force,
        dry_run,
        ignore_pending_journals,
    } = args;
    let (old_path, new_path) = (old_path.as_str(), new_path.as_str());
    if !dry_run {
        gate_on_pending_journals(ignore_pending_journals)?;
    }
    let config_dir = paths::claude_config_dir();
    // `~/.claude.json` is the sibling config file to `~/.claude/`.
    let claude_json_path = paths::claude_json_path();
    // Default snapshot location per spec §6 — now rooted under
    // Claudepot's repair tree rather than `<config_dir>/claudepot/`.
    let snapshots_dir = Some(paths::claudepot_repair_dir().join("snapshots"));

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
        claudepot_state_dir: Some(paths::claudepot_repair_dir()),
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
            if result.jsonl_lines_rewritten == 1 {
                ""
            } else {
                "s"
            },
            result.jsonl_files_modified,
            if result.jsonl_files_modified == 1 {
                ""
            } else {
                "s"
            },
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
        println!(
            "  \u{2713} Project-local .claude/settings.json autoMemoryDirectory rewritten (P9)"
        );
    }

    for warning in &result.warnings {
        println!("  \u{26a0} {}", warning);
    }

    println!();
    println!("Done.");
    Ok(())
}
