//! `log` verb — recent change-log entries for a project's memory.
//!
//! Sub-module of `commands/memory.rs`; see that file's header for
//! the per-verb layout rationale and the shared resolution helpers.

use super::*;

/// `claudepot memory log [--project <PATH>] [--file <FILE>]
/// [--limit N] [--show-diff]` — print recent change-log entries.
pub async fn log(
    ctx: &AppContext,
    project: Option<&str>,
    file: Option<&str>,
    limit: Option<usize>,
    show_diff: bool,
) -> Result<()> {
    let project_root = resolve_project(project)?;
    let anchor = ProjectMemoryAnchor::for_project(&project_root);
    let log = open_log()?;
    let q = ChangeQuery {
        limit: Some(limit.unwrap_or(50)),
        ..Default::default()
    };

    let rows = match file {
        Some(f) => {
            let target = resolve_memory_file(&project_root, f)?;
            log.query_for_path(&target, &q)?
        }
        None => log.query_for_project(&anchor.slug, &q)?,
    };

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("No change-log entries.");
        return Ok(());
    }

    println!(
        "  {:<14}  {:<10}  {:<28}  {:<28}",
        "When", "Type", "Role", "File"
    );
    println!(
        "  {:<14}  {:<10}  {:<28}  {:<28}",
        "────", "────", "────", "────"
    );
    for r in &rows {
        let kind = match r.change_type {
            claudepot_core::memory_log::ChangeType::Created => "created",
            claudepot_core::memory_log::ChangeType::Modified => "modified",
            claudepot_core::memory_log::ChangeType::Deleted => "deleted",
        };
        let basename = r
            .abs_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        println!(
            "  {:<14}  {:<10}  {:<28}  {:<28}",
            format_ns_relative(r.detected_at_ns),
            kind,
            role_label(r.role),
            basename
        );
        if show_diff {
            if let Some(diff) = &r.diff_text {
                println!();
                for line in diff.lines() {
                    println!("    {line}");
                }
                println!();
            } else if let Some(reason) = r.diff_omit_reason {
                let label = match reason {
                    DiffOmitReason::TooLarge => "(diff omitted: file too large)",
                    DiffOmitReason::Binary => "(diff omitted: binary file)",
                    DiffOmitReason::Endpoint => "(no diff: creation/deletion or no-op write)",
                    DiffOmitReason::Baseline => "(baseline: first time seen)",
                };
                println!("    {label}");
            }
        }
    }
    Ok(())
}
