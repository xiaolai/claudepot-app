//! Tauri commands for the Projects section and its clean / repair
//! preview surfaces.
//!
//! Read-only (list, show), plan-only (dry-run move), and the clean
//! launcher / status commands. Repair journal editing / op-start lives
//! in `commands_repair.rs`.

use crate::dto::{
    CleanPreviewDto, DryRunPlanDto, JournalEntryDto, MoveArgsDto, ProjectDetailDto,
    ProjectInfoDto,
};
use crate::ops::{
    emit_terminal, new_op_id, new_running_op, spawn_op_thread, CleanResultSummary, OpKind,
    RunningOpInfo, RunningOps,
};
use claudepot_core::paths;
use claudepot_core::project;
use claudepot_core::project_dry_run_service::DryRunOutcome;
use claudepot_core::project_repair;
use tauri::{AppHandle, State};

/// Default journal nag threshold per spec §8 Q7 — mirrors the CLI.
pub(crate) const JOURNAL_NAG_THRESHOLD_SECS: u64 = 86_400;

pub(crate) fn claudepot_home_dirs()
    -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)
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
        let detail =
            project::show_project(&cfg, &path).map_err(|e| format!("show failed: {e}"))?;
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

    let actionable =
        project_repair::list_actionable(&journals, &locks, JOURNAL_NAG_THRESHOLD_SECS)
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
    let protected = claudepot_core::protected_paths::resolved_set_or_defaults(
        &paths::claudepot_data_dir(),
    );

    // `spawn_op_thread` uses a plain OS thread, not `spawn_blocking`
    // — Tauri's sync #[command] runs outside a tokio runtime context
    // on at least some dispatch paths, and `spawn_blocking` panics
    // with "no reactor running" there. Our work is blocking I/O (fs
    // scans, remove_dir_all) with no await points anyway.
    let ops_for_task = ops.inner().clone();
    spawn_op_thread(app, ops_for_task, op_id.clone(), move |sink, app, ops, op_id| {
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
    });

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
        let entries = project_repair::list_pending_with_status(
            &journals,
            &locks,
            JOURNAL_NAG_THRESHOLD_SECS,
        )
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
        let entries = project_repair::list_pending_with_status(
            &journals,
            &locks,
            JOURNAL_NAG_THRESHOLD_SECS,
        )
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
