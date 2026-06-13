//! `prune` verb — wholesale-delete sessions matching a filter.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

/// Flag bundle for `claudepot session prune`, flattened into the
/// `SessionAction::Prune` variant in `main.rs`.
#[derive(Debug, clap::Args)]
pub struct PruneArgs {
    /// Match sessions whose last activity is older than the given
    /// duration. Accepts `7d`, `24h`, `90m`, `3600s`.
    #[arg(long)]
    pub older_than: Option<String>,
    /// Match sessions whose size is at least the given value.
    /// Accepts `10MB`, `500KB`, `1024`.
    #[arg(long)]
    pub larger_than: Option<String>,
    /// Repeatable: narrow to sessions whose cwd equals one of these.
    #[arg(long)]
    pub project: Vec<String>,
    /// Only include sessions that recorded an error.
    #[arg(long)]
    pub has_error: bool,
    /// Only include sidechain (subagent) sessions.
    #[arg(long)]
    pub sidechain: bool,
    /// Actually move files into the trash. Without this flag,
    /// prune only prints the plan.
    #[arg(long)]
    pub execute: bool,
}

pub fn prune_cmd(ctx: &AppContext, args: PruneArgs) -> Result<()> {
    use claudepot_core::session_prune::{execute_prune, plan_prune, PruneFilter};
    let PruneArgs {
        older_than,
        larger_than,
        project,
        has_error,
        sidechain,
        execute,
    } = args;
    let mut filter = PruneFilter::default();
    if let Some(s) = older_than.as_deref() {
        filter.older_than = Some(parse_duration(s)?);
    }
    if let Some(s) = larger_than.as_deref() {
        filter.larger_than = Some(parse_size(s)?);
    }
    filter.project = project.iter().map(PathBuf::from).collect();
    filter.has_error = if has_error { Some(true) } else { None };
    filter.is_sidechain = if sidechain { Some(true) } else { None };

    let cfg = paths::claude_config_dir();
    let plan = plan_prune(&cfg, &filter).context("plan prune")?;

    if plan.entries.is_empty() {
        if ctx.json {
            print_json(&plan)?;
        } else {
            println!("No sessions match the filter.");
        }
        return Ok(());
    }

    if !execute {
        if ctx.json {
            print_json(&plan)?;
            return Ok(());
        }
        println!("Plan (dry-run):");
        for e in &plan.entries {
            println!(
                "  - {}    {}    {}",
                e.file_path.display(),
                format_size(e.size_bytes),
                e.last_ts_ms
                    .map(format_ts_ms)
                    .unwrap_or_else(|| "—".to_string())
            );
        }
        println!(
            "Total: {} file(s), {} → trash",
            plan.entries.len(),
            format_size(plan.total_bytes)
        );
        println!("Run with --execute to apply. Trash retained for 7 days.");
        return Ok(());
    }

    let data_dir = paths::claudepot_data_dir();
    let sink = claudepot_core::project_progress::NoopSink;
    let report = execute_prune(&data_dir, &plan, &sink).context("execute prune")?;
    if ctx.json {
        print_json(&report)?;
        return Ok(());
    }
    println!(
        "Moved {} file(s) to trash, {} freed.",
        report.moved.len(),
        format_size(report.freed_bytes)
    );
    for (p, reason) in &report.failed {
        eprintln!("  ✗ {}: {}", p.display(), reason);
    }
    Ok(())
}
