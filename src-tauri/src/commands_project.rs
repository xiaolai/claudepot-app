//! Tauri commands for the Projects section and its clean / repair
//! preview surfaces.
//!
//! Read-only (list, show), plan-only (dry-run move), and the clean
//! launcher / status commands. Repair journal editing / op-start lives
//! in `commands_repair.rs`.

use crate::dto::{
    CleanPreviewDto, DryRunPlanDto, JournalEntryDto, MoveArgsDto, ProjectDetailDto, ProjectInfoDto,
    ProjectRestoreReportDto, ProjectTrashListingDto, RemoveProjectPreviewBasicDto,
    RemoveProjectPreviewDto, RemoveProjectPreviewExtrasDto, RemoveProjectResultDto,
};
use crate::ops::{
    emit_terminal, new_op_id, new_running_op, spawn_op_thread, CleanResultSummary, OpKind,
    RunningOpInfo, RunningOps,
};
use claudepot_core::paths;
use claudepot_core::project;
use claudepot_core::project_dry_run_service::DryRunOutcome;
use claudepot_core::project_remove::{
    remove_project as core_remove_project, remove_project_preview, remove_project_preview_basic,
    remove_project_preview_extras, RemoveArgs,
};
use claudepot_core::project_repair;
use claudepot_core::project_trash;
use tauri::{AppHandle, State};

/// Default journal nag threshold per spec §8 Q7 — mirrors the CLI.
pub(crate) const JOURNAL_NAG_THRESHOLD_SECS: u64 = 86_400;

pub(crate) fn claudepot_home_dirs() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)
{
    paths::claudepot_repair_dirs()
}

#[tauri::command]
pub async fn project_list() -> Result<Vec<ProjectInfoDto>, String> {
    // `list_projects` fans out over every project slug, running
    // `dir_size` (recursive), `recover_cwd_from_sessions` (JSONL I/O),
    // `classify_reachability` (stat + optional slow-mount checks), and
    // `canonicalize` on each. Multi-hundred-ms in the tail; keep it
    // off the Tokio IPC worker so other commands don't queue behind it.
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = paths::claude_config_dir();
        let projects = project::list_projects(&cfg).map_err(|e| format!("list failed: {e}"))?;
        Ok(projects.iter().map(ProjectInfoDto::from).collect())
    })
    .await
    .map_err(|e| format!("list join: {e}"))?
}

#[tauri::command]
pub async fn project_show(path: String) -> Result<ProjectDetailDto, String> {
    // Same heavy I/O shape as `project_list`, focused on a single slug.
    // Fires on every row click — the freeze the user feels is this
    // command holding a worker for seconds on large projects or
    // stat-slow source paths.
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let detail = project::show_project(&cfg, &path).map_err(|e| format!("show failed: {e}"))?;
        Ok(ProjectDetailDto::from(&detail))
    })
    .await
    .map_err(|e| format!("show join: {e}"))?
}

/// Sentinel the client checks for and silently discards. Distinguished
/// from real failures so the preview pane doesn't flash an error
/// state just because the user kept typing.
///
/// Lives in the IPC layer on purpose — it's a Tauri-bridge protocol
/// artifact, not a domain concept. The core service surfaces a typed
/// `DryRunOutcome::Superseded`; we map it to this string here.
const DRY_RUN_SUPERSEDED: &str = "__claudepot_dry_run_superseded__";

#[tauri::command]
pub async fn project_move_dry_run(
    args: MoveArgsDto,
    svc: State<'_, crate::state::DryRunState>,
) -> Result<DryRunPlanDto, String> {
    let cfg = paths::claude_config_dir();
    let claude_json_path = dirs::home_dir().map(|h| h.join(".claude.json"));
    let repair_root = paths::claudepot_repair_dir();
    let snapshots_dir = Some(repair_root.join("snapshots"));
    let core_args = project::MoveArgs {
        old_path: args.old_path.into(),
        new_path: args.new_path.into(),
        config_dir: cfg,
        claude_json_path,
        snapshots_dir,
        no_move: args.no_move,
        merge: args.merge,
        overwrite: args.overwrite,
        force: args.force,
        dry_run: true, // enforced; caller cannot turn this off
        ignore_pending_journals: args.ignore_pending_journals,
        claudepot_state_dir: Some(repair_root),
    };

    let token = args.cancel_token.unwrap_or(0);
    // `dry_run` walks the source + target trees, reads snapshots, and
    // consults `.claude.json`. Keep it off the Tokio IPC worker so
    // rapid-typing token races don't starve other commands.
    let svc_arc = std::sync::Arc::clone(&svc.0);
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        svc_arc
            .dry_run(core_args, token)
            .map_err(|e| format!("dry-run failed: {e}"))
    })
    .await
    .map_err(|e| format!("dry-run join: {e}"))??;

    match outcome {
        DryRunOutcome::Plan(p) => Ok(DryRunPlanDto::from(&p)),
        DryRunOutcome::Superseded => Err(DRY_RUN_SUPERSEDED.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Project clean (orphan reclaim) surface
// ---------------------------------------------------------------------------

/// Return the set of projects that would be cleaned and the count of
/// unreachable candidates skipped. Read-only: no lock, no deletion.
/// The pending-journals gate is NOT applied here because this is just
/// a preview — the gate fires on `project_clean_execute`.
#[tauri::command]
pub async fn project_clean_preview() -> Result<CleanPreviewDto, String> {
    // `clean_preview` calls `clean_orphans(dry_run=true)` internally,
    // which scans every project slug — same heavy path as
    // `project_list`. Runs on the blocking pool.
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = paths::claude_config_dir();
        let (_journals, locks, snaps) = claudepot_home_dirs();
        // `claude.json` inspection during preview would be a read;
        // skip for now — the execute path handles that, and the
        // preview just shows what will be removed.
        let preview = project::clean_preview(
            &cfg,
            None,
            Some(snaps.as_path()),
            Some(locks.as_path()),
            &paths::claudepot_data_dir(),
        )
        .map_err(|e| format!("clean preview failed: {e}"))?;
        Ok(CleanPreviewDto::from(&preview))
    })
    .await
    .map_err(|e| format!("clean preview join: {e}"))?
}

/// Kick off a clean in the background, returning the op_id the UI
/// subscribes to on `op-progress::<op_id>`. Gated on no pending rename
/// journals; the `__clean__` lock is acquired inside `clean_orphans`
/// so two concurrent starts can't race (the loser errors out via the
/// terminal op event).
#[tauri::command]
pub async fn project_clean_start(
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let (journals, locks, snaps) = claudepot_home_dirs();

    let actionable = project_repair::list_actionable(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
        .map_err(|e| format!("journal check failed: {e}"))?;
    if !actionable.is_empty() {
        return Err(format!(
            "refusing to clean while {} rename journal(s) are pending. Resolve them in the Repair view first.",
            actionable.len()
        ));
    }

    let op_id = new_op_id();
    ops.insert(new_running_op(&op_id, OpKind::CleanProjects, "", ""));

    let cfg = paths::claude_config_dir();
    let claude_json = dirs::home_dir().map(|h| h.join(".claude.json"));
    // Resolve protected paths once on the spawning thread so the
    // background task gets a snapshot — list mutations during a
    // multi-second clean must not change the rules mid-flight. On
    // read failure, fall back to built-in defaults (audit fix: an
    // empty set would silently disable protection for `/`, `~`,
    // `/Users`, etc.).
    let protected =
        claudepot_core::protected_paths::resolved_set_or_defaults(&paths::claudepot_data_dir());

    // `spawn_op_thread` uses a plain OS thread, not `spawn_blocking`
    // — Tauri's sync #[command] runs outside a tokio runtime context
    // on at least some dispatch paths, and `spawn_blocking` panics
    // with "no reactor running" there. Our work is blocking I/O (fs
    // scans, remove_dir_all) with no await points anyway.
    let ops_for_task = ops.inner().clone();
    spawn_op_thread(
        app,
        ops_for_task,
        op_id.clone(),
        move |sink, app, ops, op_id| {
            let repair_root = paths::claudepot_repair_dir();
            let result = project::clean_orphans_with_progress(
                &cfg,
                claude_json.as_deref(),
                Some(snaps.as_path()),
                Some(locks.as_path()),
                Some(repair_root.as_path()),
                &protected,
                false,
                &sink,
            );
            match result {
                Ok((clean, _orphans)) => {
                    ops.update(&op_id, |op| {
                        op.clean_result = Some(CleanResultSummary::from_core(&clean));
                    });
                    emit_terminal(&app, &ops, &op_id, None);
                }
                Err(e) => {
                    emit_terminal(&app, &ops, &op_id, Some(format!("clean failed: {e}")));
                }
            }
        },
    );

    Ok(op_id)
}

/// Fetch the current state of an in-flight clean. Mirrors
/// `project_move_status`. Returns `None` after the post-terminal
/// grace window expires.
#[tauri::command]
pub async fn project_clean_status(
    op_id: String,
    ops: State<'_, RunningOps>,
) -> Result<Option<RunningOpInfo>, String> {
    Ok(ops.get(&op_id))
}

#[tauri::command]
pub async fn repair_list() -> Result<Vec<JournalEntryDto>, String> {
    // Walks the journal dir + reads each journal header. Typical N is
    // tiny, but the PendingJournalsBanner polls on refresh and the
    // Maintenance view fans it out with other ops.
    tauri::async_runtime::spawn_blocking(|| {
        let (journals, locks, _snaps) = claudepot_home_dirs();
        let entries =
            project_repair::list_pending_with_status(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
                .map_err(|e| format!("repair list failed: {e}"))?;
        Ok(entries.iter().map(JournalEntryDto::from).collect())
    })
    .await
    .map_err(|e| format!("repair list join: {e}"))?
}

/// Cheap count for the PendingJournalsBanner. Only counts *actionable*
/// entries — excludes the `abandoned` class so the banner doesn't
/// perpetually nag about a user-dismissed entry.
#[tauri::command]
pub async fn repair_pending_count() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (journals, locks, _snaps) = claudepot_home_dirs();
        let entries =
            project_repair::list_actionable(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
                .map_err(|e| format!("repair count failed: {e}"))?;
        Ok(entries.len())
    })
    .await
    .map_err(|e| format!("repair count join: {e}"))?
}

/// Status-aware banner input: counts per journal class so the UI can
/// pick a neutral / warning tone based on staleness. Abandoned entries
/// are filtered out; running entries are surfaced separately so the
/// banner can suppress itself for them (RunningOpStrip already shows
/// the op live).
#[tauri::command]
pub async fn repair_status_summary() -> Result<crate::dto::PendingJournalsSummaryDto, String> {
    tauri::async_runtime::spawn_blocking(|| {
        use claudepot_core::project_journal::JournalStatus;
        let (journals, locks, _snaps) = claudepot_home_dirs();
        let entries =
            project_repair::list_pending_with_status(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
                .map_err(|e| format!("repair summary failed: {e}"))?;

        let mut pending = 0usize;
        let mut stale = 0usize;
        let mut running = 0usize;
        for e in &entries {
            match e.status {
                JournalStatus::Pending => pending += 1,
                JournalStatus::Stale => stale += 1,
                JournalStatus::Running => running += 1,
                JournalStatus::Abandoned => {} // filtered
            }
        }
        Ok(crate::dto::PendingJournalsSummaryDto {
            pending,
            stale,
            running,
        })
    })
    .await
    .map_err(|e| format!("repair summary join: {e}"))?
}

// ---------------------------------------------------------------------------
// project remove — single-target trash
// ---------------------------------------------------------------------------

/// Build the standard claudepot path layout into a RemoveArgs bundle.
/// Pulled out so preview and execute share exactly the same resolution
/// path — the GUI's confirmation modal renders against the preview,
/// then the execute hits the same target.
fn remove_paths() -> (
    std::path::PathBuf, // config_dir
    std::path::PathBuf, // claude_json
    std::path::PathBuf, // history
    std::path::PathBuf, // snapshots
    std::path::PathBuf, // locks
    std::path::PathBuf, // data_dir
) {
    let config_dir = paths::claude_config_dir();
    let claude_json = dirs::home_dir()
        .map(|h| h.join(".claude.json"))
        .unwrap_or_else(|| std::path::PathBuf::from(".claude.json"));
    let history = config_dir.join("history.jsonl");
    let (_journals, locks, snaps) = claudepot_home_dirs();
    let data_dir = paths::claudepot_data_dir();
    (config_dir, claude_json, history, snaps, locks, data_dir)
}

/// Cheap preview — slug, paths, sessions, size, last_modified. No
/// live-session probe, no large-file reads. The GUI calls this for
/// the modal's first paint so the disclosure shows up instantly even
/// when `~/.claude.json` or `history.jsonl` are multi-MB.
#[tauri::command]
pub async fn project_remove_preview_basic(
    target: String,
) -> Result<RemoveProjectPreviewBasicDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (config_dir, claude_json, history, snaps, locks, data_dir) = remove_paths();
        let args = RemoveArgs {
            config_dir: &config_dir,
            claude_json_path: Some(&claude_json),
            history_path: Some(&history),
            snapshots_dir: &snaps,
            locks_dir: &locks,
            data_dir: &data_dir,
            target: &target,
        };
        let basic = remove_project_preview_basic(&args)
            .map_err(|e| format!("preview basic failed: {e}"))?;
        Ok(RemoveProjectPreviewBasicDto::from(&basic))
    })
    .await
    .map_err(|e| format!("preview basic join: {e}"))?
}

/// Slow preview — runs the lsof-backed live-session probe and parses
/// `~/.claude.json` + `history.jsonl` end-to-end. Returns the
/// disabled-state metadata the modal uses to gate the Remove button
/// and annotate the disclosure ("with .claude.json entry · N history
/// lines"). Issued in parallel with `project_remove_preview_basic`.
#[tauri::command]
pub async fn project_remove_preview_extras(
    target: String,
) -> Result<RemoveProjectPreviewExtrasDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (config_dir, claude_json, history, snaps, locks, data_dir) = remove_paths();
        let args = RemoveArgs {
            config_dir: &config_dir,
            claude_json_path: Some(&claude_json),
            history_path: Some(&history),
            snapshots_dir: &snaps,
            locks_dir: &locks,
            data_dir: &data_dir,
            target: &target,
        };
        let extras = remove_project_preview_extras(&args)
            .map_err(|e| format!("preview extras failed: {e}"))?;
        Ok(RemoveProjectPreviewExtrasDto::from(&extras))
    })
    .await
    .map_err(|e| format!("preview extras join: {e}"))?
}

/// Read-only preview the GUI's RemoveProjectModal renders. Live-session
/// presence is reported but not blocking — the modal needs to surface
/// the reason and disable the confirm path itself, per the design rule
/// ("disabled buttons state a reason inline").
#[tauri::command]
pub async fn project_remove_preview(target: String) -> Result<RemoveProjectPreviewDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (config_dir, claude_json, history, snaps, locks, data_dir) = remove_paths();
        let args = RemoveArgs {
            config_dir: &config_dir,
            claude_json_path: Some(&claude_json),
            history_path: Some(&history),
            snapshots_dir: &snaps,
            locks_dir: &locks,
            data_dir: &data_dir,
            target: &target,
        };
        let preview = remove_project_preview(&args).map_err(|e| format!("preview failed: {e}"))?;
        Ok(RemoveProjectPreviewDto::from(&preview))
    })
    .await
    .map_err(|e| format!("preview join: {e}"))?
}

/// Execute the trash. Synchronous-from-the-frontend's-perspective —
/// removes are O(seconds) for typical projects and don't need the
/// op-progress channel. The pending-journals gate fires here, mirroring
/// the CLI gate.
#[tauri::command]
pub async fn project_remove_execute(target: String) -> Result<RemoveProjectResultDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (journals, locks_dir, _snaps) = claudepot_home_dirs();
        let actionable =
            project_repair::list_actionable(&journals, &locks_dir, JOURNAL_NAG_THRESHOLD_SECS)
                .map_err(|e| format!("journal check failed: {e}"))?;
        if !actionable.is_empty() {
            return Err(format!(
                "refusing to remove while {} rename journal(s) are pending. Resolve them in the Repair view first.",
                actionable.len()
            ));
        }

        let (config_dir, claude_json, history, snaps, locks, data_dir) = remove_paths();
        let args = RemoveArgs {
            config_dir: &config_dir,
            claude_json_path: Some(&claude_json),
            history_path: Some(&history),
            snapshots_dir: &snaps,
            locks_dir: &locks,
            data_dir: &data_dir,
            target: &target,
        };
        let result = core_remove_project(&args)
            .map_err(|e| format!("remove failed: {e}"))?;
        Ok(RemoveProjectResultDto::from(&result))
    })
    .await
    .map_err(|e| format!("remove join: {e}"))?
}

#[tauri::command]
pub async fn project_trash_list() -> Result<ProjectTrashListingDto, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let data_dir = paths::claudepot_data_dir();
        let listing = project_trash::list(&data_dir, project_trash::ProjectTrashFilter::default())
            .map_err(|e| format!("trash list failed: {e}"))?;
        Ok(ProjectTrashListingDto::from(&listing))
    })
    .await
    .map_err(|e| format!("trash list join: {e}"))?
}

#[tauri::command]
pub async fn project_trash_restore(entry_id: String) -> Result<ProjectRestoreReportDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (config_dir, claude_json, history, _snaps, _locks, data_dir) = remove_paths();
        let report = project_trash::restore(
            &data_dir,
            &entry_id,
            &config_dir,
            Some(&claude_json),
            Some(&history),
        )
        .map_err(|e| format!("restore failed: {e}"))?;
        Ok(ProjectRestoreReportDto::from(&report))
    })
    .await
    .map_err(|e| format!("restore join: {e}"))?
}

#[tauri::command]
pub async fn project_trash_empty(older_than_days: Option<u64>) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let data_dir = paths::claudepot_data_dir();
        let filter = project_trash::ProjectTrashFilter {
            older_than: older_than_days
                .map(|d| std::time::Duration::from_secs(d.saturating_mul(86_400))),
        };
        project_trash::empty(&data_dir, filter).map_err(|e| format!("empty failed: {e}"))
    })
    .await
    .map_err(|e| format!("empty join: {e}"))?
}
