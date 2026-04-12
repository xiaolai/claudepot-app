//! Dry-run planning and display formatting for the project module.

use crate::error::ProjectError;
use crate::project_helpers::{count_files_with_ext, dir_size, estimate_history_matches};
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
    let cc_old = config_dir.join("projects").join(old_san);
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

    Ok(DryRunPlan {
        would_move_dir: *scenario == super::project::MoveScenario::MoveAndUpdate,
        old_cc_dir: old_san.to_string(),
        new_cc_dir: new_san.to_string(),
        session_count,
        cc_dir_size,
        estimated_history_lines,
        conflict,
    })
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
    }

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
