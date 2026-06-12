//! `list` verb — table of a project's memory files.
//!
//! Sub-module of `commands/memory.rs`; see that file's header for
//! the per-verb layout rationale and the shared resolution helpers.

use super::*;

/// `claudepot memory list [--project <PATH>]`. Honors `--json`.
pub async fn list(ctx: &AppContext, project: Option<&str>) -> Result<()> {
    let project_root = resolve_project(project)?;
    let result = enumerate_project_memory(&project_root, true)
        .with_context(|| format!("enumerate memory for {}", project_root.display()))?;
    let log = open_log().ok();
    let stats = log
        .as_ref()
        .and_then(|l| l.project_file_stats(&result.anchor.slug).ok())
        .unwrap_or_default();
    let stats_by_path: std::collections::HashMap<_, _> =
        stats.iter().map(|s| (s.abs_path.clone(), s)).collect();

    if ctx.json {
        let rows: Vec<_> = result
            .files
            .iter()
            .map(|f| {
                let stat = stats_by_path.get(&f.abs_path);
                json!({
                    "path": f.abs_path,
                    "role": f.role,
                    "size_bytes": f.size_bytes,
                    "mtime_unix_ns": f.mtime_unix_ns,
                    "line_count": f.line_count,
                    "lines_past_cutoff": f.lines_past_cutoff,
                    "last_change_unix_ns": stat.and_then(|s| s.last_change_unix_ns),
                    "change_count_30d": stat.map(|s| s.change_count_30d).unwrap_or(0),
                })
            })
            .collect();
        let body = json!({
            "anchor": {
                "project_root": result.anchor.project_root,
                "auto_memory_anchor": result.anchor.auto_memory_anchor,
                "slug": result.anchor.slug,
                "auto_memory_dir": result.anchor.auto_memory_dir,
            },
            "files": rows,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }

    if !ctx.quiet {
        eprintln!(
            "Project:         {}\nAuto-memory dir: {}\n",
            result.anchor.project_root.display(),
            result.anchor.auto_memory_dir.display()
        );
    }
    print_files_table(&result.files, &stats_by_path);
    Ok(())
}

fn print_files_table(
    files: &[MemoryFileSummary],
    stats: &std::collections::HashMap<PathBuf, &claudepot_core::memory_log::MemoryFileStats>,
) {
    if files.is_empty() {
        println!("No memory files yet.");
        return;
    }
    let header = format!(
        "  {:<28}  {:<6}  {:>9}  {:>12}  {:>14}  {:>9}",
        "Role", "Lines", "Size", "Cutoff", "Last change", "Edits/30d"
    );
    println!("{header}");
    println!(
        "  {:<28}  {:<6}  {:>9}  {:>12}  {:>14}  {:>9}",
        "────", "─────", "────", "──────", "───────────", "─────────"
    );
    for f in files {
        let stat = stats.get(&f.abs_path);
        let last = stat
            .and_then(|s| s.last_change_unix_ns)
            .map(format_ns_relative)
            .unwrap_or_else(|| "—".to_string());
        let cutoff = match f.lines_past_cutoff {
            Some(n) if n > 0 => format!("⚠ +{n}"),
            _ => "—".to_string(),
        };
        println!(
            "  {:<28}  {:<6}  {:>9}  {:>12}  {:>14}  {:>9}",
            role_label(f.role),
            f.line_count,
            pretty_bytes(f.size_bytes),
            cutoff,
            last,
            stat.map(|s| s.change_count_30d).unwrap_or(0),
        );
        println!("    {}", f.abs_path.display());
    }
}

fn pretty_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}
