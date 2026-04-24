//! Tauri commands for long-running project-move + repair-journal ops.
//!
//! Every long-running verb (`project_move_start`, `repair_resume_start`,
//! `repair_rollback_start`) runs on a dedicated OS thread and reports
//! via the `op-progress::<op_id>` channel. The synchronous maintenance
//! surface (`repair_abandon`, `repair_break_lock`, `repair_gc`, etc.)
//! lives here too — one module per logical concern.

use crate::commands_project::{claudepot_home_dirs, JOURNAL_NAG_THRESHOLD_SECS};
use crate::dto::MoveArgsDto;
use crate::ops::{
    emit_terminal, new_op_id, new_running_op, spawn_op_thread, MoveResultSummary, OpKind,
    RunningOpInfo, RunningOps,
};
use claudepot_core::paths;
use claudepot_core::project;
use claudepot_core::project_repair;
use tauri::{AppHandle, State};

#[derive(serde::Serialize)]
pub struct BreakLockOutcomeDto {
    pub prior_pid: u32,
    pub prior_hostname: String,
    pub prior_started: String,
    pub audit_path: String,
}

#[derive(serde::Serialize)]
pub struct GcOutcomeDto {
    pub removed_journals: usize,
    pub removed_snapshots: usize,
    pub bytes_freed: u64,
    pub would_remove: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbandonedCleanupEntryDto {
    pub id: String,
    pub journal_path: String,
    pub sidecar_path: String,
    pub referenced_snapshots: Vec<String>,
    pub bytes: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbandonedCleanupReportDto {
    pub entries: Vec<AbandonedCleanupEntryDto>,
    pub removed_journals: usize,
    pub removed_snapshots: usize,
    pub bytes_freed: u64,
}

impl From<claudepot_core::project_repair::AbandonedCleanupReport>
    for AbandonedCleanupReportDto
{
    fn from(r: claudepot_core::project_repair::AbandonedCleanupReport) -> Self {
        Self {
            entries: r
                .entries
                .into_iter()
                .map(|e| AbandonedCleanupEntryDto {
                    id: e.id,
                    journal_path: e.journal_path.to_string_lossy().to_string(),
                    sidecar_path: e.sidecar_path.to_string_lossy().to_string(),
                    referenced_snapshots: e
                        .referenced_snapshots
                        .into_iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                    bytes: e.bytes,
                })
                .collect(),
            removed_journals: r.removed_journals,
            removed_snapshots: r.removed_snapshots,
            bytes_freed: r.bytes_freed,
        }
    }
}

fn find_journal(id: &str) -> Result<claudepot_core::project_repair::JournalEntry, String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )
    .map_err(|e| format!("repair list failed: {e}"))?;
    entries
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("no journal with id '{id}'"))
}

fn spawn_repair_op(
    app: AppHandle,
    ops: RunningOps,
    kind: OpKind,
    entry: claudepot_core::project_repair::JournalEntry,
) -> String {
    let op_id = new_op_id();
    ops.insert(new_running_op(
        &op_id,
        kind,
        entry.journal.old_path.clone(),
        entry.journal.new_path.clone(),
    ));

    let old_path_for_task = entry.journal.old_path.clone();
    // See `project_clean_start` for why this is an OS thread.
    spawn_op_thread(app, ops, op_id.clone(), move |sink, app, ops, op_id| {
        let cfg = paths::claude_config_dir();
        let claude_json = dirs::home_dir().map(|h| h.join(".claude.json"));
        let snaps = Some(paths::claudepot_repair_dir().join("snapshots"));
        let result = match kind {
            OpKind::RepairResume => {
                project_repair::resume(&entry, cfg, claude_json, snaps, &sink)
            }
            OpKind::RepairRollback => {
                project_repair::rollback(&entry, cfg, claude_json, snaps, &sink)
            }
            OpKind::MoveProject
            | OpKind::CleanProjects
            | OpKind::SessionPrune
            | OpKind::SessionSlim
            | OpKind::SessionShare => {
                unreachable!("wrong spawn path")
            }
        };
        finalize_op(&app, &ops, &op_id, &old_path_for_task, result);
    });

    op_id
}

/// Shared finalizer for every op spawn: on Ok, stash the structured
/// result so the UI can render snapshot paths; on Err, look up the
/// newest journal whose `old_path` matches so the UI can deep-link
/// "Open Repair" at the exact failed entry. Emits the terminal event
/// either way.
fn finalize_op(
    app: &AppHandle,
    ops: &RunningOps,
    op_id: &str,
    old_path: &str,
    result: Result<claudepot_core::project::MoveResult, claudepot_core::error::ProjectError>,
) {
    match result {
        Ok(mv) => {
            let summary = MoveResultSummary::from_core(&mv);
            ops.update(op_id, |op| op.move_result = Some(summary));
            emit_terminal(app, ops, op_id, None);
        }
        Err(e) => {
            let msg = e.to_string();
            let journal_id = newest_journal_id_for(old_path);
            ops.update(op_id, |op| op.failed_journal_id = journal_id.clone());
            emit_terminal(app, ops, op_id, Some(msg));
        }
    }
}

/// Best-effort: scan the journals dir for the most recent journal
/// whose old_path matches, and return its id. None on lookup failure —
/// the UI falls back to "Open Repair" without a specific target.
fn newest_journal_id_for(old_path: &str) -> Option<String> {
    let (journals, locks, _snaps) = claudepot_home_dirs();
    let entries = project_repair::list_pending_with_status(
        &journals,
        &locks,
        JOURNAL_NAG_THRESHOLD_SECS,
    )
    .ok()?;
    entries
        .into_iter()
        .filter(|e| e.journal.old_path == old_path)
        .max_by_key(|e| e.journal.started_unix_secs)
        .map(|e| e.id)
}

#[tauri::command]
pub async fn project_move_start(
    args: MoveArgsDto,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let cfg = paths::claude_config_dir();
    let claude_json = dirs::home_dir().map(|h| h.join(".claude.json"));
    let repair_root = paths::claudepot_repair_dir();
    let snaps = Some(repair_root.join("snapshots"));
    // Defensive: ignore `dry_run` from the DTO — this endpoint always
    // actually executes. Callers that want dry-run use
    // `project_move_dry_run` instead.
    let core_args = project::MoveArgs {
        old_path: args.old_path.clone().into(),
        new_path: args.new_path.clone().into(),
        config_dir: cfg,
        claude_json_path: claude_json,
        snapshots_dir: snaps,
        no_move: args.no_move,
        merge: args.merge,
        overwrite: args.overwrite,
        force: args.force,
        dry_run: false,
        ignore_pending_journals: args.ignore_pending_journals,
        claudepot_state_dir: Some(repair_root),
    };

    let op_id = new_op_id();
    ops.insert(new_running_op(
        &op_id,
        OpKind::MoveProject,
        args.old_path.clone(),
        args.new_path.clone(),
    ));

    let ops_for_task = ops.inner().clone();
    let old_path_for_task = args.old_path;
    // See `project_clean_start` for why this is an OS thread.
    spawn_op_thread(app, ops_for_task, op_id.clone(), move |sink, app, ops, op_id| {
        let result = project::move_project(&core_args, &sink);
        finalize_op(&app, &ops, &op_id, &old_path_for_task, result);
    });

    Ok(op_id)
}

#[tauri::command]
pub async fn project_move_status(
    op_id: String,
    ops: State<'_, RunningOps>,
) -> Result<Option<RunningOpInfo>, String> {
    Ok(ops.get(&op_id))
}

#[tauri::command]
pub async fn repair_resume_start(
    id: String,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let entry = find_journal(&id)?;
    Ok(spawn_repair_op(
        app,
        ops.inner().clone(),
        OpKind::RepairResume,
        entry,
    ))
}

#[tauri::command]
pub async fn repair_rollback_start(
    id: String,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let entry = find_journal(&id)?;
    Ok(spawn_repair_op(
        app,
        ops.inner().clone(),
        OpKind::RepairRollback,
        entry,
    ))
}

#[tauri::command]
pub async fn repair_abandon(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = find_journal(&id)?;
        project_repair::abandon(&entry).map_err(|e| format!("abandon failed: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("abandon join: {e}"))?
}

#[tauri::command]
pub async fn repair_break_lock(path: String) -> Result<BreakLockOutcomeDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (journals, locks, _snaps) = claudepot_home_dirs();
        let lock_path = project_repair::resolve_lock_file(&locks, &path)
            .ok_or_else(|| format!("no lock file found for '{path}'"))?;
        let broken = project_repair::break_lock_with_audit(&lock_path, &journals)
            .map_err(|e| format!("break-lock failed: {e}"))?;
        Ok(BreakLockOutcomeDto {
            prior_pid: broken.prior.pid,
            prior_hostname: broken.prior.hostname,
            prior_started: broken.prior.start_iso8601,
            audit_path: broken.audit_path.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("break-lock join: {e}"))?
}

/// Preview every abandoned journal's artifacts (journal + sidecar +
/// referenced snapshots) without deleting anything. Used to populate
/// the "Clean recovery artifacts" card in Maintenance.
#[tauri::command]
pub async fn repair_preview_abandoned() -> Result<AbandonedCleanupReportDto, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (journals, _locks, _snaps) = claudepot_home_dirs();
        let result = project_repair::preview_abandoned(&journals)
            .map_err(|e| format!("preview_abandoned failed: {e}"))?;
        Ok(result.into())
    })
    .await
    .map_err(|e| format!("preview_abandoned join: {e}"))?
}

/// Remove every abandoned journal + sidecar + its referenced
/// snapshots. Safer than `repair_gc(0, false)`: only touches files
/// linked to an abandoned entry; unreferenced or recent snapshots
/// from successful ops are left alone.
#[tauri::command]
pub async fn repair_cleanup_abandoned() -> Result<AbandonedCleanupReportDto, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (journals, _locks, _snaps) = claudepot_home_dirs();
        let result = project_repair::cleanup_abandoned(&journals)
            .map_err(|e| format!("cleanup_abandoned failed: {e}"))?;
        Ok(result.into())
    })
    .await
    .map_err(|e| format!("cleanup_abandoned join: {e}"))?
}

#[tauri::command]
pub async fn repair_gc(older_than_days: u64, dry_run: bool) -> Result<GcOutcomeDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (journals, _locks, snapshots) = claudepot_home_dirs();
        let result = project_repair::gc(&journals, &snapshots, older_than_days, dry_run)
            .map_err(|e| format!("gc failed: {e}"))?;
        Ok(GcOutcomeDto {
            removed_journals: result.removed_journals,
            removed_snapshots: result.removed_snapshots,
            bytes_freed: result.bytes_freed,
            would_remove: result
                .would_remove
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        })
    })
    .await
    .map_err(|e| format!("gc join: {e}"))?
}

/// Snapshot of currently-tracked ops. UI's RunningOpStrip polls this
/// as a backstop if events drop.
#[tauri::command]
pub async fn running_ops_list(ops: State<'_, RunningOps>) -> Result<Vec<RunningOpInfo>, String> {
    Ok(ops.list())
}
