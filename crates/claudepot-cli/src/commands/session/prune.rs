//! `prune` verb — wholesale-delete sessions matching a filter.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

#[allow(clippy::too_many_arguments)]
pub fn prune_cmd(
    ctx: &AppContext,
    older_than: Option<&str>,
    larger_than: Option<&str>,
    project: Vec<String>,
    has_error: bool,
    sidechain: bool,
    execute: bool,
) -> Result<()> {
    use claudepot_core::session_prune::{execute_prune, plan_prune, PruneFilter};
    let mut filter = PruneFilter::default();
    if let Some(s) = older_than {
        filter.older_than = Some(parse_duration(s)?);
    }
    if let Some(s) = larger_than {
        filter.larger_than = Some(parse_size(s)?);
    }
    filter.project = project.iter().map(PathBuf::from).collect();
    filter.has_error = if has_error { Some(true) } else { None };
    filter.is_sidechain = if sidechain { Some(true) } else { None };

    let cfg = paths::claude_config_dir();
    let plan = plan_prune(&cfg, &filter).context("plan prune")?;

    if plan.entries.is_empty() {
        if ctx.json {
            print_json(&plan);
        } else {
            println!("No sessions match the filter.");
        }
        return Ok(());
    }

    if !execute {
        if ctx.json {
            print_json(&plan);
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
        print_json(&report);
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

