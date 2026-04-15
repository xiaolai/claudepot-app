//! Dry-run planning and display formatting for the project module.

use crate::error::ProjectError;
use crate::project_helpers::{
    count_files_with_ext, dir_size, estimate_history_matches, find_project_dir_by_prefix,
};
use crate::project_sanitize::MAX_SANITIZED_LENGTH;
use crate::project_types::DryRunPlan;
use std::fs;
use std::path::Path;

pub(crate) fn compute_dry_run_plan(
    config_dir: &Path,
    old_norm: &str,
    _new_norm: &str,
    old_san: &str,
    new_san: &str,
    scenario: &super::project::MoveScenario,
) -> Result<DryRunPlan, ProjectError> {
    // Same long-path prefix-fallback as move_project (see spec §2).
    let cc_old_exact = config_dir.join("projects").join(old_san);
    let cc_old = if cc_old_exact.exists() {
        cc_old_exact
    } else if old_san.len() >= MAX_SANITIZED_LENGTH {
        find_project_dir_by_prefix(config_dir, old_san)?.unwrap_or(cc_old_exact)
    } else {
        cc_old_exact
    };
    let cc_new = config_dir.join("projects").join(new_san);

    let (session_count, cc_dir_size) = if cc_old.exists() {
        (count_files_with_ext(&cc_old, "jsonl"), dir_size(&cc_old))
    } else {
        (0, 0)
    };

    let estimated_history_lines = estimate_history_matches(config_dir, old_norm);

    let conflict = if old_san != new_san && cc_new.exists() {
        let is_empty = fs::read_dir(&cc_new)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true);
        if is_empty {
            None
        } else {
            Some(format!(
                "CC dir already exists at '{}' (non-empty). Use --merge or --overwrite.",
                new_san
            ))
        }
    } else {
        None
    };

    // P6 preview: count jsonl/meta.json files under the CC dir.
    let estimated_jsonl_files = if cc_old.exists() {
        count_rewrite_candidates(&cc_old)
    } else {
        0
    };

    // P7 preview: would the ~/.claude.json key rename fire? We don't
    // have the path here (caller-provided), so leave it true if the
    // map key rename is plausibly needed (old != new).
    let would_rewrite_claude_json = old_san != new_san;

    // P8 preview: would auto-memory move? Only if the git root will
    // change, which is true whenever the project is (or is inside)
    // the git root and the path changes.
    let would_move_memory_dir = {
        let old_root = crate::project_memory::find_canonical_git_root(Path::new(old_norm))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| old_norm.to_string());
        let new_root = crate::project_memory::find_canonical_git_root(Path::new(_new_norm))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| _new_norm.to_string());
        old_root != new_root
    };

    // P9 preview: would project-local settings.json rewrite? Check if
    // the source project has a .claude/settings.json at all.
    let would_rewrite_project_settings = Path::new(old_norm)
        .join(".claude")
        .join("settings.json")
        .exists();

    Ok(DryRunPlan {
        would_move_dir: *scenario == super::project::MoveScenario::MoveAndUpdate,
        old_cc_dir: old_san.to_string(),
        new_cc_dir: new_san.to_string(),
        session_count,
        cc_dir_size,
        estimated_history_lines,
        conflict,
        estimated_jsonl_files,
        would_rewrite_claude_json,
        would_move_memory_dir,
        would_rewrite_project_settings,
    })
}

/// Recursively count `.jsonl` and `.meta.json` files — the P6 rewrite
/// target set.
fn count_rewrite_candidates(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                n += count_rewrite_candidates(&path);
            } else if ft.is_file() {
                let s = path.to_string_lossy();
                if s.ends_with(".jsonl") || s.ends_with(".meta.json") {
                    n += 1;
                }
            }
        }
    }
    n
}

pub(crate) fn format_dry_run_plan(plan: &DryRunPlan, old_norm: &str, new_norm: &str) -> String {
    let mut out = String::from("Dry run \u{2014} no changes will be made.\n\nWould:\n");

    let mut step = 1;
    if plan.would_move_dir {
        out.push_str(&format!(
            "  {}. Move {} \u{2192} {}\n",
            step, old_norm, new_norm
        ));
        step += 1;
    }

    if plan.old_cc_dir != plan.new_cc_dir {
        out.push_str(&format!(
            "  {}. Rename CC dir: {} \u{2192} {}\n     ({} sessions, {})\n",
            step,
            plan.old_cc_dir,
            plan.new_cc_dir,
            plan.session_count,
            format_size(plan.cc_dir_size)
        ));
        step += 1;
    }

    if plan.estimated_history_lines > 0 {
        out.push_str(&format!(
            "  {}. Rewrite ~{} history.jsonl entries\n",
            step, plan.estimated_history_lines
        ));
        step += 1;
    }

    if plan.estimated_jsonl_files > 0 {
        out.push_str(&format!(
            "  {}. Rewrite cwd fields across {} session/subagent files (P6)\n",
            step, plan.estimated_jsonl_files
        ));
        step += 1;
    }

    if plan.would_rewrite_claude_json {
        out.push_str(&format!(
            "  {}. Rewrite ~/.claude.json projects map key (P7)\n",
            step
        ));
        step += 1;
    }

    if plan.would_move_memory_dir {
        out.push_str(&format!(
            "  {}. Move auto-memory dir for git-root change (P8)\n",
            step
        ));
        step += 1;
    }

    if plan.would_rewrite_project_settings {
        out.push_str(&format!(
            "  {}. Rewrite project-local .claude/settings.json autoMemoryDirectory (P9)\n",
            step
        ));
        step += 1;
    }
    let _ = step;

    if let Some(ref conflict) = plan.conflict {
        out.push_str(&format!("\nConflict: {}\n", conflict));
    } else {
        out.push_str("\nNo conflicts detected.\n");
    }

    out
}

/// Format bytes as human-readable size.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
