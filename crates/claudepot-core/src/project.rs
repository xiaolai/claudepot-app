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
    if !args.force
        && (live_session_present(&args.config_dir, &old_san, &old_norm)
            || live_session_present(&args.config_dir, &new_san, &new_norm))
        {
            let _ = journal.mark_error("live CC session detected");
            return Err(ProjectError::ClaudeRunning(old_norm.clone()));
        }

    // Phase 3: Move actual directory
    if scenario == MoveScenario::MoveAndUpdate {
        if let Some(parent) = Path::new(&new_norm).parent() {
            fs::create_dir_all(parent).map_err(ProjectError::Io)?;
        }
        // E3: Case-only rename on case-insensitive FS needs a two-step
        // via an intermediate name, else `fs::rename` is a no-op on
        // APFS/NTFS. The intermediate name is recorded in the journal
        // (audit B3 fix) so a crash between the two renames leaves a
        // recoverable trail instead of a stranded `*.claudepot-caserename-*`
        // directory.
        let case_only = is_case_only_rename(&old_norm, &new_norm);
        if case_only {
            let tmp_name = format!(
                "{}.claudepot-caserename-{}",
                new_norm,
                std::process::id()
            );
            // Record before mutating disk so a crash mid-rename has the
            // breadcrumb. snapshot_paths is repurposed here: the path
            // is the in-flight temp dir, not a copy of pre-state, so
            // repair / cleanup tooling can finish or roll back the
            // half-renamed move.
            let _ = journal.record_snapshot(std::path::PathBuf::from(&tmp_name));
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
                    // Audit B3 fix: record the temp dir so repair can
                    // finish or roll back if we crash between renames.
                    let _ = journal.record_snapshot(tmp.clone());
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

/// True iff `old` and `new` differ only in ASCII letter case AND both
/// resolve to the same on-disk entry (i.e. the FS is case-insensitive
/// for these inputs). Used to trigger the E3 two-step case-only rename.
///
/// Audit B3 fix: the legacy implementation was string-only
/// (`old.eq_ignore_ascii_case(new)`), which on a case-sensitive FS
/// where `foo/` and `Foo/` are distinct directories would mis-classify
/// a real `foo → Foo` rename (target exists, source exists, different
/// inodes) as case-only. The two-step rename would then move `foo`
/// through a temporary name and overwrite `Foo` — data loss.
///
/// Same-file identity is the authoritative signal: if both paths exist
/// AND `metadata.dev/ino` agree (Unix) or `metadata.file_index/volume`
/// agree (Windows), the FS is case-insensitive for this case rename.
/// If either path is missing we fall back to the string check — there's
/// no inode to compare and the rename is mechanically a normal move.
fn is_case_only_rename(old: &str, new: &str) -> bool {
    if old == new || !old.eq_ignore_ascii_case(new) {
        return false;
    }
    // Both string-equal-modulo-case. Probe the FS for same-file
    // identity. If the FS reports two distinct entries, this is NOT a
    // case-only rename — it's a real move with a target collision.
    match same_file_identity(old, new) {
        Some(true) => true,
        // Definitively distinct entries on a case-sensitive FS.
        Some(false) => false,
        // Couldn't probe (one side missing, stat error). Fall back to
        // the string check so the legacy "either-side-only-exists"
        // case rename keeps working.
        None => true,
    }
}

/// Probe whether two paths point at the same FS entry. Returns
/// `Some(true)` on confirmed same-inode (case-insensitive FS), `Some(false)`
/// on confirmed distinct entries (case-sensitive FS), and `None` when we
/// can't tell — typically because one side doesn't exist or stat failed.
fn same_file_identity(a: &str, b: &str) -> Option<bool> {
    let ma = fs::metadata(a).ok()?;
    let mb = fs::metadata(b).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(ma.dev() == mb.dev() && ma.ino() == mb.ino())
    }
    #[cfg(windows)]
    {
        // On Windows, same-file identity requires opening both handles
        // and comparing FILE_ID_INFO. `std::os::windows::fs::MetadataExt`
        // exposes `volume_serial_number()` and `file_index()` (added in
        // 1.75); use those when available. If we can't get a clean
        // pair, return None so the caller falls back to string parity.
        use std::os::windows::fs::MetadataExt;
        match (
            ma.volume_serial_number(),
            mb.volume_serial_number(),
            ma.file_index(),
            mb.file_index(),
        ) {
            (Some(va), Some(vb), Some(ia), Some(ib)) => Some(va == vb && ia == ib),
            _ => None,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (ma, mb);
        None
    }
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

/// Read-only preview of what `clean_orphans` would do — the list of
/// candidate orphans plus the aggregate counts the GUI's confirmation
/// modal needs to render an honest disclosure (total bytes reclaimed,
/// how many candidates fall under the user's protected-paths set).
///
/// This is the business policy that used to live inline in the
/// `project_clean_preview` Tauri command. Computing it here keeps the
/// preview policy in lock-step with the execute path: the candidate
/// list comes from `clean_orphans(.., dry_run=true)` rather than a
/// re-scan, so no list can ever drift between preview and execute.
///
/// The protected-set resolution uses
/// `protected_paths::resolved_set_or_defaults`, which falls back to
/// the built-in defaults if the user's `protected_paths.json` is
/// unreadable. The audit-flagged failure mode — empty set silently
/// disabling protection — cannot happen here.
#[derive(Debug, Clone)]
pub struct CleanPreview {
    pub orphans: Vec<ProjectInfo>,
    pub orphans_found: usize,
    pub unreachable_skipped: usize,
    pub total_bytes: u64,
    pub protected_count: usize,
}

pub fn clean_preview(
    config_dir: &Path,
    claude_json_path: Option<&Path>,
    snapshots_dir: Option<&Path>,
    locks_dir: Option<&Path>,
    claudepot_data_dir: &Path,
) -> Result<CleanPreview, ProjectError> {
    let (result, orphans) = clean_orphans(
        config_dir,
        claude_json_path,
        snapshots_dir,
        locks_dir,
        true, // dry-run: lock-step with execute path's candidate list
    )?;

    let total_bytes: u64 = orphans.iter().map(|p| p.total_size_bytes).sum();
    // Empty-dir orphans are excluded from the protected count for the
    // same reason `clean_orphans_with_progress` excludes them from
    // `protected_paths_skipped`: their `original_path` came from the
    // lossy `unsanitize_path` fallback, so a protected-set match would
    // be coincidental, not authoritative. Mirror that predicate
    // exactly so preview and execute report the same number.
    let protected =
        crate::protected_paths::resolved_set_or_defaults(claudepot_data_dir);
    let protected_count = orphans
        .iter()
        .filter(|p| !p.is_empty && protected.contains(&p.original_path))
        .count();

    Ok(CleanPreview {
        orphans,
        orphans_found: result.orphans_found,
        unreachable_skipped: result.unreachable_skipped,
        total_bytes,
        protected_count,
    })
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
pub(crate) fn remove_claude_json_entries_batch(
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
pub(crate) fn remove_history_lines_batch(
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
#[path = "project_tests.rs"]
mod tests;
