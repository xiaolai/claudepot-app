//! `claudepot memory` — list / view / log per-project memory artifacts.
//!
//! Read-only by design (per the v1 plan) — no edit verbs. The display
//! mirrors what the GUI's MemoryPane shows so users get the same data
//! whether they're looking through the desktop app or the terminal.

use crate::AppContext;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, Utc};
use claudepot_core::memory_log::{ChangeQuery, DiffOmitReason, MemoryLog};
use claudepot_core::memory_view::{
    enumerate_project_memory, read_memory_content, MemoryFileRole, MemoryFileSummary,
    ProjectMemoryAnchor,
};
use claudepot_core::paths::claudepot_data_dir;
use claudepot_core::project_helpers::resolve_path;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Resolve `--project <PATH>` (or default to the current working dir)
/// to an absolute, OS-canonical path. Used by every memory verb.
fn resolve_project(project: Option<&str>) -> Result<PathBuf> {
    let raw = match project {
        Some(p) => p.to_string(),
        None => std::env::current_dir()?.to_string_lossy().into_owned(),
    };
    let resolved =
        resolve_path(&raw).map_err(|e| anyhow!("resolve project path {}: {}", raw, e))?;
    Ok(PathBuf::from(resolved))
}

fn open_log() -> Result<MemoryLog> {
    let path = claudepot_data_dir().join("memory_changes.db");
    MemoryLog::open(&path).with_context(|| format!("open memory log at {}", path.display()))
}

fn role_label(role: MemoryFileRole) -> &'static str {
    match role {
        MemoryFileRole::ClaudeMdProject => "project · CLAUDE.md",
        MemoryFileRole::ClaudeMdProjectLocal => "project · .claude/CLAUDE.md",
        MemoryFileRole::AutoMemoryIndex => "auto-memory · MEMORY.md",
        MemoryFileRole::AutoMemoryTopic => "auto-memory · topic",
        MemoryFileRole::KairosLog => "auto-memory · daily log",
        MemoryFileRole::ClaudeMdGlobal => "global · CLAUDE.md",
    }
}

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
    stats: &std::collections::HashMap<
        PathBuf,
        &claudepot_core::memory_log::MemoryFileStats,
    >,
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

fn format_ns_relative(ns: i64) -> String {
    let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let delta_ns = (now_ns - ns).max(0);
    let secs = delta_ns / 1_000_000_000;
    if secs < 60 {
        return format!("{}s ago", secs);
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{}m ago", mins);
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("{}h ago", hours);
    }
    let days = hours / 24;
    if days < 30 {
        return format!("{}d ago", days);
    }
    // Fall back to absolute date for older entries.
    let secs = ns / 1_000_000_000;
    let nsec = (ns % 1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nsec)
        .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string())
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

/// `claudepot memory view <FILE> [--project <PATH>]` — print contents.
pub async fn view(_ctx: &AppContext, file: &str, project: Option<&str>) -> Result<()> {
    let project_root = resolve_project(project)?;
    let target = resolve_memory_file(&project_root, file)?;
    let content = read_memory_content(&target, &[project_root])
        .with_context(|| format!("read memory file {}", target.display()))?;
    print!("{}", content);
    if !content.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn resolve_memory_file(project_root: &Path, file: &str) -> Result<PathBuf> {
    let p = Path::new(file);
    if p.is_absolute() && p.exists() {
        return Ok(p.to_path_buf());
    }
    let anchor = ProjectMemoryAnchor::for_project(project_root);
    let candidates: Vec<PathBuf> = vec![
        project_root.join(file),
        project_root.join(".claude").join(file),
        anchor.auto_memory_dir.join(file),
        claudepot_core::paths::claude_config_dir().join(file),
    ];
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(anyhow!(
        "memory file not found: {}. Try `claudepot memory list` to see available files.",
        file
    ))
}

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
