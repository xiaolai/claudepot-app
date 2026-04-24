//! Session prune / slim / trash Tauri commands.
//!
//! The prune + slim operations are long-running and flow through the
//! `ops.rs` op-progress pipeline. `session_trash_*` is a
//! read/restore/empty surface over `claudepot_core::trash`.

use crate::ops::{
    emit_terminal, new_op_id, new_running_op, spawn_op_thread, OpKind, RunningOps,
};
use claudepot_core::paths;
use tauri::{AppHandle, State};

fn filter_from_dto(
    dto: crate::dto::PruneFilterDto,
) -> claudepot_core::session_prune::PruneFilter {
    claudepot_core::session_prune::PruneFilter {
        older_than: dto.older_than_secs.map(std::time::Duration::from_secs),
        larger_than: dto.larger_than_bytes,
        project: dto
            .project
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect(),
        has_error: dto.has_error,
        is_sidechain: dto.is_sidechain,
    }
}

fn slim_opts_from_dto(
    dto: crate::dto::SlimOptsDto,
) -> claudepot_core::session_slim::SlimOpts {
    claudepot_core::session_slim::SlimOpts {
        drop_tool_results_over_bytes: dto.drop_tool_results_over_bytes,
        exclude_tools: dto.exclude_tools,
        strip_images: dto.strip_images,
        strip_documents: dto.strip_documents,
    }
}

#[tauri::command]
pub async fn session_prune_plan(
    filter: crate::dto::PruneFilterDto,
) -> Result<crate::dto::PrunePlanDto, String> {
    let f = filter_from_dto(filter);
    let plan = claudepot_core::session_prune::plan_prune(&paths::claude_config_dir(), &f)
        .map_err(|e| format!("plan_prune: {e}"))?;
    Ok((&plan).into())
}

#[tauri::command]
pub async fn session_prune_start(
    filter: crate::dto::PruneFilterDto,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let f = filter_from_dto(filter);
    let plan = claudepot_core::session_prune::plan_prune(&paths::claude_config_dir(), &f)
        .map_err(|e| format!("plan_prune: {e}"))?;
    let op_id = new_op_id();
    ops.insert(new_running_op(&op_id, OpKind::SessionPrune, "", ""));
    let ops_c = ops.inner().clone();
    spawn_op_thread(app, ops_c, op_id.clone(), move |sink, app, ops, op_id| {
        let data_dir = paths::claudepot_data_dir();
        let res = claudepot_core::session_prune::execute_prune(&data_dir, &plan, &sink);
        let err = res.err().map(|e| e.to_string());
        emit_terminal(&app, &ops, &op_id, err);
    });
    Ok(op_id)
}

#[tauri::command]
pub async fn session_slim_plan(
    path: String,
    opts: crate::dto::SlimOptsDto,
) -> Result<crate::dto::SlimPlanDto, String> {
    let opts = slim_opts_from_dto(opts);
    let plan = claudepot_core::session_slim::plan_slim(std::path::Path::new(&path), &opts)
        .map_err(|e| format!("plan_slim: {e}"))?;
    Ok((&plan).into())
}

#[tauri::command]
pub async fn session_slim_start(
    path: String,
    opts: crate::dto::SlimOptsDto,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let opts = slim_opts_from_dto(opts);
    let path_buf = std::path::PathBuf::from(&path);
    let op_id = new_op_id();
    ops.insert(new_running_op(
        &op_id,
        OpKind::SessionSlim,
        path.clone(),
        path,
    ));
    let ops_c = ops.inner().clone();
    spawn_op_thread(app, ops_c, op_id.clone(), move |sink, app, ops, op_id| {
        let data_dir = paths::claudepot_data_dir();
        let res = claudepot_core::session_slim::execute_slim(
            &data_dir, &path_buf, &opts, &sink,
        );
        let err = res.err().map(|e| e.to_string());
        emit_terminal(&app, &ops, &op_id, err);
    });
    Ok(op_id)
}

#[tauri::command]
pub async fn session_slim_plan_all(
    filter: crate::dto::PruneFilterDto,
    opts: crate::dto::SlimOptsDto,
) -> Result<crate::dto::BulkSlimPlanDto, String> {
    let filter = filter_from_dto(filter);
    let opts = slim_opts_from_dto(opts);
    let config_dir = paths::claude_config_dir();
    let plan = claudepot_core::session_slim::plan_slim_all(&config_dir, &filter, &opts)
        .map_err(|e| format!("plan_slim_all: {e}"))?;
    Ok((&plan).into())
}

#[tauri::command]
pub async fn session_slim_start_all(
    filter: crate::dto::PruneFilterDto,
    opts: crate::dto::SlimOptsDto,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let filter = filter_from_dto(filter);
    let opts = slim_opts_from_dto(opts);
    let op_id = new_op_id();
    ops.insert(new_running_op(
        &op_id,
        OpKind::SessionSlim,
        "--all",
        "--all",
    ));
    let ops_c = ops.inner().clone();
    spawn_op_thread(app, ops_c, op_id.clone(), move |sink, app, ops, op_id| {
        let config_dir = paths::claude_config_dir();
        let data_dir = paths::claudepot_data_dir();
        let plan = match claudepot_core::session_slim::plan_slim_all(
            &config_dir,
            &filter,
            &opts,
        ) {
            Ok(p) => p,
            Err(e) => {
                emit_terminal(&app, &ops, &op_id, Some(e.to_string()));
                return;
            }
        };
        let report =
            claudepot_core::session_slim::execute_slim_all(&data_dir, &plan, &opts, &sink);
        // Propagate per-file failures to the UI. Planning errors
        // (rows that couldn't be scanned) and execute failures are
        // both reported — a partial success with any failures is not
        // a clean `complete`.
        let mut problems: Vec<String> = Vec::new();
        for (p, e) in &plan.failed_to_plan {
            problems.push(format!("plan {}: {e}", p.display()));
        }
        for (p, e) in &report.failed {
            problems.push(format!("exec {}: {e}", p.display()));
        }
        let err_msg = if problems.is_empty() {
            None
        } else {
            Some(format!(
                "bulk slim finished with {} success, {} skipped, {} failed:\n{}",
                report.succeeded.len(),
                report.skipped_live.len(),
                problems.len(),
                problems.join("\n"),
            ))
        };
        emit_terminal(&app, &ops, &op_id, err_msg);
    });
    Ok(op_id)
}

#[tauri::command]
pub async fn session_trash_list(
    older_than_secs: Option<u64>,
) -> Result<crate::dto::TrashListingDto, String> {
    let filter = claudepot_core::trash::TrashFilter {
        older_than: older_than_secs.map(std::time::Duration::from_secs),
        kind: None,
    };
    let listing = claudepot_core::trash::list(&paths::claudepot_data_dir(), filter)
        .map_err(|e| format!("trash list: {e}"))?;
    Ok((&listing).into())
}

#[tauri::command]
pub async fn session_trash_restore(
    entry_id: String,
    override_cwd: Option<String>,
) -> Result<String, String> {
    let cwd = override_cwd.as_deref().map(std::path::Path::new);
    let restored =
        claudepot_core::trash::restore(&paths::claudepot_data_dir(), &entry_id, cwd)
            .map_err(|e| format!("trash restore: {e}"))?;
    Ok(restored.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn session_trash_empty(older_than_secs: Option<u64>) -> Result<u64, String> {
    let filter = claudepot_core::trash::TrashFilter {
        older_than: older_than_secs.map(std::time::Duration::from_secs),
        kind: None,
    };
    claudepot_core::trash::empty(&paths::claudepot_data_dir(), filter)
        .map_err(|e| format!("trash empty: {e}"))
}
