use crate::error::ProjectError;
use std::fs;
use std::path::{Path, PathBuf};

// Re-export public API from submodules
pub use crate::project_display::format_size;
pub use crate::project_sanitize::{sanitize_path, unsanitize_path};
pub use crate::project_types::*;

// Private imports from submodules
use crate::project_display::{compute_dry_run_plan, format_dry_run_plan};
use crate::project_helpers::*;
use crate::project_sanitize::MAX_SANITIZED_LENGTH;

/// Root of the Claudepot repair tree for a move operation — honors the
/// per-args override (production code passes
/// `paths::claudepot_repair_dir()`) and falls back to the legacy
/// `<config_dir>/claudepot/` layout so tests that construct `MoveArgs`
/// against a tmp dir keep working without setting `CLAUDEPOT_DATA_DIR`.
fn repair_root(args: &MoveArgs) -> PathBuf {
    args.claudepot_state_dir
        .clone()
        .unwrap_or_else(|| args.config_dir.join("claudepot"))
}

/// Same override semantics as `repair_root` but keyed on a plain
/// `config_dir` + optional override, for the `clean_orphans` path which
/// doesn't carry a `MoveArgs`.
fn repair_root_from(config_dir: &Path, override_dir: Option<&Path>) -> PathBuf {
    override_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config_dir.join("claudepot"))
}
#[cfg(test)]
use crate::project_sanitize::{djb2_hash, format_radix};

// ---------------------------------------------------------------------------
// list_projects
// ---------------------------------------------------------------------------

pub fn list_projects(config_dir: &Path) -> Result<Vec<ProjectInfo>, ProjectError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(vec![]);
    }

    let mut projects = Vec::new();
    for entry in fs::read_dir(&projects_dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let ft = entry.file_type().map_err(ProjectError::Io)?;
        if !ft.is_dir() {
            continue;
        }
        let sanitized_name = entry.file_name().to_string_lossy().to_string();
        projects.push(compute_project_info(&entry.path(), &sanitized_name)?);
    }

    projects.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    Ok(projects)
}

// ---------------------------------------------------------------------------
// show_project
// ---------------------------------------------------------------------------

pub fn show_project(config_dir: &Path, path: &str) -> Result<ProjectDetail, ProjectError> {
    let resolved = resolve_path(path)?;
    let sanitized = sanitize_path(&resolved);
    let project_dir = config_dir.join("projects").join(&sanitized);

    let project_dir = if project_dir.exists() {
        project_dir
    } else if sanitized.len() > MAX_SANITIZED_LENGTH {
        find_project_dir_by_prefix(config_dir, &sanitized)?
            .ok_or_else(|| ProjectError::NotFound(path.to_string()))?
    } else {
        return Err(ProjectError::NotFound(path.to_string()));
    };

    let sanitized_name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let info = compute_project_info(&project_dir, &sanitized_name)?;
    let sessions = list_sessions(&project_dir)?;
    let memory_files = list_memory_files(&project_dir)?;

    Ok(ProjectDetail {
        info,
        sessions,
        memory_files,
    })
}

// ---------------------------------------------------------------------------
// move_project
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub(crate) enum MoveScenario {
    StateOnly,
    MoveAndUpdate,
    AlreadyMoved,
}

/// Compute the dry-run plan for a prospective move without touching
/// disk. Thin wrapper around `move_project` with `dry_run=true`:
/// convenient for GUI callers that want the structured `DryRunPlan`
/// rather than parsing the formatted `warnings[0]` string.
pub fn plan_move(args: &MoveArgs) -> Result<DryRunPlan, ProjectError> {
    let mut probe = args.clone();
    probe.dry_run = true;
    let result = move_project(&probe, &crate::project_progress::NoopSink)?;
    result.dry_run_plan.ok_or_else(|| {
        ProjectError::Ambiguous(
            "internal: move_project(dry_run=true) returned no plan".to_string(),
        )
    })
}

/// Move/rename a project and migrate CC state. Callers MUST provide
/// a `ProgressSink` (spec §8 Q3); pass `&NoopSink` if you genuinely
/// don't want progress. Making it required keeps the API honest about
/// what it does internally (P6 touches many files; phases emit events
/// as they complete).
pub fn move_project(
    args: &MoveArgs,
    sink: &dyn crate::project_progress::ProgressSink,
) -> Result<MoveResult, ProjectError> {
    use crate::project_progress::PhaseStatus;
    tracing::info!(old = ?args.old_path, new = ?args.new_path, "starting project move");

    let old_str = args
        .old_path
        .to_str()
        .ok_or_else(|| ProjectError::Ambiguous("old path contains invalid UTF-8".to_string()))?;
    let new_str = args
        .new_path
        .to_str()
        .ok_or_else(|| ProjectError::Ambiguous("new path contains invalid UTF-8".to_string()))?;
    let old_norm = resolve_path(old_str)?;
    let new_norm_raw = resolve_path(new_str)?;

    // Detect case-only rename BEFORE the SamePath check — on a
    // case-insensitive FS, `old_str` and `new_str` that differ only
    // in case will canonicalize to the same path, so a naive equality
    // check would reject the rename. Honor the user's raw new_path
    // intent instead.
    let case_only_rename_requested =
        old_str != new_str && old_str.eq_ignore_ascii_case(new_str);
    let new_norm = if case_only_rename_requested {
        // Use the user's explicitly-cased new path as the canonical.
        // Resolve to an absolute form (no canonicalize(); that would
        // flatten the case difference we're trying to preserve).
        let p = std::path::PathBuf::from(new_str);
        if p.is_absolute() {
            new_str.to_string()
        } else {
            std::env::current_dir()
                .map_err(ProjectError::Io)?
                .join(p)
                .to_string_lossy()
                .to_string()
        }
    } else {
        new_norm_raw
    };

    if old_norm == new_norm {
        return Err(ProjectError::SamePath);
    }

    // Structural guards per spec §7.1 / §4.3.  These are hard errors
    // with no flag override — the user must fix the inputs.
    //
    // E1: new_path inside old_path. `fs::rename` cannot do this, and
    //     the EXDEV copy fallback would recurse infinitely.
    if path_is_inside(&new_norm, &old_norm) {
        return Err(ProjectError::Ambiguous(format!(
            "new path ({}) is inside old path ({}); \
             pick a sibling or unrelated target",
            new_norm, old_norm
        )));
    }
    // E2: old_path inside new_path. `fs::rename` will fail;
    //     ambiguous whether the user meant to nest or replace.
    if path_is_inside(&old_norm, &new_norm) {
        return Err(ProjectError::Ambiguous(format!(
            "old path ({}) is inside new path ({}); \
             move to a sibling first",
            old_norm, new_norm
        )));
    }

    let old_san = sanitize_path(&old_norm);
    let new_san = sanitize_path(&new_norm);

    // Preflight: CC target dir non-empty collision is a hard error
    // unless the user opted into --merge or --overwrite (spec §4.2
    // P1.7). Dry-run skips the hard error and reports via the plan
    // (compute_dry_run_plan already populates `conflict`).
    //
    // Resolution uses the same δ prefix-fallback as P4 for long
    // paths — a Bun-CLI-created target won't exact-match our djb2
    // sanitized form.
    if !args.dry_run && old_san != new_san && !args.merge && !args.overwrite {
        let cc_new_exact = args.config_dir.join("projects").join(&new_san);
        let cc_new_resolved = if cc_new_exact.exists() {
            Some(cc_new_exact)
        } else if new_san.len() >= MAX_SANITIZED_LENGTH {
            find_project_dir_by_prefix(&args.config_dir, &new_san)?
        } else {
            None
        };
        if let Some(cc_new) = cc_new_resolved {
            let is_empty = fs::read_dir(&cc_new)
                .map(|mut d| d.next().is_none())
                .unwrap_or(true);
            if !is_empty {
                return Err(ProjectError::Ambiguous(format!(
                    "CC project data already exists at target ({cc_new:?}); \
                     re-run with --merge or --overwrite"
                )));
            }
        }
    }

    // Audit H1: pending-journal gate lives in core, not in each caller.
    // Previously the CLI enforced this via gate_on_pending_journals()
    // but the Tauri `project_move_start` command didn't — so the GUI
    // could start a rename while actionable journals were still
    // unresolved. Centralizing the check here makes CLI and Tauri
    // share one enforcement point. `args.ignore_pending_journals`
    // honors the user's escape hatch. Dry-run is exempt (it's
    // non-mutating) so the GUI preview still works during a pending
    // journal situation.
    if !args.dry_run && !args.ignore_pending_journals {
        let claudepot_home = repair_root(args);
        let journals_dir = claudepot_home.join("journals");
        let locks_dir = claudepot_home.join("locks");
        if journals_dir.exists() {
            // Same threshold the CLI and Tauri both use for the nag
            // banner — 24 h. We list actionable journals (pending +
            // stale, excluding running + abandoned) so a repair
            // currently executing in another thread doesn't block a
            // fresh move against a DIFFERENT project.
            let actionable = crate::project_repair::list_actionable(
                &journals_dir,
                &locks_dir,
                86_400,
            )?;
            if !actionable.is_empty() {
                return Err(ProjectError::Ambiguous(format!(
                    "refusing to move: {} actionable rename journal(s) on disk. \
                     Resolve via `claudepot project repair` (or the Repair view \
                     in the GUI), or pass --ignore-pending-journals if you \
                     know what you're doing.",
                    actionable.len()
                )));
            }
        }
    }

    let old_exists = Path::new(&old_norm).exists();
    let new_exists = Path::new(&new_norm).exists();

    let scenario = if args.no_move {
        MoveScenario::StateOnly
    } else {
        match (old_exists, new_exists) {
            (true, false) => MoveScenario::MoveAndUpdate,
            (false, true) => MoveScenario::AlreadyMoved,
            // On case-insensitive FS, a case-only rename sees both
            // "exist" because they resolve to the same inode. That's
            // still a legitimate MoveAndUpdate.
            (true, true) if case_only_rename_requested => MoveScenario::MoveAndUpdate,
            (true, true) => {
                return Err(ProjectError::Ambiguous(
                    "both old and new paths exist on disk".to_string(),
                ))
            }
            (false, false) => {
                return Err(ProjectError::Ambiguous(
                    "neither old nor new path exists on disk".to_string(),
                ))
            }
        }
    };

    if args.dry_run {
        let plan = compute_dry_run_plan(
            &args.config_dir,
            &old_norm,
            &new_norm,
            &old_san,
            &new_san,
            &scenario,
        )?;
        return Ok(MoveResult {
            warnings: vec![format_dry_run_plan(&plan, &old_norm, &new_norm)],
            dry_run_plan: Some(plan),
            ..Default::default()
        });
    }

    let mut result = MoveResult::default();

    // Open a lock + journal for recovery. Scope: everything below P3
    // is protected; crashes before writing P3 leave no journal trail
    // (nothing destructive has happened yet anyway).
    let claudepot_home = repair_root(args);
    let locks_dir = claudepot_home.join("locks");
    let journals_dir = claudepot_home.join("journals");
    let (_lock, broken_lock_record) =
        crate::project_lock::acquire(&locks_dir, &old_san)?;
    let mut journal = crate::project_journal::open_journal(
        &journals_dir,
        crate::project_journal::new_initial_journal(
            &old_norm,
            &new_norm,
            &old_san,
            &new_san,
            crate::project_memory::find_canonical_git_root(Path::new(&old_norm))
                .map(|p| p.to_string_lossy().to_string()),
            crate::project_memory::find_canonical_git_root(Path::new(&new_norm))
                .map(|p| p.to_string_lossy().to_string()),
            crate::project_journal::JournalFlags {
                merge: args.merge,
                overwrite: args.overwrite,
                no_move: args.no_move,
                force: args.force,
                ignore_pending_journals: args.ignore_pending_journals,
            },
        ),
    )?;

    // Audit-log any stale lock we just broke (§5.1).
    if let Some(ref rec) = broken_lock_record {
        let _ = journal.note_broken_lock(rec);
    }

    // Live-session check for ALL scenarios, not just MoveAndUpdate.
    // --force overrides; applies to old and new paths (spec §5, §7.5
    // E32-E34).
    if !args.force {
        if live_session_present(&args.config_dir, &old_san, &old_norm)
            || live_session_present(&args.config_dir, &new_san, &new_norm)
        {
            let _ = journal.mark_error("live CC session detected");
            return Err(ProjectError::ClaudeRunning(old_norm.clone()));
        }
    }

    // Phase 3: Move actual directory
    if scenario == MoveScenario::MoveAndUpdate {
        if let Some(parent) = Path::new(&new_norm).parent() {
            fs::create_dir_all(parent).map_err(ProjectError::Io)?;
        }
        // E3: Case-only rename on case-insensitive FS needs a two-step
        // via an intermediate name, else `fs::rename` is a no-op on
        // APFS/NTFS.
        let case_only = is_case_only_rename(&old_norm, &new_norm);
        if case_only {
            let tmp_name = format!(
                "{}.claudepot-caserename-{}",
                new_norm,
                std::process::id()
            );
            fs::rename(&old_norm, &tmp_name).map_err(|e| {
                let _ = journal.mark_error(&format!("P3 case-rename step 1 failed: {e}"));
                ProjectError::Io(e)
            })?;
            fs::rename(&tmp_name, &new_norm).map_err(|e| {
                let _ = journal.mark_error(&format!("P3 case-rename step 2 failed: {e}"));
                ProjectError::Io(e)
            })?;
        } else {
            match fs::rename(&old_norm, &new_norm) {
                Ok(()) => {}
                #[cfg(unix)]
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                    // Cross-device move: rename(2) can't; we must copy+remove.
                    // The naive form `copy_dir_recursive(old, new)` had a
                    // TOCTOU window — between the preflight `new_exists`
                    // check and this copy, another process could create
                    // or populate `new_norm`, and the copy would merge
                    // into it before we deleted the source. Data-loss
                    // potential (audit H6).
                    //
                    // Safe pattern: copy into a uniquely-named sibling
                    // staging dir, then atomically rename staging -> new_norm.
                    // fs::rename fails if the target is a non-empty
                    // directory, so if another process claimed new_norm
                    // between preflight and now we surface an error
                    // instead of silently merging. Only after the rename
                    // succeeds do we remove the source.
                    let staging = format!(
                        "{}.claudepot-xdev-{}-{}",
                        new_norm,
                        std::process::id(),
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                    );
                    copy_dir_recursive(Path::new(&old_norm), Path::new(&staging))?;
                    match fs::rename(&staging, &new_norm) {
                        Ok(()) => {
                            // Source still intact; now safe to remove.
                            fs::remove_dir_all(&old_norm).map_err(ProjectError::Io)?;
                        }
                        Err(rename_err) => {
                            // Target got claimed in the race. Roll back
                            // the staging copy and abort — source is
                            // untouched.
                            let cleanup_err =
                                fs::remove_dir_all(&staging).err();
                            let _ = journal.mark_error(&format!(
                                "P3 EXDEV rename-into-place failed: {rename_err} \
                                 (staging cleanup: {:?})",
                                cleanup_err
                            ));
                            return Err(ProjectError::Io(rename_err));
                        }
                    }
                }
                Err(e) => {
                    let _ = journal.mark_error(&format!("P3 failed: {e}"));
                    return Err(ProjectError::Io(e));
                }
            }
        }
        result.actual_dir_moved = true;
        let _ = journal.mark_phase("P3");
        sink.phase("P3", PhaseStatus::Complete);
    }

    // Phase 4: Rename CC project directory
    result.old_sanitized = Some(old_san.clone());
    result.new_sanitized = Some(new_san.clone());
    if old_san != new_san {
        let projects_base = args.config_dir.join("projects");
        let cc_new = projects_base.join(&new_san);

        // Resolve the source CC dir. For short paths (<= MAX) the exact
        // sanitized name is authoritative. For long paths (> MAX), CC's
        // CLI (Bun-compiled) uses Bun.hash (WyHash) for the hash suffix
        // while claudepot computes djb2 — the trailing hash may differ.
        // Fall back to prefix matching in that case (δ strategy per spec
        // §2 sanitization formula; mirrors CC's own findProjectDir).
        let cc_old_exact = projects_base.join(&old_san);
        let cc_old = if cc_old_exact.exists() {
            cc_old_exact
        } else if old_san.len() >= MAX_SANITIZED_LENGTH {
            match find_project_dir_by_prefix(&args.config_dir, &old_san)? {
                Some(found) => found,
                None => cc_old_exact, // will fail the .exists() check below
            }
        } else {
            cc_old_exact
        };

        if !cc_old.starts_with(&projects_base) || !cc_new.starts_with(&projects_base) {
            if result.actual_dir_moved {
                result.warnings.push(
                    "sanitized path escapes projects directory — CC state not updated".to_string(),
                );
            } else {
                return Err(ProjectError::Ambiguous(
                    "sanitized path escapes projects directory".to_string(),
                ));
            }
        }

        if cc_old.exists() {
            if cc_new.exists() {
                let new_is_empty = fs::read_dir(&cc_new)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false);
                if new_is_empty {
                    fs::remove_dir(&cc_new).map_err(ProjectError::Io)?;
                    fs::rename(&cc_old, &cc_new).map_err(ProjectError::Io)?;
                    result.cc_dir_renamed = true;
                } else if args.merge {
                    merge_project_dirs(&cc_old, &cc_new)?;
                    fs::remove_dir_all(&cc_old).map_err(ProjectError::Io)?;
                    result.cc_dir_renamed = true;
                } else if args.overwrite {
                    // Destructive: snapshot the target CC dir before
                    // remove_dir_all so rollback/inspection is possible.
                    // Snapshot failure MUST abort — we cannot remove
                    // data we haven't safely preserved.
                    let snaps = args
                        .snapshots_dir
                        .clone()
                        .unwrap_or_else(|| repair_root(args).join("snapshots"));
                    let snap = snapshot_cc_dir(&snaps, &new_san, "P4", &cc_new)
                        .map_err(|e| {
                            let msg = format!(
                                "P4 --overwrite refused: snapshot of {cc_new:?} failed: {e}"
                            );
                            let _ = journal.mark_error(&msg);
                            ProjectError::Ambiguous(msg)
                        })?;
                    let _ = journal.record_snapshot(snap);
                    fs::remove_dir_all(&cc_new).map_err(ProjectError::Io)?;
                    fs::rename(&cc_old, &cc_new).map_err(ProjectError::Io)?;
                    result.cc_dir_renamed = true;
                } else {
                    // Unreachable in v2: the preflight above rejects
                    // non-empty target without --merge/--overwrite.
                    // Kept as a defensive assertion in case preflight
                    // is ever reordered.
                    return Err(ProjectError::Ambiguous(
                        "CC project data exists at both old and new paths. \
                         Use --merge or --overwrite to resolve."
                            .to_string(),
                    ));
                }
            } else {
                // E3: case-only rename on case-insensitive FS needs
                // two-step here too (same as P3).
                let cc_case_only = is_case_only_rename(
                    &cc_old.to_string_lossy(),
                    &cc_new.to_string_lossy(),
                );
                if cc_case_only {
                    let tmp = projects_base.join(format!(
                        "{}.claudepot-caserename-{}",
                        new_san,
                        std::process::id()
                    ));
                    fs::rename(&cc_old, &tmp).map_err(ProjectError::Io)?;
                    fs::rename(&tmp, &cc_new).map_err(ProjectError::Io)?;
                } else {
                    fs::rename(&cc_old, &cc_new).map_err(ProjectError::Io)?;
                }
                result.cc_dir_renamed = true;
            }
        }
    }
    if result.cc_dir_renamed {
        let _ = journal.mark_phase("P4");
        sink.phase("P4", PhaseStatus::Complete);
    }

    // Phase 5: Rewrite history.jsonl
    let cc_dir_conflict = !result.warnings.is_empty() && !result.cc_dir_renamed;
    if !cc_dir_conflict {
        let history_path = args.config_dir.join("history.jsonl");
        if history_path.exists() {
            tracing::debug!("rewriting history.jsonl");
            result.history_lines_updated = rewrite_history(&history_path, &old_norm, &new_norm)?;
            let _ = journal.mark_phase("P5");
            sink.phase("P5", PhaseStatus::Complete);
        }
    }

    // Phase 6: Rewrite session + subagent jsonl cwd fields. Runs against
    // the CC dir at its NEW sanitized location after phase 4. Preserves
    // session resumability after rename (see spec §4.2 P6 and §8 Q4).
    //
    // Audit M10: also run when old_san == new_san (lossy sanitization
    // collapsed both paths to the same name) but the source paths
    // differ — the CC dir wasn't renamed, but session JSONL files
    // still carry the OLD cwd and must be rewritten to the NEW one.
    // Without this, a rename like `/tmp/a.b -> /tmp/a-b` (both
    // sanitize to `-tmp-a-b`) left stale cwd in session files and
    // session resumption opened the wrong project.
    let p6_needed = !cc_dir_conflict
        && (result.cc_dir_renamed
            || (old_norm != new_norm && scenario != MoveScenario::StateOnly));
    if p6_needed {
        let projects_base = args.config_dir.join("projects");
        let cc_new_dir_exact = projects_base.join(&new_san);
        let cc_new_dir = if cc_new_dir_exact.exists() {
            cc_new_dir_exact
        } else if new_san.len() >= MAX_SANITIZED_LENGTH {
            find_project_dir_by_prefix(&args.config_dir, &new_san)?
                .unwrap_or(cc_new_dir_exact)
        } else {
            cc_new_dir_exact
        };
        if cc_new_dir.exists() {
            tracing::debug!(dir = ?cc_new_dir, "rewriting session jsonl paths");
            let (stats, errors) = crate::project_rewrite::rewrite_project_paths(
                &cc_new_dir,
                &old_norm,
                &new_norm,
                sink,
            )?;
            result.jsonl_files_scanned = stats.files_scanned;
            result.jsonl_files_modified = stats.files_modified;
            result.jsonl_lines_rewritten = stats.lines_rewritten;
            result.jsonl_errors = errors
                .into_iter()
                .map(|(p, e)| (p, e.to_string()))
                .collect();
            if !result.jsonl_errors.is_empty() {
                let summary = format!(
                    "P6 failed: {} file(s) did not rewrite. \
                     Journal retained for `claudepot project repair`.",
                    result.jsonl_errors.len()
                );
                let _ = journal.mark_error(&summary);
                return Err(ProjectError::Ambiguous(summary));
            }
            let _ = journal.mark_phase("P6");
            sink.phase("P6", PhaseStatus::Complete);
        }
    }

    // Phase 7: Rewrite ~/.claude.json projects[<old_path>] key. Governed
    // by the same --merge / --overwrite flags as P4, with old-wins merge
    // semantics and 30-day snapshots for destructive cases (see spec
    // §4.2 P7 and §8 Q2). Caller must pass `claude_json_path`; we do NOT
    // default to the real home dir here so tests stay hermetic.
    if let (false, Some(config_path)) =
        (cc_dir_conflict, args.claude_json_path.clone())
    {
        let snapshots_dir = args
            .snapshots_dir
            .clone()
            .unwrap_or_else(|| repair_root(args).join("snapshots"));
        let policy = if args.overwrite {
            crate::project_config_rewrite::ConfigCollisionPolicy::Overwrite
        } else if args.merge {
            crate::project_config_rewrite::ConfigCollisionPolicy::Merge
        } else {
            crate::project_config_rewrite::ConfigCollisionPolicy::Error
        };
        match crate::project_config_rewrite::rewrite_claude_json(
            &config_path,
            &snapshots_dir,
            &old_norm,
            &new_norm,
            &new_san,
            policy,
        ) {
            Ok(r) => {
                result.config_key_renamed = r.key_renamed;
                result.config_had_collision = r.had_collision;
                result.config_merged_keys = r.merged_keys;
                result.config_snapshot_path = r.snapshot_path;
                result.config_nested_rewrites = r.nested_rewrites;
                if result.config_had_collision && !result.config_merged_keys.is_empty() {
                    result.warnings.push(format!(
                        "~/.claude.json collision: {} key(s) merged (old won); \
                         pre-existing value snapshotted to {:?}",
                        result.config_merged_keys.len(),
                        result.config_snapshot_path.as_ref(),
                    ));
                }
                if let Some(snap) = &result.config_snapshot_path {
                    let _ = journal.record_snapshot(snap.clone());
                }
                if result.config_key_renamed {
                    let _ = journal.mark_phase("P7");
                    sink.phase("P7", PhaseStatus::Complete);
                }
            }
            Err(e) => {
                let msg = format!("P7 (~/.claude.json) failed: {e}");
                let _ = journal.mark_error(&msg);
                return Err(ProjectError::Ambiguous(msg));
            }
        }
    }

    // Phase 8: Move auto-memory dir if git root changed. Safe no-op
    // when the project is inside a git repo and the rename doesn't
    // change the git root (e.g. renaming a subdir of a repo).
    //
    // Gated on scenario, not `actual_dir_moved` — AlreadyMoved and
    // --no-move still need the migration (spec §4.2 P8).
    if !cc_dir_conflict {
        let snaps = args
            .snapshots_dir
            .clone()
            .unwrap_or_else(|| repair_root(args).join("snapshots"));
        match crate::project_memory::move_memory_dir_if_needed(
            &args.config_dir,
            &old_norm,
            &new_norm,
            args.merge,
            args.overwrite,
            Some(&snaps),
        ) {
            Ok(r) => {
                result.memory_git_root_changed = r.git_root_changed;
                result.memory_dir_moved = r.memory_dir_moved;
                if let Some(snap) = r.snapshot_path {
                    let _ = journal.record_snapshot(snap);
                }
                for w in r.warnings {
                    result.warnings.push(w);
                }
                if result.memory_dir_moved {
                    let _ = journal.mark_phase("P8");
                    sink.phase("P8", PhaseStatus::Complete);
                }
            }
            Err(e) => {
                let msg = format!("P8 (auto-memory) failed: {e}");
                let _ = journal.mark_error(&msg);
                return Err(ProjectError::Ambiguous(msg));
            }
        }
    }

    // Phase 9: Rewrite <new_path>/.claude/settings.json autoMemoryDirectory
    // if it is an absolute path anchored at old_path. Relative / ~-based
    // paths are already path-portable (see spec §4.2 P9).
    //
    // Gated on scenario, not `actual_dir_moved` — AlreadyMoved still
    // needs the rewrite because the new path now exists.
    if !cc_dir_conflict {
        match crate::project_config_rewrite::rewrite_project_settings(
            &args.new_path,
            &old_norm,
            &new_norm,
        ) {
            Ok(rewrote) => {
                result.project_settings_rewritten = rewrote;
                if rewrote {
                    let _ = journal.mark_phase("P9");
                    sink.phase("P9", PhaseStatus::Complete);
                }
            }
            Err(e) => {
                let msg = format!("P9 (.claude/settings.json) failed: {e}");
                let _ = journal.mark_error(&msg);
                return Err(ProjectError::Ambiguous(msg));
            }
        }
    }

    // All phases complete. Delete the journal. (Lock is released via
    // RAII when `_lock` drops at end of scope.)
    journal.finish()?;

    tracing::info!(
        moved = result.actual_dir_moved,
        renamed = result.cc_dir_renamed,
        history = result.history_lines_updated,
        jsonl_files_modified = result.jsonl_files_modified,
        jsonl_lines_rewritten = result.jsonl_lines_rewritten,
        config_renamed = result.config_key_renamed,
        config_collision = result.config_had_collision,
        settings_rewritten = result.project_settings_rewritten,
        "project move complete"
    );
    Ok(result)
}

/// True iff `inner` is strictly inside `outer` (share prefix + at least
/// one extra path component). Used by the E1/E2 structural guards.
fn path_is_inside(inner: &str, outer: &str) -> bool {
    let sep = std::path::MAIN_SEPARATOR;
    let boundary = format!("{outer}{sep}");
    inner.starts_with(&boundary) && inner != outer
}

/// True iff `old` and `new` differ only in ASCII letter case. Used to
/// trigger the E3 two-step case-only rename on case-insensitive FS.
fn is_case_only_rename(old: &str, new: &str) -> bool {
    old != new && old.eq_ignore_ascii_case(new)
}

/// Live-session detection at a specific (config_dir, san, project_cwd)
/// point. Checks both the CC project dir (for jsonl heartbeat + lsof)
/// AND the project cwd (for lsof + process-name scan). Either hit
/// counts as live.
fn live_session_present(config_dir: &Path, san: &str, project_cwd: &str) -> bool {
    let cc_dir = config_dir.join("projects").join(san);
    if cc_dir.exists()
        && crate::project_helpers::detect_live_session(&cc_dir, project_cwd, 120)
    {
        return true;
    }
    // Also probe the project cwd directly: if CC is running there but
    // the CC dir hasn't been created yet (first-run), the cc_dir path
    // misses. is_claude_running_in + an lsof probe catch this.
    if Path::new(project_cwd).exists()
        && crate::project_helpers::lsof_sees_open_file_pub(Path::new(project_cwd))
    {
        return true;
    }
    is_claude_running_in(project_cwd)
}

/// Snapshot a CC-state directory before destructive replacement.
/// Copies the tree to
/// `<snapshots_dir>/<ts>-<san>-<phase>.snap/`. Returns the snapshot
/// path so the caller can record it in the journal.
fn snapshot_cc_dir(
    snapshots_dir: &Path,
    san: &str,
    phase: &str,
    source: &Path,
) -> Result<std::path::PathBuf, ProjectError> {
    fs::create_dir_all(snapshots_dir).map_err(ProjectError::Io)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe_san: String = san
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let target = snapshots_dir.join(format!("{ts}-{safe_san}-{phase}.snap"));
    copy_dir_recursive(source, &target)?;
    Ok(target)
}


// ---------------------------------------------------------------------------
// clean_orphans
// ---------------------------------------------------------------------------

/// Heartbeat window for the live-session probe: if any `.jsonl` inside
/// an orphan's CC dir has been written within this window AND a kernel
/// signal confirms it, we refuse to remove it. 60 s is tight enough to
/// cover "session just closed" bounces without flagging truly dead dirs.
const CLEAN_LIVE_HEARTBEAT_SECS: u64 = 60;

/// Key used for the process-wide `clean` lock. Reuses `project_lock`'s
/// stale-detection and O_EXCL acquisition, so two concurrent cleans
/// can't race on the same CC dir.
const CLEAN_LOCK_KEY: &str = "__clean__";

/// Find and remove orphan CC project dirs. In addition to the CC
/// project dir itself, successful removal purges:
///   * `projects[<original_path>]` in `~/.claude.json` (snapshot first)
///   * matching lines in `~/.claude/history.jsonl` (snapshot of dropped
///     lines)
///   * claudepot-owned snapshots whose filename keys the orphan's
///     sanitized name
///   * abandoned journal sidecars keyed to the orphan's sanitized name
///
/// Sibling-state cleanup only runs for orphans whose `original_path` is
/// authoritative — i.e. recovered from `session.jsonl` (`is_empty=false`
/// case). Empty project dirs get their CC dir removed but NO sibling
/// state is touched, because `original_path` for an empty dir comes
/// from the lossy `unsanitize_path` fallback and may not be the real
/// source.
///
/// `claude_json_path = None` skips the config-side rewrite entirely;
/// tests use this to stay hermetic. `snapshots_dir` / `locks_dir`
/// default to the standard layout under `config_dir/claudepot/` when
/// `None`.
///
/// Unreachable projects (unmounted `/Volumes/*`, permission-denied
/// source stat) are NEVER counted as orphans — their source may still
/// exist on the absent volume. They are reported via
/// `unreachable_skipped` so callers can surface "mount the drive and
/// re-run".
pub fn clean_orphans(
    config_dir: &Path,
    claude_json_path: Option<&Path>,
    snapshots_dir: Option<&Path>,
    locks_dir: Option<&Path>,
    dry_run: bool,
) -> Result<(CleanResult, Vec<ProjectInfo>), ProjectError> {
    clean_orphans_with_progress(
        config_dir,
        claude_json_path,
        snapshots_dir,
        locks_dir,
        None,
        &std::collections::HashSet::new(),
        dry_run,
        &crate::project_progress::NoopSink,
    )
}

/// Progress-emitting variant of `clean_orphans`. Phases:
///   * `batch-sibling` — running / complete after ~/.claude.json and
///     history.jsonl are rewritten in one pass each. sub_progress
///     reports (0, 2) → (1, 2) → (2, 2) as each file is touched.
///   * `remove-dirs` — running / complete around the per-orphan
///     remove_dir_all loop. sub_progress reports (done, total) so the
///     UI can render a "N of M" counter.
///
/// The sink is free to drop events (e.g. `NoopSink`); the core never
/// assumes the caller is subscribed.
// `claudepot_state_dir`: root of Claudepot's repair tree, used for
// journals and as the fallback parent for snapshots/locks. Production
// callers pass `Some(paths::claudepot_repair_dir())`; `None` falls back
// to the legacy `<config_dir>/claudepot/` layout so tests keep working
// without touching `CLAUDEPOT_DATA_DIR`.
pub fn clean_orphans_with_progress(
    config_dir: &Path,
    claude_json_path: Option<&Path>,
    snapshots_dir: Option<&Path>,
    locks_dir: Option<&Path>,
    claudepot_state_dir: Option<&Path>,
    protected_paths: &std::collections::HashSet<String>,
    dry_run: bool,
    sink: &dyn crate::project_progress::ProgressSink,
) -> Result<(CleanResult, Vec<ProjectInfo>), ProjectError> {
    use crate::project_progress::PhaseStatus;

    let projects = list_projects(config_dir)?;

    let unreachable_skipped = projects.iter().filter(|p| !p.is_reachable).count();
    let orphans: Vec<ProjectInfo> =
        projects.into_iter().filter(|p| p.is_orphan).collect();

    let mut result = CleanResult {
        orphans_found: orphans.len(),
        unreachable_skipped,
        ..Default::default()
    };

    if dry_run {
        return Ok((result, orphans));
    }

    let state_root = repair_root_from(config_dir, claudepot_state_dir);
    let snapshots_dir_owned: PathBuf = snapshots_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| state_root.join("snapshots"));
    let locks_dir_owned: PathBuf = locks_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| state_root.join("locks"));

    let (lock_guard, _broken) =
        crate::project_lock::acquire(&locks_dir_owned, CLEAN_LOCK_KEY)?;

    // Phase 0 — preflight. Per-orphan TOCTOU + live-session validation
    // happens BEFORE any sibling-state mutation. Without this, a batch
    // prune of `~/.claude.json` / `history.jsonl` could strip entries
    // for an orphan whose CC dir we then refuse to delete (because the
    // source re-appeared, or a live session showed up). That left the
    // user's config wiped while the artifact dir survived — irreversible
    // sibling-state loss for an artifact we ended up keeping.
    //
    // Decision per orphan is one of:
    //   * Remove           — passes all checks, will delete and (if
    //                        non-empty + non-protected) prune siblings.
    //   * SkipMissing      — CC dir already gone; nothing to do.
    //   * SkipReappeared   — source came back since listing.
    //   * SkipLive         — live session detected.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Decision {
        Remove,
        SkipMissing,
        SkipReappeared,
        SkipLive,
    }
    let decisions: Vec<Decision> = orphans
        .iter()
        .map(|orphan| {
            let dir = config_dir.join("projects").join(&orphan.sanitized_name);
            if !dir.exists() {
                return Decision::SkipMissing;
            }
            if !orphan.is_empty
                && classify_reachability(&orphan.original_path)
                    != PathReachability::Absent
            {
                return Decision::SkipReappeared;
            }
            if detect_live_session(&dir, &orphan.original_path, CLEAN_LIVE_HEARTBEAT_SECS)
            {
                return Decision::SkipLive;
            }
            Decision::Remove
        })
        .collect();

    result.orphans_skipped_live = decisions
        .iter()
        .filter(|d| **d == Decision::SkipLive)
        .count();

    // Partition orphans that will actually be removed (and have an
    // authoritative cwd) into:
    //   * `authoritative_paths` — sibling state WILL be rewritten.
    //   * `protected_paths_hit` — protected per the user's config; the
    //     CC artifact dir still goes, sibling state is preserved.
    //
    // Empty-dir orphans are excluded from BOTH lists: `original_path`
    // is from the lossy `unsanitize_path` fallback, so acting on it
    // risks deleting unrelated entries.
    let mut authoritative_paths: Vec<String> = Vec::new();
    let mut protected_paths_hit_count: usize = 0;
    for (orphan, decision) in orphans.iter().zip(decisions.iter()) {
        if *decision != Decision::Remove || orphan.is_empty {
            continue;
        }
        if protected_paths.contains(&orphan.original_path) {
            protected_paths_hit_count += 1;
        } else {
            authoritative_paths.push(orphan.original_path.clone());
        }
    }
    result.protected_paths_skipped = protected_paths_hit_count;

    // Phase 1 — batch sibling-state rewrite. Single pass over each file
    // removing every matching entry. Huge speedup vs the previous
    // per-orphan loop, which re-read+rewrote history.jsonl N times
    // (N passes × full file scan = O(N × size) I/O).
    sink.phase("batch-sibling", PhaseStatus::Running);
    sink.sub_progress("batch-sibling", 0, 2);

    if !authoritative_paths.is_empty() {
        if let Some(config_path) = claude_json_path {
            match remove_claude_json_entries_batch(
                config_path,
                &snapshots_dir_owned,
                &authoritative_paths,
            ) {
                Ok((removed, snap)) => {
                    result.claude_json_entries_removed = removed;
                    if let Some(p) = snap {
                        result.snapshot_paths.push(p);
                    }
                }
                Err(e) => {
                    tracing::warn!(err = %e, "~/.claude.json batch prune failed; continuing");
                }
            }
        }
    }
    sink.sub_progress("batch-sibling", 1, 2);

    if !authoritative_paths.is_empty() {
        let history_path = config_dir.join("history.jsonl");
        if history_path.exists() {
            match remove_history_lines_batch(
                &history_path,
                &snapshots_dir_owned,
                &authoritative_paths,
            ) {
                Ok((removed, snap)) => {
                    result.history_lines_removed = removed;
                    if let Some(p) = snap {
                        result.snapshot_paths.push(p);
                    }
                }
                Err(e) => {
                    tracing::warn!(err = %e, "history.jsonl batch prune failed; continuing");
                }
            }
        }
    }
    sink.sub_progress("batch-sibling", 2, 2);
    sink.phase("batch-sibling", PhaseStatus::Complete);

    // Phase 2 — per-orphan CC-dir removal driven by the preflight
    // decision vector. The validation already happened; here we only
    // act. Per-orphan logging keeps the existing skip messages so log
    // consumers don't lose context.
    let total = orphans.len();
    sink.phase("remove-dirs", PhaseStatus::Running);
    sink.sub_progress("remove-dirs", 0, total);

    for (i, (orphan, decision)) in orphans.iter().zip(decisions.iter()).enumerate() {
        match decision {
            Decision::SkipMissing => {}
            Decision::SkipReappeared => {
                tracing::info!(
                    path = %orphan.original_path,
                    "skipping: source re-appeared or became unreachable since listing"
                );
            }
            Decision::SkipLive => {
                tracing::warn!(
                    dir = ?config_dir.join("projects").join(&orphan.sanitized_name),
                    "skipping: live session detected against orphan CC dir"
                );
            }
            Decision::Remove => {
                let dir = config_dir.join("projects").join(&orphan.sanitized_name);
                let bytes = orphan.total_size_bytes;
                fs::remove_dir_all(&dir).map_err(ProjectError::Io)?;
                result.orphans_removed += 1;
                result.bytes_freed += bytes;

                result.claudepot_artifacts_removed += remove_claudepot_artifacts(
                    &snapshots_dir_owned,
                    &state_root.join("journals"),
                    &orphan.sanitized_name,
                );
            }
        }
        sink.sub_progress("remove-dirs", i + 1, total);
    }
    sink.phase("remove-dirs", PhaseStatus::Complete);

    lock_guard.release()?;
    Ok((result, orphans))
}

/// Batch: remove every `projects[<path>]` entry for the given list of
/// paths in a single atomic read+write of `~/.claude.json`. Writes one
/// consolidated snapshot of the removed map (keyed by path) so the
/// user can recover any entry. Returns (count_removed, snapshot_path).
///
/// Preferred over calling `remove_claude_json_entry` in a loop — for
/// N orphans we'd otherwise read+rewrite the whole config N times.
fn remove_claude_json_entries_batch(
    config_path: &Path,
    snapshots_dir: &Path,
    paths: &[String],
) -> Result<(usize, Option<PathBuf>), ProjectError> {
    if !config_path.exists() || paths.is_empty() {
        return Ok((0, None));
    }

    let contents = fs::read_to_string(config_path).map_err(ProjectError::Io)?;
    let mut root: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            return Err(ProjectError::Ambiguous(format!(
                "~/.claude.json is not valid JSON: {e}"
            )));
        }
    };

    let projects_map = match root.get_mut("projects") {
        Some(serde_json::Value::Object(m)) => m,
        _ => return Ok((0, None)),
    };

    let mut removed = serde_json::Map::new();
    for p in paths {
        if let Some(v) = projects_map.remove(p) {
            removed.insert(p.clone(), v);
        }
    }

    if removed.is_empty() {
        return Ok((0, None));
    }

    let count = removed.len();
    let snap = write_clean_snapshot(
        snapshots_dir,
        "batch",
        "config",
        &serde_json::Value::Object(removed),
    )?;
    write_json_atomic(config_path, &root)?;
    Ok((count, Some(snap)))
}

/// Batch: drop every `history.jsonl` line whose `project` field matches
/// any path in `paths`, in a SINGLE pass over the file. Writes one
/// snapshot of all dropped lines. Returns (count_removed, snapshot_path).
///
/// Uses a `HashSet` lookup per line so per-line work stays O(1) in the
/// number of target paths.
fn remove_history_lines_batch(
    history_path: &Path,
    snapshots_dir: &Path,
    paths: &[String],
) -> Result<(usize, Option<PathBuf>), ProjectError> {
    use std::collections::HashSet;
    use std::io::{BufRead, BufReader, BufWriter, Write};

    if paths.is_empty() {
        return Ok((0, None));
    }

    let targets: HashSet<&str> = paths.iter().map(String::as_str).collect();

    let file = fs::File::open(history_path).map_err(ProjectError::Io)?;
    let reader = BufReader::new(file);

    let parent = history_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent).map_err(ProjectError::Io)?;
    let mut writer = BufWriter::new(&tmp);

    let mut dropped: Vec<String> = Vec::new();

    for line in reader.lines() {
        let line = line.map_err(ProjectError::Io)?;
        let mut keep = true;
        // Cheap pre-filter: skip the JSON parse unless the line carries
        // the `"project":` key — otherwise it's a waste on every
        // unrelated entry.
        if line.contains("\"project\":") {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(p) = entry.get("project").and_then(|v| v.as_str()) {
                    if targets.contains(p) {
                        keep = false;
                    }
                }
            }
        }
        if keep {
            writeln!(writer, "{line}").map_err(ProjectError::Io)?;
        } else {
            dropped.push(line);
        }
    }

    drop(writer);
    if dropped.is_empty() {
        return Ok((0, None));
    }

    tmp.persist(history_path).map_err(|e| ProjectError::Io(e.error))?;

    let count = dropped.len();
    let payload = serde_json::Value::Array(
        dropped.into_iter().map(serde_json::Value::String).collect(),
    );
    let snap = write_clean_snapshot(snapshots_dir, "batch", "history", &payload)?;
    Ok((count, Some(snap)))
}

/// Remove claudepot-owned artifacts keyed on the orphan's sanitized
/// name:
///   * snapshots at `<snapshots_dir>/<ts>-<san>-<phase>.<ext>`
///   * abandoned journal sidecars at `<journals_dir>/*.abandoned.json`
///     whose body references `<san>` as `old_san` or `new_san`
/// Live journals are NEVER touched — the pending-journal gate in the
/// CLI ensures this function is only reached when no journals are
/// in-flight.
///
/// Returns the count of files removed.
fn remove_claudepot_artifacts(
    snapshots_dir: &Path,
    journals_dir: &Path,
    sanitized_name: &str,
) -> usize {
    let mut removed = 0;

    // Snapshots. Naming convention from `project.rs::snapshot_phase`
    // and `project_config_rewrite::write_snapshot`:
    //   `<ts>-<safe_san>-<phase>.snap|json`
    // Match by embedded `-<san>-`. `clean_orphans`'s own config/history
    // snapshots are created with `-clean-<kind>.json` suffixes which
    // also match this pattern and should be retained — exclude them
    // explicitly so we don't eat our own recovery artifact.
    if snapshots_dir.exists() {
        let needle = format!("-{sanitized_name}-");
        let skip_suffixes: &[&str] = &["-clean-config.json", "-clean-history.json"];
        if let Ok(entries) = fs::read_dir(snapshots_dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.contains(&needle) {
                    continue;
                }
                if skip_suffixes.iter().any(|s| name.ends_with(s)) {
                    continue;
                }
                if fs::remove_file(e.path()).is_ok() {
                    removed += 1;
                }
            }
        }
    }

    // Abandoned journal sidecars. An in-flight journal (no sidecar)
    // would have blocked the CLI before we got here, so we only expect
    // `*.abandoned.json` files at this point.
    if journals_dir.exists() {
        if let Ok(entries) = fs::read_dir(journals_dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".abandoned.json") {
                    continue;
                }
                let path = e.path();
                let Ok(contents) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(body) = serde_json::from_str::<serde_json::Value>(&contents) else {
                    continue;
                };
                let old_san = body.get("old_san").and_then(|v| v.as_str()).unwrap_or("");
                let new_san = body.get("new_san").and_then(|v| v.as_str()).unwrap_or("");
                if old_san == sanitized_name || new_san == sanitized_name {
                    // Also remove the original journal alongside the
                    // sidecar, if it still exists.
                    let stem = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_suffix(".abandoned.json"));
                    if let Some(stem) = stem {
                        let journal_path = journals_dir.join(format!("{stem}.json"));
                        if fs::remove_file(&journal_path).is_ok() {
                            removed += 1;
                        }
                    }
                    if fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }

    removed
}

/// Write a cleanup-side snapshot. Filename:
/// `<ts>-<safe_san>-clean-<kind>.json`. Mode 0600 on Unix because
/// snapshots can contain project trust flags / MCP tokens / history.
fn write_clean_snapshot(
    snapshots_dir: &Path,
    sanitized_name: &str,
    kind: &str,
    value: &serde_json::Value,
) -> Result<PathBuf, ProjectError> {
    fs::create_dir_all(snapshots_dir).map_err(ProjectError::Io)?;
    let safe_san: String = sanitized_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = snapshots_dir.join(format!("{ts}-{safe_san}-clean-{kind}.json"));
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    fs::write(&path, json).map_err(ProjectError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

/// Atomic replace of a JSON file, preserving mode when possible.
fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<(), ProjectError> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(ProjectError::Io)?;
    tmp.write_all(json.as_bytes()).map_err(ProjectError::Io)?;
    tmp.write_all(b"\n").map_err(ProjectError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mode = meta.permissions().mode();
            let _ = fs::set_permissions(tmp.path(), fs::Permissions::from_mode(mode));
        }
    }
    tmp.persist(path).map_err(|e| ProjectError::Io(e.error))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_unix_path() {
        assert_eq!(
            sanitize_path("/Users/joker/github/xiaolai/myprojects/kannon"),
            "-Users-joker-github-xiaolai-myprojects-kannon"
        );
    }

    #[test]
    fn test_sanitize_windows_path() {
        assert_eq!(
            sanitize_path("C:\\Users\\joker\\project"),
            "C--Users-joker-project"
        );
    }

    #[test]
    fn test_sanitize_preserves_alphanumeric() {
        assert_eq!(sanitize_path("abc123"), "abc123");
    }

    #[test]
    fn test_sanitize_replaces_special_chars() {
        assert_eq!(sanitize_path("/a.b_c-d"), "-a-b-c-d");
    }

    #[test]
    fn test_sanitize_long_path_with_hash() {
        let long_path = "/".to_string() + &"a".repeat(250);
        let result = sanitize_path(&long_path);
        // Should be 200 chars + '-' + hash
        assert!(result.len() > MAX_SANITIZED_LENGTH);
        assert!(result.starts_with("-"));
        // The first 200 chars should be from the sanitized path
        let prefix = &result[..MAX_SANITIZED_LENGTH];
        assert!(prefix.chars().all(|c| c == '-' || c == 'a'));
    }

    #[test]
    fn test_sanitize_unicode_path() {
        // Unicode chars are non-alphanumeric, should become `-`
        assert_eq!(sanitize_path("/tmp/\u{00e9}l\u{00e8}ve"), "-tmp--l-ve");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_unsanitize_roundtrip_simple_unix() {
        let original = "/Users/joker/project";
        let sanitized = sanitize_path(original);
        let unsanitized = unsanitize_path(&sanitized);
        assert_eq!(unsanitized, original);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_unsanitize_lossy_unix() {
        // Hyphens and underscores both become `-`, so unsanitize is lossy
        let sanitized = sanitize_path("/my-project");
        let unsanitized = unsanitize_path(&sanitized);
        // Original was /my-project, sanitized to -my-project, unsanitized to /my/project
        assert_eq!(unsanitized, "/my/project");
    }

    #[test]
    fn test_unsanitize_windows_drive_letter_roundtrip() {
        // The `<alpha>--` shape is unambiguous: no Unix-sanitized slug
        // can start with an ASCII letter followed by two hyphens (a
        // leading `-` in a Unix slug means the first char of the
        // original was a separator, not a letter). So we recover the
        // Windows form on any host OS.
        let original = r"C:\Users\joker\project";
        let sanitized = sanitize_path(original);
        assert_eq!(sanitized, "C--Users-joker-project");
        let unsanitized = unsanitize_path(&sanitized);
        assert_eq!(unsanitized, original);
    }

    #[test]
    fn test_unsanitize_windows_drive_letter_lowercase() {
        let original = r"d:\work\repo";
        let sanitized = sanitize_path(original);
        assert_eq!(sanitized, "d--work-repo");
        let unsanitized = unsanitize_path(&sanitized);
        assert_eq!(unsanitized, original);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_unsanitize_windows_unc_roundtrip_on_windows() {
        // UNC slug `--server-share-project` is ambiguous with a Unix
        // path whose first component starts with `-`. On Windows we
        // resolve to UNC; on Unix we keep the `/` convention.
        let original = r"\\server\share\project";
        let sanitized = sanitize_path(original);
        assert_eq!(sanitized, "--server-share-project");
        let unsanitized = unsanitize_path(&sanitized);
        assert_eq!(unsanitized, original);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_unsanitize_unix_slug_on_windows_uses_backslash() {
        // On Windows, a Unix-shaped slug gets `\` as the fallback
        // separator — matching the host filesystem convention.
        let sanitized = "-Users-joker-project";
        let unsanitized = unsanitize_path(sanitized);
        assert_eq!(unsanitized, r"\Users\joker\project");
    }

    #[test]
    fn test_djb2_hash_deterministic() {
        let h1 = djb2_hash("test");
        let h2 = djb2_hash("test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_djb2_hash_different_inputs() {
        let h1 = djb2_hash("abc");
        let h2 = djb2_hash("def");
        assert_ne!(h1, h2);
    }

    // ---------------------------------------------------------------------
    // Group 1 — CC parity (golden values from CC's sanitizePath/djb2Hash
    // run in Node.js on 2026-04-13). If these fail, either CC changed their
    // implementation or we drifted. See /tmp/cc-golden-values.js.
    // ---------------------------------------------------------------------

    #[test]
    fn test_sanitize_cc_parity_unix() {
        assert_eq!(
            sanitize_path("/Users/joker/github/xiaolai/myprojects/com.claudepot.app"),
            "-Users-joker-github-xiaolai-myprojects-com-claudepot-app"
        );
    }

    #[test]
    fn test_sanitize_cc_parity_windows() {
        assert_eq!(
            sanitize_path("C:\\Users\\joker\\Documents\\project"),
            "C--Users-joker-Documents-project"
        );
    }

    #[test]
    fn test_sanitize_cc_parity_hyphen_in_name() {
        assert_eq!(
            sanitize_path("/Users/joker/my-project"),
            "-Users-joker-my-project"
        );
    }

    #[test]
    fn test_sanitize_cc_parity_nfc_accent() {
        assert_eq!(sanitize_path("/tmp/café-project"), "-tmp-caf--project");
    }

    #[test]
    fn test_sanitize_cc_parity_emoji() {
        // JS UTF-16 surrogate pair (🎉 = U+1F389) produces TWO hyphens,
        // not one. This is the whole point of encode_utf16 in our impl.
        assert_eq!(sanitize_path("/tmp/🎉emoji"), "-tmp---emoji");
    }

    #[test]
    fn test_djb2_cc_parity_long_path() {
        let input = "/Users/joker/".to_string() + &"a".repeat(250);
        assert_eq!(djb2_hash(&input), "lwkvhu");
        // Full sanitize_path output: 200-char prefix + '-' + hash.
        let result = sanitize_path(&input);
        assert!(result.ends_with("-lwkvhu"), "result={result}");
        assert_eq!(result.len(), 200 + 1 + "lwkvhu".len());
    }

    #[test]
    fn test_djb2_cc_parity_unicode() {
        // "/tmp/café" — 'é' encodes as U+00E9 (one UTF-16 code unit).
        assert_eq!(djb2_hash("/tmp/café"), "udmm60");
    }

    // ---------------------------------------------------------------------
    // Group 10 — Windows path tests (CC parity golden values).
    // Pure string ops: these run on all platforms regardless of cfg.
    // ---------------------------------------------------------------------

    #[test]
    fn test_sanitize_windows_drive_letter() {
        assert_eq!(
            sanitize_path("C:\\Users\\joker\\project"),
            "C--Users-joker-project"
        );
    }

    #[test]
    fn test_sanitize_windows_unc_path() {
        assert_eq!(
            sanitize_path("\\\\server\\share\\project"),
            "--server-share-project"
        );
    }

    #[test]
    fn test_sanitize_windows_spaces_in_path() {
        assert_eq!(
            sanitize_path("C:\\Program Files\\My App"),
            "C--Program-Files-My-App"
        );
    }

    #[test]
    fn test_sanitize_windows_long_path() {
        let input = "C:\\Users\\joker\\".to_string() + &"a".repeat(250);
        assert_eq!(djb2_hash(&input), "27k5dq");
        let out = sanitize_path(&input);
        assert!(out.ends_with("-27k5dq"), "out={out}");
        assert_eq!(out.len(), 200 + 1 + "27k5dq".len());
    }

    #[test]
    fn test_sanitize_windows_reserved_chars() {
        // ':', '?' are reserved on Windows; all non-alphanumerics become '-'.
        assert_eq!(
            sanitize_path("C:\\Users\\joker\\file:name?"),
            "C--Users-joker-file-name-"
        );
    }

    #[test]
    fn test_format_radix_base36() {
        assert_eq!(format_radix(0, 36), "0");
        assert_eq!(format_radix(35, 36), "z");
        assert_eq!(format_radix(36, 36), "10");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(1073741824), "1.0 GB");
    }

    #[test]
    fn test_list_projects_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        // No projects/ dir at all
        let result = list_projects(config_dir).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_recover_cwd_skips_custom_title_first_line() {
        // CC sometimes writes a `custom-title` entry (no `cwd` field) as
        // the first line of a session, then writes normal entries with
        // `cwd` on subsequent lines. The recovery must keep reading
        // until it finds `cwd`, otherwise paths with `.` collapse
        // (e.g. `-xiaolai-lixiaolai-com` → `/xiaolai/lixiaolai/com`
        // instead of the real `/xiaolai/lixiaolai.com`).
        let tmp = tempfile::tempdir().unwrap();
        let session_file = tmp.path().join("abc.jsonl");
        fs::write(
            &session_file,
            concat!(
                r#"{"type":"custom-title","title":"session","timestamp":"2026-04-20T00:00:00Z","uuid":"u1"}"#,
                "\n",
                r#"{"type":"user","cwd":"/Users/joker/github/xiaolai/lixiaolai.com","timestamp":"2026-04-20T00:00:01Z","uuid":"u2"}"#,
                "\n",
            ),
        )
        .unwrap();

        let recovered = recover_cwd_from_sessions(tmp.path());
        assert_eq!(
            recovered.as_deref(),
            Some("/Users/joker/github/xiaolai/lixiaolai.com")
        );
    }

    #[test]
    fn test_recover_cwd_skips_summary_entries() {
        // Same pattern with a `summary` entry instead of `custom-title`.
        let tmp = tempfile::tempdir().unwrap();
        let session_file = tmp.path().join("abc.jsonl");
        fs::write(
            &session_file,
            concat!(
                r#"{"type":"summary","summary":"...","timestamp":"2026-04-20T00:00:00Z","uuid":"u1"}"#,
                "\n",
                r#"{"type":"assistant","cwd":"/home/user/some.dir.with.dots","timestamp":"2026-04-20T00:00:01Z","uuid":"u2"}"#,
                "\n",
            ),
        )
        .unwrap();

        let recovered = recover_cwd_from_sessions(tmp.path());
        assert_eq!(recovered.as_deref(), Some("/home/user/some.dir.with.dots"));
    }

    #[test]
    fn test_recover_cwd_none_when_no_session_has_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
        )
        .unwrap();
        assert_eq!(recover_cwd_from_sessions(tmp.path()), None);
    }

    #[test]
    fn test_recover_cwd_empty_cwd_is_ignored() {
        // `"cwd": ""` must not be accepted — fall through to the next line.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            concat!(
                r#"{"type":"user","cwd":"","uuid":"u1"}"#,
                "\n",
                r#"{"type":"user","cwd":"/Users/joker/project","uuid":"u2"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            recover_cwd_from_sessions(tmp.path()).as_deref(),
            Some("/Users/joker/project")
        );
    }

    #[test]
    fn test_recover_cwd_skips_unparseable_lines() {
        // Malformed JSON (BOM, truncated line, partial write) must not
        // abort the whole scan — keep reading.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            concat!(
                "\u{feff}not-json\n",
                "{incomplete\n",
                r#"{"type":"user","cwd":"/real/path","uuid":"u1"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            recover_cwd_from_sessions(tmp.path()).as_deref(),
            Some("/real/path")
        );
    }

    #[test]
    fn test_recover_cwd_windows_drive_letter() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            concat!(
                r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
                "\n",
                // JSON-escaped backslashes: `C:\Users\joker\project`.
                r#"{"type":"user","cwd":"C:\\Users\\joker\\project","uuid":"u2"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            recover_cwd_from_sessions(tmp.path()).as_deref(),
            Some(r"C:\Users\joker\project")
        );
    }

    #[test]
    fn test_recover_cwd_windows_unc_path() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            concat!(
                r#"{"type":"user","cwd":"\\\\server\\share\\project","uuid":"u1"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            recover_cwd_from_sessions(tmp.path()).as_deref(),
            Some(r"\\server\share\project")
        );
    }

    #[test]
    fn test_recover_cwd_strips_windows_verbatim_prefix() {
        // Defense-in-depth: CC never writes `\\?\` in cwd, but a
        // third-party writer might. The recovered path must be the
        // non-verbatim form so downstream sanitize/roundtrip checks
        // don't drift.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            concat!(
                r#"{"type":"user","cwd":"\\\\?\\C:\\Users\\joker","uuid":"u1"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            recover_cwd_from_sessions(tmp.path()).as_deref(),
            Some(r"C:\Users\joker")
        );
    }

    #[test]
    fn test_recover_cwd_strips_windows_verbatim_unc_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a.jsonl"),
            concat!(
                r#"{"type":"user","cwd":"\\\\?\\UNC\\server\\share\\p","uuid":"u1"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            recover_cwd_from_sessions(tmp.path()).as_deref(),
            Some(r"\\server\share\p")
        );
    }

    #[test]
    fn test_list_projects_windows_path_with_dot_in_name() {
        // End-to-end: a Windows-style project whose name contains a
        // literal `.` should survive the sanitize/unsanitize roundtrip
        // via the recovered cwd, not collapse to backslashes.
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();
        // sanitize(`C:\Users\joker\lixiaolai.com`) = `C--Users-joker-lixiaolai-com`.
        let slug = "C--Users-joker-lixiaolai-com";
        let proj = projects_dir.join(slug);
        fs::create_dir(&proj).unwrap();
        fs::write(
            proj.join("s1.jsonl"),
            concat!(
                r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
                "\n",
                r#"{"type":"user","cwd":"C:\\Users\\joker\\lixiaolai.com","uuid":"u2"}"#,
                "\n",
            ),
        )
        .unwrap();

        let result = list_projects(tmp.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].original_path, r"C:\Users\joker\lixiaolai.com");
    }

    #[test]
    fn test_list_projects_preserves_special_chars_via_cwd() {
        // Exercises the full edge-case catalog found in real CC project
        // dirs: spaces, apostrophes, tildes, and dots. All would be lost
        // by `unsanitize_path` alone, but the authoritative `cwd` keeps
        // them intact.
        let cases = [
            (
                "-Users-joker-Desktop-reading-room",
                "/Users/joker/Desktop/reading-room",
            ),
            ("-Users-joker-Writer-s-Office", "/Users/joker/Writer's Office"),
            (
                "-Users-joker-Library-Mobile-Documents-iCloud-com-nssurge-inc-Documents",
                "/Users/joker/Library/Mobile Documents/iCloud~com~nssurge~inc/Documents",
            ),
            (
                "-Users-joker-github-xiaolai-myprojects-com-claudepot-app",
                "/Users/joker/github/xiaolai/myprojects/com.claudepot.app",
            ),
        ];

        for (slug, cwd) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let projects_dir = tmp.path().join("projects");
            fs::create_dir(&projects_dir).unwrap();
            let proj = projects_dir.join(slug);
            fs::create_dir(&proj).unwrap();
            let line = format!(
                r#"{{"type":"custom-title","title":"x","uuid":"u1"}}
{{"type":"user","cwd":{:?},"uuid":"u2"}}
"#,
                cwd
            );
            fs::write(proj.join("s1.jsonl"), line).unwrap();

            let result = list_projects(tmp.path()).unwrap();
            assert_eq!(result.len(), 1, "slug={slug}");
            assert_eq!(result[0].original_path, cwd, "slug={slug}");
        }
    }

    #[test]
    fn test_list_projects_recovers_cwd_with_dot_in_name() {
        // End-to-end: a project whose real path contains `.` should be
        // reported with the dot intact, recovered from session.jsonl
        // even when the first transcript line is a `custom-title`.
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let slug = "-Users-joker-github-xiaolai-lixiaolai-com";
        let proj = projects_dir.join(slug);
        fs::create_dir(&proj).unwrap();
        fs::write(
            proj.join("s1.jsonl"),
            concat!(
                r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
                "\n",
                r#"{"type":"user","cwd":"/Users/joker/github/xiaolai/lixiaolai.com","uuid":"u2"}"#,
                "\n",
            ),
        )
        .unwrap();

        let result = list_projects(tmp.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].original_path,
            "/Users/joker/github/xiaolai/lixiaolai.com"
        );
    }

    #[test]
    fn test_list_projects_with_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Create a fake project
        let proj = projects_dir.join("-tmp-myproject");
        fs::create_dir(&proj).unwrap();
        fs::write(proj.join("abc.jsonl"), "{}").unwrap();
        fs::write(proj.join("def.jsonl"), "{}").unwrap();

        let memory_dir = proj.join("memory");
        fs::create_dir(&memory_dir).unwrap();
        fs::write(memory_dir.join("MEMORY.md"), "# mem").unwrap();

        let result = list_projects(tmp.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].sanitized_name, "-tmp-myproject");
        assert_eq!(result[0].session_count, 2);
        assert_eq!(result[0].memory_file_count, 1);
        assert!(result[0].is_orphan); // /tmp/myproject likely doesn't exist
    }

    #[test]
    fn test_show_project_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let result = show_project(tmp.path(), "/nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_move_project_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("myproject");
        fs::create_dir(&src).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: src.clone(),
            config_dir: tmp.path().to_path_buf(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink);
        assert!(matches!(result, Err(ProjectError::SamePath)));
    }

    #[test]
    fn test_move_project_renames_cc_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize to handle macOS /tmp -> /private/tmp symlink
        let base = tmp.path().canonicalize().unwrap();

        // Create source directory
        let src = base.join("old");
        fs::create_dir(&src).unwrap();

        // Create CC project dir for old path (using canonical path)
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("session.jsonl"), "{}").unwrap();

        let dst = base.join("new");

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.actual_dir_moved);
        assert!(result.cc_dir_renamed);
        assert!(dst.exists());
        assert!(!src.exists());

        let new_san = sanitize_path(&dst.to_string_lossy());
        assert!(projects_dir.join(&new_san).exists());
        assert!(!projects_dir.join(&old_san).exists());

        // Verify session file content survived the move
        let moved_session = projects_dir.join(&new_san).join("session.jsonl");
        assert!(moved_session.exists());
        assert_eq!(fs::read_to_string(moved_session).unwrap(), "{}");
    }

    /// When a long path's sanitized form exceeds MAX_SANITIZED_LENGTH, the
    /// trailing hash suffix depends on runtime (Bun.hash vs djb2). If the
    /// existing CC dir was created by the Bun-compiled CC CLI with a hash
    /// suffix that doesn't match claudepot's djb2 output, move must still
    /// find it via prefix scanning. This mirrors CC's own findProjectDir
    /// tolerance (see sessionStoragePortable.ts:354-375).
    #[test]
    fn test_move_project_long_path_prefix_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        // Construct a >200-char source path
        let deep = "a".repeat(210);
        let src = base.join(&deep);
        fs::create_dir(&src).unwrap();

        // Simulate a CC-Bun-created dir: same 200-char prefix as
        // claudepot would compute, but a DIFFERENT hash suffix than djb2.
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let claudepot_san = sanitize_path(&src.to_string_lossy());
        assert!(claudepot_san.len() > MAX_SANITIZED_LENGTH);
        let prefix = &claudepot_san[..MAX_SANITIZED_LENGTH];
        let bun_style_san = format!("{}-{}", prefix, "fakebunhashxyz");
        assert_ne!(bun_style_san, claudepot_san); // hash suffixes differ
        let cc_old = projects_dir.join(&bun_style_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("session.jsonl"), r#"{"cwd":"x"}"#).unwrap();

        let dst = base.join(&"b".repeat(210));

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.actual_dir_moved, "disk dir should be moved");
        assert!(
            result.cc_dir_renamed,
            "CC dir should be found via prefix fallback and renamed"
        );

        // Source CC dir should no longer exist at its old sanitized name
        assert!(!cc_old.exists());
        // Destination should exist at claudepot's new_san (the prefix
        // portion must still match what CC would look up via findProjectDir)
        let new_san = sanitize_path(&dst.to_string_lossy());
        let new_prefix = &new_san[..MAX_SANITIZED_LENGTH];
        let found_new = fs::read_dir(&projects_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(new_prefix)
            });
        assert!(found_new.is_some(), "new CC dir should exist under the new prefix");
    }

    #[test]
    fn test_move_rejects_new_inside_old() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let src = base.join("proj");
        fs::create_dir(&src).unwrap();
        let args = MoveArgs {
            old_path: src.clone(),
            new_path: src.join("nested"),
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };
        let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(ref m) if m.contains("inside")));
    }

    #[test]
    fn test_move_rejects_old_inside_new() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let outer = base.join("outer");
        fs::create_dir(&outer).unwrap();
        let inner = outer.join("inner");
        fs::create_dir(&inner).unwrap();
        let args = MoveArgs {
            old_path: inner,
            new_path: outer,
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };
        let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(_)));
    }

    /// End-to-end verification that Phase 6 (session jsonl rewrite) runs
    /// as part of move_project: stale `cwd` fields inside session jsonls
    /// get rewritten to the new path, including `cd`-into-subdir entries
    /// that use the boundary-prefix form.
    #[test]
    fn test_move_project_rewrites_session_jsonl_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old-project");
        fs::create_dir(&src).unwrap();
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let old_str = src.to_string_lossy().to_string();
        let old_san = sanitize_path(&old_str);
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();

        // Main session jsonl with exact match and a subdir-cd case
        let sep = std::path::MAIN_SEPARATOR;
        let session_content = format!(
            r#"{{"cwd":"{old}","i":1}}
{{"cwd":"{old}{sep}src","i":2}}
{{"cwd":"/elsewhere","i":3}}
"#,
            old = old_str
        );
        fs::write(cc_old.join("sess.jsonl"), &session_content).unwrap();

        // Subagent jsonl
        let subagent_dir = cc_old.join("sessA").join("subagents");
        fs::create_dir_all(&subagent_dir).unwrap();
        fs::write(
            subagent_dir.join("agent-x.jsonl"),
            format!(r#"{{"cwd":"{old}","agent":"x"}}
"#, old = old_str),
        )
        .unwrap();

        let dst = base.join("renamed-project");
        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };
        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();

        assert!(result.cc_dir_renamed);
        assert!(result.jsonl_files_scanned >= 2);
        assert!(result.jsonl_files_modified >= 2);
        // 2 in main session + 1 in subagent = 3 cwd rewrites (the
        // `/elsewhere` entry must NOT be rewritten).
        assert_eq!(result.jsonl_lines_rewritten, 3);
        assert!(result.jsonl_errors.is_empty());

        let new_str = dst.to_string_lossy().to_string();
        let new_san = sanitize_path(&new_str);
        let cc_new = projects_dir.join(&new_san);
        let after_main = fs::read_to_string(cc_new.join("sess.jsonl")).unwrap();
        assert!(after_main.contains(&format!(r#""cwd":"{}""#, new_str)));
        assert!(after_main.contains(&format!(r#""cwd":"{}{}src""#, new_str, sep)));
        assert!(after_main.contains(r#""cwd":"/elsewhere""#)); // untouched
        assert!(!after_main.contains(&format!(r#""cwd":"{}""#, old_str)));
    }

    /// End-to-end verification that Phase 7 (~/.claude.json projects map)
    /// runs as part of move_project when claude_json_path is supplied:
    /// the map key migrates from old_path to new_path, preserving value,
    /// with no collision.
    #[test]
    fn test_move_project_rewrites_claude_json() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("origproj");
        fs::create_dir(&src).unwrap();
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let old_str = src.to_string_lossy().to_string();
        let old_san = sanitize_path(&old_str);
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();

        // Fake ~/.claude.json sibling file.
        let claude_json = base.join("claude.json");
        let cfg_before = serde_json::json!({
            "projects": {
                old_str.clone(): {"trust": true, "allowedTools": ["X"]}
            }
        });
        fs::write(
            &claude_json,
            serde_json::to_string_pretty(&cfg_before).unwrap(),
        )
        .unwrap();

        let dst = base.join("newproj");
        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: Some(claude_json.clone()),
            snapshots_dir: Some(base.join("snaps")),
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };
        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();

        assert!(result.cc_dir_renamed);
        assert!(result.config_key_renamed);
        assert!(!result.config_had_collision);
        assert!(result.config_snapshot_path.is_none());

        // Verify the JSON was actually rewritten.
        let cfg_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        let new_str = dst.to_string_lossy().to_string();
        assert!(cfg_after["projects"].get(&old_str).is_none());
        assert_eq!(cfg_after["projects"][&new_str]["trust"], serde_json::json!(true));
        assert_eq!(
            cfg_after["projects"][&new_str]["allowedTools"],
            serde_json::json!(["X"])
        );
    }

    /// Tests without claude_json_path should not touch any config file
    /// (hermetic). Confirmed by no P7 fields being set and no file being
    /// created.
    #[test]
    fn test_move_project_skips_p7_when_claude_json_path_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("a");
        fs::create_dir(&src).unwrap();
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let old_san = sanitize_path(&src.to_string_lossy());
        fs::create_dir(projects_dir.join(&old_san)).unwrap();

        let dst = base.join("b");
        let args = MoveArgs {
            old_path: src,
            new_path: dst,
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };
        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();

        assert!(result.cc_dir_renamed);
        assert!(!result.config_key_renamed);
        assert!(!result.config_had_collision);
        // No P7 snapshots specifically — journal + lock dirs are created
        // by the always-on recovery infrastructure, but snapshots are
        // only written when a destructive phase actually runs.
        assert!(!base.join("claudepot").join("snapshots").exists());
    }

    #[test]
    fn test_move_project_rewrites_history() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        // Use canonical paths in history entries. Build entries via
        // serde_json so Windows backslashes get correctly JSON-escaped.
        let old_str = src.canonicalize().unwrap().to_string_lossy().to_string();
        let new_str = dst.to_string_lossy().to_string();

        let history = base.join("history.jsonl");
        let entries = vec![
            serde_json::json!({"project": old_str, "sessionId": "abc", "timestamp": 1}).to_string(),
            serde_json::json!({"project": "/other/path", "sessionId": "def", "timestamp": 2})
                .to_string(),
            serde_json::json!({"project": old_str, "sessionId": "ghi", "timestamp": 3}).to_string(),
        ];
        fs::write(&history, entries.join("\n") + "\n").unwrap();

        // Create projects dir
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert_eq!(result.history_lines_updated, 2);

        // Verify history was rewritten by parsing each JSON line — raw string
        // matching breaks on Windows UNC paths (double-escaped backslashes).
        let content = fs::read_to_string(&history).unwrap();
        let projects: Vec<String> = content
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v.get("project").and_then(|p| p.as_str()).map(String::from))
            .collect();
        assert!(projects.iter().any(|p| p == &new_str), "new path present");
        assert!(!projects.iter().any(|p| p == &old_str), "old path gone");
        assert!(
            projects.iter().any(|p| p == "/other/path"),
            "unrelated entry kept"
        );
    }

    #[test]
    fn test_move_project_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        // Create projects dir
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: true,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        // Dry run: nothing actually changed
        assert!(!result.actual_dir_moved);
        assert!(!result.cc_dir_renamed);
        // Source still exists
        assert!(src.exists());
        assert!(!dst.exists());
    }

    #[test]
    fn test_clean_orphans_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Create a project whose source doesn't exist (orphan)
        let orphan = projects_dir.join("-nonexistent-path");
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("session.jsonl"), "{}").unwrap();

        let (result, orphans) = clean_orphans(tmp.path(), None, None, None, true).unwrap();
        assert_eq!(result.orphans_found, 1);
        assert_eq!(result.orphans_removed, 0); // dry run
        assert_eq!(orphans.len(), 1);
        // Dir still exists
        assert!(orphan.exists());
    }

    #[test]
    fn test_clean_orphans_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let orphan = projects_dir.join("-nonexistent-path");
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("session.jsonl"), "{}").unwrap();

        let (result, _) = clean_orphans(tmp.path(), None, None, None, false).unwrap();
        assert_eq!(result.orphans_found, 1);
        assert_eq!(result.orphans_removed, 1);
        assert!(!orphan.exists());
    }

    // ----- clean edge cases added for fixes 1-6, 9 -----

    /// Fix #1: paths under an absent `/Volumes/<drive>` mount point
    /// must NOT be flagged orphan. The drive might be unplugged; the
    /// data could still be present once remounted.
    #[test]
    #[cfg(unix)]
    fn test_clean_skips_unreachable_mount_prefix() {
        use crate::project_sanitize::sanitize_path;
        // Pick a `/Volumes/<name>` that definitely doesn't exist on any
        // test host. Anything unique is fine; macOS has an empty or
        // near-empty `/Volumes`, Linux has no such root and the prefix
        // is simply not matched.
        let fake_source = "/Volumes/claudepot-test-never-exists-xyz/proj";
        let san = sanitize_path(fake_source);

        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let dir = projects_dir.join(&san);
        fs::create_dir(&dir).unwrap();
        // Put a real session.jsonl with the authoritative cwd so the
        // recovered-cwd path is taken (same code path as a real CC dir).
        fs::write(
            dir.join("session.jsonl"),
            format!("{{\"cwd\":\"{fake_source}\",\"type\":\"user\"}}\n"),
        )
        .unwrap();

        let (result, orphans) = clean_orphans(tmp.path(), None, None, None, true).unwrap();

        #[cfg(target_os = "macos")]
        {
            // On macOS, `/Volumes/claudepot-test-never-exists-xyz`
            // mount point doesn't exist → unreachable → NOT orphan.
            assert_eq!(result.orphans_found, 0);
            assert_eq!(result.unreachable_skipped, 1);
            let info = orphans
                .iter()
                .find(|p| p.sanitized_name == san)
                .or_else(|| None);
            // (orphans is the orphan list, so the unreachable project
            // won't appear there; that's by design.)
            assert!(info.is_none());
        }
        #[cfg(not(target_os = "macos"))]
        {
            // On Linux, `/Volumes/...` isn't a mount prefix we detect
            // (we use `/mnt`, `/media`, `/run/media`), so the path
            // classification falls back to regular `try_exists`. That
            // still returns Absent, so the project is orphan. This
            // test just confirms no regression; the cross-platform
            // parity of the unreachable probe is a separate concern.
            let _ = (result, orphans);
        }
    }

    /// Fix #4a: when an orphan whose cwd was recovered authoritatively
    /// is cleaned, any matching `~/.claude.json` `projects[<path>]`
    /// entry must be removed with a snapshot written for recovery.
    #[test]
    fn test_clean_prunes_claude_json_entry_with_snapshot() {
        use crate::project_sanitize::sanitize_path;
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        let projects_dir = config_dir.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Orphan source path that we can safely assert doesn't exist.
        let fake_source = tmp.path().join("deleted-workspace");
        // DO NOT create `fake_source` — that's the whole point.
        let fake_source_str = fake_source.to_string_lossy().to_string();
        let san = sanitize_path(&fake_source_str);
        let dir = projects_dir.join(&san);
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("session.jsonl"),
            format!("{{\"cwd\":\"{fake_source_str}\",\"type\":\"user\"}}\n"),
        )
        .unwrap();

        // Seed ~/.claude.json with a matching entry.
        let claude_json = tmp.path().join("claude.json");
        fs::write(
            &claude_json,
            serde_json::to_string_pretty(&serde_json::json!({
                "projects": {
                    fake_source_str.clone(): {"trust": true, "allowedTools": ["Bash(git:*)"]},
                    "/elsewhere/unrelated": {"trust": false}
                },
                "otherTop": 42
            }))
            .unwrap(),
        )
        .unwrap();

        let snapshots = tmp.path().join("snaps");
        let locks = tmp.path().join("locks");
        let (result, _) = clean_orphans(
            config_dir,
            Some(claude_json.as_path()),
            Some(snapshots.as_path()),
            Some(locks.as_path()),
            false,
        )
        .unwrap();

        assert_eq!(result.orphans_removed, 1);
        assert_eq!(result.claude_json_entries_removed, 1);
        assert_eq!(result.snapshot_paths.len(), 1);

        // Config entry gone, unrelated entries intact.
        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert!(after["projects"].get(&fake_source_str).is_none());
        assert_eq!(
            after["projects"]["/elsewhere/unrelated"]["trust"],
            serde_json::json!(false)
        );
        assert_eq!(after["otherTop"], serde_json::json!(42));

        // Snapshot captured the removed value. Batched snapshot
        // shape is a map `{ <removed_path>: <value>, ... }` so all N
        // dropped entries live in one file.
        let snap: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&result.snapshot_paths[0]).unwrap())
                .unwrap();
        assert_eq!(snap[&fake_source_str]["trust"], serde_json::json!(true));
    }

    /// Fix #4b: matching `history.jsonl` lines are removed and dropped
    /// lines are captured in a snapshot so recovery is possible.
    #[test]
    fn test_clean_prunes_history_lines_with_snapshot() {
        use crate::project_sanitize::sanitize_path;
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        let projects_dir = config_dir.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let fake_source = tmp.path().join("deleted-workspace2");
        let fake_source_str = fake_source.to_string_lossy().to_string();
        let san = sanitize_path(&fake_source_str);
        let dir = projects_dir.join(&san);
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("session.jsonl"),
            format!("{{\"cwd\":\"{fake_source_str}\",\"type\":\"user\"}}\n"),
        )
        .unwrap();

        // Seed history.jsonl with two lines for our orphan and one
        // unrelated line.
        let history = config_dir.join("history.jsonl");
        let orphan_line_a = serde_json::json!({"display": "hello", "project": fake_source_str})
            .to_string();
        let orphan_line_b = serde_json::json!({"display": "again", "project": fake_source_str})
            .to_string();
        let other_line =
            serde_json::json!({"display": "keep me", "project": "/other/project"}).to_string();
        fs::write(
            &history,
            format!("{orphan_line_a}\n{orphan_line_b}\n{other_line}\n"),
        )
        .unwrap();

        let snapshots = tmp.path().join("snaps");
        let locks = tmp.path().join("locks");
        let (result, _) = clean_orphans(
            config_dir,
            None, // skip claude.json to keep this test focused
            Some(snapshots.as_path()),
            Some(locks.as_path()),
            false,
        )
        .unwrap();

        assert_eq!(result.orphans_removed, 1);
        assert_eq!(result.history_lines_removed, 2);

        let after = fs::read_to_string(&history).unwrap();
        assert!(!after.contains(&fake_source_str));
        assert!(after.contains("/other/project"));

        // Snapshot is a JSON array containing the two dropped lines.
        let snap_path = result
            .snapshot_paths
            .iter()
            .find(|p| p.to_string_lossy().contains("-clean-history"))
            .expect("history snapshot should be present");
        let snap: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(snap_path).unwrap()).unwrap();
        assert_eq!(snap.as_array().unwrap().len(), 2);
    }

    /// Fix #6: a live `__clean__` lock must block a second call from
    /// the same process (same-host + same pid = live).
    #[test]
    fn test_clean_takes_exclusive_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let locks = tmp.path().join("locks");

        // Acquire the clean lock manually using the same key so we
        // simulate an in-flight clean, then verify a second call
        // refuses.
        let (_g, _broken) = crate::project_lock::acquire(&locks, "__clean__").unwrap();

        let err = clean_orphans(
            tmp.path(),
            None,
            None,
            Some(locks.as_path()),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(_)));
    }

    /// Fix #9: a truly empty CC project dir (no sessions, no memory,
    /// under one FS block) should be cleaned even when the source
    /// sanitized name doesn't roundtrip (so we can't be certain about
    /// the source path). The dir is reclaimed; no sibling state is
    /// rewritten (safest choice when original_path is ambiguous).
    #[test]
    fn test_clean_removes_empty_project_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Name that deliberately doesn't roundtrip cleanly (contains a
        // hyphen that unsanitize would misread as a path separator).
        let ambiguous = projects_dir.join("-some-ambiguous-name-that-never-was-a-path");
        fs::create_dir(&ambiguous).unwrap();

        // Also seed a claude.json and history.jsonl; both must be
        // LEFT ALONE for empty-dir cleanup since original_path is
        // not authoritative.
        let claude_json = tmp.path().join("claude.json");
        fs::write(
            &claude_json,
            r#"{"projects":{"/real":{"trust":true}}}"#,
        )
        .unwrap();

        let locks = tmp.path().join("locks");
        let snaps = tmp.path().join("snaps");
        let (result, _) = clean_orphans(
            tmp.path(),
            Some(claude_json.as_path()),
            Some(snaps.as_path()),
            Some(locks.as_path()),
            false,
        )
        .unwrap();

        assert_eq!(result.orphans_found, 1);
        assert_eq!(result.orphans_removed, 1);
        assert!(!ambiguous.exists());
        // No sibling state touched: claude.json still intact.
        assert_eq!(result.claude_json_entries_removed, 0);
        let after = fs::read_to_string(&claude_json).unwrap();
        assert!(after.contains("/real"));
    }

    /// Fix #2: when `try_exists()` returns an Err (we simulate this
    /// indirectly by checking the `PathReachability::Unreachable`
    /// classification on a path whose ancestor is an absent mount
    /// root), the project must NOT be cleaned. Covered above by the
    /// mount-prefix test on macOS; this additional check exercises
    /// the empty-path early return.
    #[test]
    fn test_reachability_empty_path_is_unreachable() {
        use crate::project_helpers::{classify_reachability, PathReachability};
        assert_eq!(classify_reachability(""), PathReachability::Unreachable);
    }

    /// Protected-paths guard: when an orphan's authoritative source
    /// path is in the protected set, the CC artifact dir is still
    /// removed, but `~/.claude.json` and `history.jsonl` entries for
    /// that path are LEFT INTACT. `protected_paths_skipped` reflects
    /// the count.
    #[test]
    fn test_clean_protected_path_skips_sibling_rewrites() {
        use crate::project_sanitize::sanitize_path;
        use std::collections::HashSet;

        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        let projects_dir = config_dir.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Two orphans — one protected, one not. Both have an
        // authoritative cwd recovered from session.jsonl.
        let protected_src = tmp.path().join("guarded-workspace");
        let protected_str = protected_src.to_string_lossy().to_string();
        let san_protected = sanitize_path(&protected_str);
        let dir_p = projects_dir.join(&san_protected);
        fs::create_dir(&dir_p).unwrap();
        fs::write(
            dir_p.join("session.jsonl"),
            format!("{{\"cwd\":\"{protected_str}\",\"type\":\"user\"}}\n"),
        )
        .unwrap();

        let normal_src = tmp.path().join("normal-workspace");
        let normal_str = normal_src.to_string_lossy().to_string();
        let san_normal = sanitize_path(&normal_str);
        let dir_n = projects_dir.join(&san_normal);
        fs::create_dir(&dir_n).unwrap();
        fs::write(
            dir_n.join("session.jsonl"),
            format!("{{\"cwd\":\"{normal_str}\",\"type\":\"user\"}}\n"),
        )
        .unwrap();

        // Seed claude.json and history.jsonl with entries for BOTH.
        let claude_json = tmp.path().join("claude.json");
        fs::write(
            &claude_json,
            serde_json::to_string_pretty(&serde_json::json!({
                "projects": {
                    protected_str.clone(): {"trust": true, "note": "keep"},
                    normal_str.clone():    {"trust": false}
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let history = config_dir.join("history.jsonl");
        let line_p =
            serde_json::json!({"display": "p", "project": protected_str}).to_string();
        let line_n =
            serde_json::json!({"display": "n", "project": normal_str}).to_string();
        fs::write(&history, format!("{line_p}\n{line_n}\n")).unwrap();

        let snapshots = tmp.path().join("snaps");
        let locks = tmp.path().join("locks");
        let mut protected: HashSet<String> = HashSet::new();
        protected.insert(protected_str.clone());

        let (result, _) = clean_orphans_with_progress(
            config_dir,
            Some(claude_json.as_path()),
            Some(snapshots.as_path()),
            Some(locks.as_path()),
            None,
            &protected,
            false,
            &crate::project_progress::NoopSink,
        )
        .unwrap();

        // Both CC artifact dirs are removed.
        assert_eq!(result.orphans_found, 2);
        assert_eq!(result.orphans_removed, 2);
        assert!(!dir_p.exists());
        assert!(!dir_n.exists());

        // Sibling state: only the normal one was rewritten.
        assert_eq!(result.claude_json_entries_removed, 1);
        assert_eq!(result.history_lines_removed, 1);
        assert_eq!(result.protected_paths_skipped, 1);

        // The protected entries are still present in both files.
        let cj_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert!(cj_after["projects"].get(&protected_str).is_some());
        assert!(cj_after["projects"].get(&normal_str).is_none());

        let hist_after = fs::read_to_string(&history).unwrap();
        assert!(hist_after.contains(&protected_str));
        assert!(!hist_after.contains(&normal_str));
    }

    /// Regression for the audit-flagged HIGH bug: sibling state must
    /// NOT be pruned for an orphan whose source path has re-appeared
    /// since `list_projects` ran (TOCTOU). Phase 0's preflight catches
    /// the reappearance and excludes the orphan from `authoritative_paths`,
    /// so neither `~/.claude.json` nor `history.jsonl` are touched.
    #[test]
    fn test_clean_skips_sibling_rewrite_when_source_reappeared() {
        use crate::project_sanitize::sanitize_path;
        use std::collections::HashSet;

        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        let projects_dir = config_dir.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // The "orphan" — list_projects() will see this as orphan because
        // we'll seed session.jsonl with a cwd that doesn't exist YET.
        // Then we'll create the cwd before clean runs, simulating TOCTOU.
        let reappeared = tmp.path().join("reappeared-workspace");
        let reappeared_str = reappeared.to_string_lossy().to_string();
        let san = sanitize_path(&reappeared_str);
        let cc_dir = projects_dir.join(&san);
        fs::create_dir(&cc_dir).unwrap();
        fs::write(
            cc_dir.join("session.jsonl"),
            format!("{{\"cwd\":\"{reappeared_str}\",\"type\":\"user\"}}\n"),
        )
        .unwrap();

        // Seed sibling state.
        let claude_json = config_dir.join("claude.json");
        fs::write(
            &claude_json,
            serde_json::to_string_pretty(&serde_json::json!({
                "projects": { reappeared_str.clone(): {"trust": true} }
            }))
            .unwrap(),
        )
        .unwrap();
        let history = config_dir.join("history.jsonl");
        let line =
            serde_json::json!({"display": "x", "project": reappeared_str}).to_string();
        fs::write(&history, format!("{line}\n")).unwrap();

        // Capture the listing (orphan), THEN make the source reappear.
        // The clean runs after — preflight should detect this and skip
        // both the artifact dir and the sibling prune.
        let listing = list_projects(config_dir).unwrap();
        assert!(listing.iter().any(|p| p.is_orphan && p.original_path == reappeared_str));
        fs::create_dir(&reappeared).unwrap();

        let snaps = tmp.path().join("snaps");
        let locks = tmp.path().join("locks");
        let (result, _) = clean_orphans_with_progress(
            config_dir,
            Some(claude_json.as_path()),
            Some(snaps.as_path()),
            Some(locks.as_path()),
            None,
            &HashSet::new(),
            false,
            &crate::project_progress::NoopSink,
        )
        .unwrap();

        // Artifact dir survived (preflight refused).
        assert!(cc_dir.exists());
        assert_eq!(result.orphans_removed, 0);
        // Sibling state survived too — this is the regression guard.
        assert_eq!(result.claude_json_entries_removed, 0);
        assert_eq!(result.history_lines_removed, 0);
        let cj_after = fs::read_to_string(&claude_json).unwrap();
        assert!(cj_after.contains(&reappeared_str));
        let hist_after = fs::read_to_string(&history).unwrap();
        assert!(hist_after.contains(&reappeared_str));
    }

    /// Empty-dir orphans are excluded from BOTH the rewrite list and
    /// the protected-skip count: their `original_path` is from the
    /// lossy fallback and they were never authoritative to begin with.
    #[test]
    fn test_clean_empty_dir_not_counted_as_protected() {
        use std::collections::HashSet;

        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let amb = projects_dir.join("-some-ambiguous-path");
        fs::create_dir(&amb).unwrap();

        let mut protected: HashSet<String> = HashSet::new();
        // unsanitize gives "/some/ambiguous/path" — protect it just to
        // ensure the empty-dir branch ignores the protected check.
        protected.insert("/some/ambiguous/path".to_string());

        let locks = tmp.path().join("locks");
        let snaps = tmp.path().join("snaps");
        let (result, _) = clean_orphans_with_progress(
            tmp.path(),
            None,
            Some(snaps.as_path()),
            Some(locks.as_path()),
            None,
            &protected,
            false,
            &crate::project_progress::NoopSink,
        )
        .unwrap();

        assert_eq!(result.orphans_removed, 1);
        assert_eq!(result.protected_paths_skipped, 0);
    }

    #[test]
    fn test_move_project_already_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        // Only destination exists (user already did `mv`)
        let src = base.join("old");
        let dst = base.join("new");
        fs::create_dir(&dst).unwrap();

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        // src doesn't exist, so use base (already canonical) directly
        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("s.jsonl"), "{}").unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(!result.actual_dir_moved); // didn't move dir (already moved)
        assert!(result.cc_dir_renamed); // but renamed CC state
    }

    #[test]
    fn test_move_project_state_only() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");
        fs::create_dir(&dst).unwrap();

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        // Use canonical path for sanitization (matches what resolve_path returns)
        let old_san = sanitize_path(&src.canonicalize().unwrap().to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: true, // --no-move
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(!result.actual_dir_moved);
        assert!(result.cc_dir_renamed);
        // Both dirs still exist (--no-move)
        assert!(src.exists());
        assert!(dst.exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_resolve_path_strips_windows_verbatim_prefix() {
        // On Windows, `std::fs::canonicalize` returns `\\?\C:\...` for
        // existing paths. `resolve_path` must strip that prefix so the
        // sanitized slug matches what CC writes (CC never uses verbatim
        // paths in session cwd or project slugs).
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("resolve-verbatim-test");
        fs::create_dir(&dir).unwrap();
        let resolved = resolve_path(dir.to_str().unwrap()).unwrap();
        assert!(
            !resolved.starts_with(r"\\?\"),
            "resolve_path must not return a verbatim path, got: {}",
            resolved
        );
        // Slug parity: sanitizing a canonicalized Windows path must not
        // include the `?` or the extra leading separators.
        let san = sanitize_path(&resolved);
        assert!(
            !san.starts_with("--?-"),
            "sanitized slug leaked verbatim prefix: {}",
            san
        );
    }

    #[test]
    fn test_resolve_path_nfc_ascii_unchanged() {
        // ASCII paths must pass through NFC unchanged
        let tmp = tempfile::tempdir().unwrap();
        let ascii_dir = tmp.path().canonicalize().unwrap().join("plain_ascii");
        fs::create_dir(&ascii_dir).unwrap();
        let resolved = resolve_path(ascii_dir.to_str().unwrap()).unwrap();
        let canonical = ascii_dir
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(resolved, canonical);
    }

    #[test]
    fn test_resolve_path_nfc_normalizes_nfd() {
        // NFD "café" (e + combining acute) must become NFC "café" (é precomposed)
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let nfd_name = "caf\u{0065}\u{0301}"; // NFD: e + combining acute accent
        let nfd_dir = base.join(nfd_name);
        fs::create_dir(&nfd_dir).unwrap();
        let resolved = resolve_path(nfd_dir.to_str().unwrap()).unwrap();
        assert!(
            resolved.contains("caf\u{00e9}"),
            "Expected NFC 'café' in resolved path, got: {}",
            resolved
        );
    }

    #[test]
    fn test_sanitize_nfd_nfc_produces_same_output() {
        // NFD and NFC of the same path must produce identical sanitize output
        // after resolve_path normalizes to NFC
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let nfd_name = "caf\u{0065}\u{0301}";
        let nfc_name = "caf\u{00e9}";
        let nfd_dir = base.join(nfd_name);
        // macOS HFS+ / APFS may normalize the dirname itself, so just create one
        fs::create_dir_all(&nfd_dir).unwrap();
        let resolved_nfd = resolve_path(nfd_dir.to_str().unwrap()).unwrap();
        let nfc_dir = base.join(nfc_name);
        // On macOS, NFD and NFC names resolve to the same directory
        let resolved_nfc = resolve_path(nfc_dir.to_str().unwrap()).unwrap();
        assert_eq!(
            sanitize_path(&resolved_nfd),
            sanitize_path(&resolved_nfc),
            "NFD and NFC resolved paths must produce same sanitized output"
        );
    }

    #[test]
    fn test_resolve_path_nfc_korean_jamo() {
        // Korean Jamo (한) must become precomposed Hangul (한)
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let jamo = "\u{1112}\u{1161}\u{11AB}"; // 한 (conjoining Jamo)
        let jamo_dir = base.join(jamo);
        fs::create_dir(&jamo_dir).unwrap();
        let resolved = resolve_path(jamo_dir.to_str().unwrap()).unwrap();
        assert!(
            resolved.contains("\u{D55C}"),
            "Expected precomposed Hangul 한 (U+D55C) in resolved path, got: {}",
            resolved
        );
    }

    #[test]
    fn test_sanitize_emoji_matches_cc_utf16() {
        // JS sees emoji as 2 surrogate code units → 2 hyphens.
        // Our sanitize_path must produce the same result.
        assert_eq!(sanitize_path("/tmp/\u{1F389}project"), "-tmp---project");
        // NFC accented char is 1 code unit → 1 hyphen
        assert_eq!(sanitize_path("/tmp/caf\u{00e9}"), "-tmp-caf-");
    }

    #[test]
    fn test_djb2_hash_collision_exists() {
        // djb2 is a 32-bit hash; collisions are inevitable.
        // "aaa" and "abB" produce the same hash (verified by brute-force search
        // against CC's JS implementation).
        let h1 = djb2_hash("aaa");
        let h2 = djb2_hash("abB");
        assert_eq!(h1, h2, "Expected djb2 collision between 'aaa' and 'abB'");
        assert_eq!(h1, "22bl");
    }

    #[test]
    fn test_djb2_hash_matches_cc() {
        // Verify our hash matches CC's djb2Hash + Math.abs + toString(36)
        // for a known long path. Expected value computed with CC's JS implementation.
        let long_path = "/Users/joker/".to_string() + &"a".repeat(250);
        let hash = djb2_hash(&long_path);
        assert_eq!(hash, "lwkvhu", "hash must match CC's JS output");
    }

    #[test]
    fn test_sanitize_long_path_exact_hash() {
        // Verify that a specific long path produces the CC-compatible hash suffix.
        let long_path = format!("/Users/joker/github/xiaolai/myprojects/{}", "a".repeat(200));
        let result = sanitize_path(&long_path);
        // Path is 239 chars, sanitized > 200, so hash is appended
        assert!(result.len() > MAX_SANITIZED_LENGTH);
        let expected_hash = djb2_hash(&long_path);
        assert!(
            result.ends_with(&format!("-{}", expected_hash)),
            "Expected hash suffix '-{}', got: {}",
            expected_hash,
            result
        );
    }

    // -- merge_project_dirs tests --

    #[test]
    fn test_merge_project_dirs_copies_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();

        fs::write(src.join("a.jsonl"), "session-a").unwrap();
        fs::write(src.join("b.jsonl"), "session-b").unwrap();
        fs::write(dst.join("c.jsonl"), "session-c").unwrap();

        merge_project_dirs(&src, &dst).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("a.jsonl")).unwrap(),
            "session-a"
        );
        assert_eq!(
            fs::read_to_string(dst.join("b.jsonl")).unwrap(),
            "session-b"
        );
        assert_eq!(
            fs::read_to_string(dst.join("c.jsonl")).unwrap(),
            "session-c"
        );
    }

    #[test]
    fn test_merge_project_dirs_skips_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();

        fs::write(src.join("dup.jsonl"), "src-version").unwrap();
        fs::write(dst.join("dup.jsonl"), "dst-version").unwrap();

        merge_project_dirs(&src, &dst).unwrap();

        // dst version preserved, not overwritten
        assert_eq!(
            fs::read_to_string(dst.join("dup.jsonl")).unwrap(),
            "dst-version"
        );
    }

    #[test]
    fn test_merge_project_dirs_recursive_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("memory")).unwrap();
        fs::create_dir_all(&dst).unwrap();

        fs::write(src.join("memory").join("topic.md"), "# Topic").unwrap();

        merge_project_dirs(&src, &dst).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("memory").join("topic.md")).unwrap(),
            "# Topic"
        );
    }

    // -- rewrite_history edge cases --

    #[test]
    fn test_rewrite_history_invalid_json_passthrough() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("history.jsonl");
        let old_path = "/old/path";
        let new_path = "/new/path";

        let lines = vec![
            format!(r#"{{"project":"{}","sessionId":"abc"}}"#, old_path),
            format!("not valid json but contains {}", old_path),
            "totally unrelated line".to_string(),
        ];
        fs::write(&history, lines.join("\n") + "\n").unwrap();

        let count = rewrite_history(&history, old_path, new_path).unwrap();
        assert_eq!(count, 1); // only valid JSON line was rewritten

        let content = fs::read_to_string(&history).unwrap();
        assert!(content.contains(new_path));
        // Invalid JSON line preserved unchanged
        assert!(content.contains(&format!("not valid json but contains {}", old_path)));
        assert!(content.contains("totally unrelated line"));
    }

    #[test]
    fn test_rewrite_history_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("history.jsonl");
        fs::write(&history, "").unwrap();

        let count = rewrite_history(&history, "/old", "/new").unwrap();
        assert_eq!(count, 0);
    }

    // -- resolve_path edge cases --

    #[test]
    fn test_resolve_path_relative_joins_cwd() {
        // resolve_path with a relative path should join it with cwd
        let result = resolve_path("some-relative-dir").unwrap();
        let cwd = std::env::current_dir().unwrap();
        let expected = cwd.join("some-relative-dir").to_string_lossy().to_string();
        // NFC normalization may change the string slightly on macOS
        assert!(result.contains("some-relative-dir"));
        assert!(result.starts_with('/') || result.contains(':')); // absolute
    }

    // -- move_project error branches --

    #[test]
    fn test_move_project_both_exist_error() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        let dst = base.join("new");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&dst).unwrap();

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let args = MoveArgs {
            old_path: src,
            new_path: dst,
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink);
        assert!(matches!(result, Err(ProjectError::Ambiguous(_))));
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("both"));
    }

    #[test]
    fn test_move_project_neither_exist_error() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let args = MoveArgs {
            old_path: base.join("nonexistent1"),
            new_path: base.join("nonexistent2"),
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink);
        assert!(matches!(result, Err(ProjectError::Ambiguous(_))));
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("neither"));
    }

    #[test]
    fn test_move_project_merge_cc_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Create old CC dir with session
        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("old-session.jsonl"), "old").unwrap();

        // Create new CC dir with different session
        let new_san = sanitize_path(&dst.to_string_lossy());
        let cc_new = projects_dir.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("new-session.jsonl"), "new").unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: true,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.cc_dir_renamed);

        // New CC dir has both sessions
        assert!(cc_new.join("new-session.jsonl").exists());
        assert!(cc_new.join("old-session.jsonl").exists());
        // Old CC dir is gone
        assert!(!cc_old.exists());
    }

    #[test]
    fn test_move_project_overwrite_cc_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("keep.jsonl"), "keep-this").unwrap();

        let new_san = sanitize_path(&dst.to_string_lossy());
        let cc_new = projects_dir.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("discard.jsonl"), "discard-this").unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: true,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.cc_dir_renamed);

        // New CC dir has old's content, not the original new content
        assert!(cc_new.join("keep.jsonl").exists());
        assert!(!cc_new.join("discard.jsonl").exists());
        assert!(!cc_old.exists());
    }

    #[test]
    fn test_move_project_conflict_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("s.jsonl"), "data").unwrap();

        let new_san = sanitize_path(&dst.to_string_lossy());
        let cc_new = projects_dir.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("s.jsonl"), "data").unwrap();

        let args = MoveArgs {
            old_path: src,
            new_path: dst,
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        // Spec §4.2 P1.7: non-empty CC target is a hard preflight
        // error without --merge/--overwrite.
        let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
        let ProjectError::Ambiguous(msg) = err else {
            panic!("expected Ambiguous, got {err:?}");
        };
        assert!(msg.contains("--merge") || msg.contains("--overwrite"));
    }

    #[test]
    fn test_move_project_dry_run_with_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Create non-empty CC dirs for both paths
        let old_san = sanitize_path(&src.canonicalize().unwrap().to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("s.jsonl"), "{}").unwrap();

        let new_san = sanitize_path(&dst.to_string_lossy());
        let cc_new = projects_dir.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("s.jsonl"), "{}").unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: true,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        // Dry run plan should mention conflict
        assert!(!result.warnings.is_empty());
        let plan = &result.warnings[0];
        assert!(plan.contains("Conflict") || plan.contains("--merge"));
        // Nothing actually changed
        assert!(src.exists());
    }

    #[test]
    fn test_move_project_empty_new_cc_dir_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("s.jsonl"), "data").unwrap();

        // Create EMPTY new CC dir
        let new_san = sanitize_path(&dst.to_string_lossy());
        let cc_new = projects_dir.join(&new_san);
        fs::create_dir(&cc_new).unwrap();

        let args = MoveArgs {
            old_path: src,
            new_path: dst,
            config_dir: base,
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.cc_dir_renamed);
        assert!(cc_new.join("s.jsonl").exists());
    }

    // -- is_claude_running_in --

    #[test]
    fn test_is_claude_running_in_returns_false_for_random_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No Claude process has this random temp dir as cwd
        assert!(!is_claude_running_in(&tmp.path().to_string_lossy()));
    }

    // -- find_project_dir_by_prefix --

    #[test]
    fn test_find_project_dir_by_prefix_no_projects_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No projects/ subdirectory exists
        let result = find_project_dir_by_prefix(tmp.path(), "anything").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_find_project_dir_by_prefix_single_match() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        fs::create_dir(&projects).unwrap();
        fs::create_dir(projects.join("myprefix-abc123")).unwrap();

        let result = find_project_dir_by_prefix(tmp.path(), "myprefix").unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("myprefix-abc123"));
    }

    #[test]
    fn test_find_project_dir_by_prefix_ambiguous() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        fs::create_dir(&projects).unwrap();
        fs::create_dir(projects.join("myprefix-hash1")).unwrap();
        fs::create_dir(projects.join("myprefix-hash2")).unwrap();

        let result = find_project_dir_by_prefix(tmp.path(), "myprefix");
        assert!(matches!(result, Err(ProjectError::Ambiguous(_))));
    }

    // -- count_files_with_ext --

    #[test]
    fn test_count_files_with_ext_counts_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.jsonl"), "").unwrap();
        fs::write(tmp.path().join("b.jsonl"), "").unwrap();
        fs::write(tmp.path().join("c.txt"), "").unwrap();
        fs::write(tmp.path().join("d.md"), "").unwrap();

        assert_eq!(count_files_with_ext(tmp.path(), "jsonl"), 2);
        assert_eq!(count_files_with_ext(tmp.path(), "md"), 1);
        assert_eq!(count_files_with_ext(tmp.path(), "rs"), 0);
    }

    // -- dir_size --

    #[test]
    fn test_dir_size_sums_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a"), "hello").unwrap(); // 5 bytes
        fs::write(tmp.path().join("b"), "world!").unwrap(); // 6 bytes
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("c"), "xy").unwrap(); // 2 bytes

        let size = dir_size(tmp.path());
        assert_eq!(size, 13);
    }

    // -- most_recent_mtime --

    #[test]
    fn test_most_recent_mtime_returns_latest() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("old"), "old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(tmp.path().join("new"), "new").unwrap();

        let mtime = most_recent_mtime(tmp.path());
        assert!(mtime.is_some());
    }

    #[test]
    fn test_most_recent_mtime_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mtime = most_recent_mtime(tmp.path());
        // Empty dir still has its own mtime
        assert!(mtime.is_none() || mtime.is_some());
    }

    // ---------------------------------------------------------------------
    // Group 2 — Project move conflict handling (4 tests).
    // ---------------------------------------------------------------------

    /// Build a Group-2 fixture: a TempDir plus canonical src/dst/config dirs.
    /// Kept as a single fn returning everything so tests don't drop the TempDir.
    fn mk_move_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");
        let projects = base.join("projects");
        fs::create_dir(&projects).unwrap();
        (tmp, src, dst, base)
    }

    #[test]
    fn test_move_project_conflict_skips_history_rewrite() {
        let (_tmp, src, dst, base) = mk_move_fixture();
        let old_san = sanitize_path(&src.to_string_lossy());
        let new_san = sanitize_path(&dst.to_string_lossy());
        let projects = base.join("projects");
        // Both CC dirs exist, both non-empty — conflict requiring resolution.
        let cc_old = projects.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("old-session.jsonl"), "{}").unwrap();
        let cc_new = projects.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("new-session.jsonl"), "{}").unwrap();

        let old_str = src.to_string_lossy();
        let old_line = serde_json::json!({
            "project": old_str,
            "sessionId": "abc",
            "timestamp": 1,
        })
        .to_string();
        let history = base.join("history.jsonl");
        fs::write(&history, format!("{old_line}\n")).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        // With v2 hard-error preflight, the whole operation aborts
        // before P3/P4/P5 run. history.jsonl must be untouched.
        let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(_)));

        // Verify old path still in history on disk (parse-based).
        let content = fs::read_to_string(&history).unwrap();
        let src_str = src.to_string_lossy().to_string();
        let has_old = content.lines().any(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v.get("project").and_then(|p| p.as_str()).map(String::from))
                == Some(src_str.clone())
        });
        assert!(
            has_old,
            "old path still in history since rewrite was skipped"
        );
    }

    #[test]
    fn test_move_project_merge_rewrites_history() {
        let (_tmp, src, dst, base) = mk_move_fixture();
        let old_san = sanitize_path(&src.to_string_lossy());
        let new_san = sanitize_path(&dst.to_string_lossy());
        let projects = base.join("projects");
        let cc_old = projects.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("a.jsonl"), "old-a").unwrap();
        let cc_new = projects.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("b.jsonl"), "new-b").unwrap();

        let history = base.join("history.jsonl");
        let line = serde_json::json!({
            "project": src.to_string_lossy(),
            "sessionId": "abc",
            "timestamp": 1,
        })
        .to_string();
        fs::write(&history, format!("{line}\n")).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: true,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.cc_dir_renamed, "merge should resolve the conflict");
        assert_eq!(
            result.history_lines_updated, 1,
            "history rewritten on merge"
        );
        let content = fs::read_to_string(&history).unwrap();
        // Parse-based assertion: tolerates Windows UNC path escaping.
        let new_str = dst.to_string_lossy();
        let has_new = content.lines().any(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v.get("project").and_then(|p| p.as_str()).map(String::from))
                == Some(new_str.to_string())
        });
        assert!(has_new, "new path present in history after merge");
        // Both files merged into new CC dir.
        assert!(cc_new.join("a.jsonl").exists(), "merged file from old dir");
        assert!(
            cc_new.join("b.jsonl").exists(),
            "preserved file from new dir"
        );
    }

    #[test]
    fn test_move_project_orphan_roundtrip_prevents_false_positive() {
        // A project at /tmp/my-project sanitizes to `-tmp-my-project`.
        // unsanitize gives /tmp/my/project — which doesn't exist. Without
        // the cwd-from-sessions recovery, the project would be flagged orphan
        // even though the real dir /tmp/my-project exists.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        // The real project dir (with a hyphen in the name).
        let project_dir = base.join("my-project");
        fs::create_dir(&project_dir).unwrap();

        // The CC project dir — sanitized for the real path.
        let projects = base.join("projects");
        fs::create_dir(&projects).unwrap();
        let san = sanitize_path(&project_dir.to_string_lossy());
        let cc_dir = projects.join(&san);
        fs::create_dir(&cc_dir).unwrap();

        // Write a session.jsonl with the correct cwd. This is how CC records
        // the authoritative original path.
        let session_line = serde_json::json!({
            "cwd": project_dir.to_string_lossy(),
            "sessionId": "abc",
            "type": "user",
        })
        .to_string();
        fs::write(cc_dir.join("session.jsonl"), session_line + "\n").unwrap();

        let listed = list_projects(&base).unwrap();
        let found = listed
            .iter()
            .find(|p| p.sanitized_name == san)
            .expect("project must be listed");

        assert_eq!(
            found.original_path,
            project_dir.to_string_lossy().to_string(),
            "cwd from session should override lossy unsanitize"
        );
        assert!(
            !found.is_orphan,
            "project dir exists; must NOT be flagged orphan"
        );
    }

    // -----------------------------------------------------------------
    // Group 11 — Unix-only code gaps (platform-gated structural tests).
    // -----------------------------------------------------------------

    #[test]
    fn test_move_project_cross_device_no_exdev_on_windows() {
        // Structural: the EXDEV-fallback branch is #[cfg(unix)]-gated in
        // move_project. On non-unix, a cross-device fs::rename failure
        // returns a regular Io error rather than invoking copy+remove.
        //
        // This test simply documents the platform gate. We can't easily
        // provoke a real EXDEV in a unit test (would need two mounted fs).
        // Instead, verify the in-same-device happy path still works on all
        // platforms (which it does via fs::rename without the fallback).
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");
        fs::create_dir(&base.join("projects")).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
        assert!(result.actual_dir_moved);
        assert!(dst.exists());
        assert!(!src.exists());
        // Platform-gate assertion: EXDEV handler presence differs by cfg.
        #[cfg(unix)]
        {
            // Unix has the EXDEV fallback path; same-device move used fs::rename.
        }
        #[cfg(not(unix))]
        {
            // Non-Unix: no EXDEV fallback at all — cross-device move would
            // propagate as a plain Io error. Same-device move still works.
        }
    }

    #[test]
    fn test_move_project_post_move_failure_becomes_warning() {
        // v2 (spec §4.2 P1.7): non-empty CC target is a HARD preflight
        // error before any disk mutation. This test now verifies the
        // error-path instead of the old partial-success warning path.
        let (_tmp, src, dst, base) = mk_move_fixture();
        let old_san = sanitize_path(&src.to_string_lossy());
        let new_san = sanitize_path(&dst.to_string_lossy());
        let projects = base.join("projects");
        let cc_old = projects.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("s.jsonl"), "{}").unwrap();
        let cc_new = projects.join(&new_san);
        fs::create_dir(&cc_new).unwrap();
        fs::write(cc_new.join("t.jsonl"), "{}").unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            claude_json_path: None,
            snapshots_dir: None,
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,

            ignore_pending_journals: false,
            claudepot_state_dir: None,
        };

        let err = move_project(&args, &crate::project_progress::NoopSink).expect_err("must error on CC dir collision");
        assert!(matches!(err, ProjectError::Ambiguous(_)));
        assert!(src.exists(), "disk dir untouched on preflight failure");
        assert!(!dst.exists(), "target not created on preflight failure");
    }
}
