//! Tauri commands for session move / orphan adoption / discard.
//!
//! All business logic lives in `claudepot_core::session_move`; this
//! module is a DTO shim + validation at the edge.
//!
//! Rejections cross as `ErrorDto`. The per-verb prefixes this module
//! used to compose (`move failed: …`, `adopt failed: …`, `discard
//! failed: …`) are gone — `MoveSessionError` names the condition, the
//! UI names the verb, and the four prefixes were about to be four
//! untranslatable sentences. See `crate::dto_error`.

use crate::dto_error::{codes, ErrorDto};
use crate::ops::{
    emit_terminal, new_op_id, new_running_op, spawn_op_thread, MoveSessionReportSummary, OpKind,
    RunningOpInfo, RunningOps,
};
use claudepot_core::paths;
use claudepot_core::session_move;
use serde_json::json;
use tauri::{AppHandle, State};
use uuid::Uuid;

/// A renderer-supplied session uuid that did not parse. Two commands
/// share the check, so they share one spelling.
fn invalid_session_id(e: uuid::Error) -> ErrorDto {
    ErrorDto::with_params(
        codes::SESSION_MOVE_INVALID_SESSION_ID,
        json!({ "detail": e.to_string() }),
        format!("invalid session id: {e}"),
    )
}

#[tauri::command]
pub async fn session_list_orphans() -> Result<Vec<crate::dto::OrphanedProjectDto>, ErrorDto> {
    // Iterates every project slug, reads each slug's first session
    // JSONL header to recover `cwd`, then stats that cwd. O(N projects)
    // syscalls; runs in parallel with `project_list` on every Projects
    // refresh. Off the Tokio worker.
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = paths::claude_config_dir();
        let orphans = claudepot_core::session_move::detect_orphaned_projects(&cfg)?;
        Ok(orphans
            .iter()
            .map(crate::dto::OrphanedProjectDto::from)
            .collect())
    })
    .await
    .map_err(ErrorDto::task_join)?
}

// The `.claude.json` location is owned by `claudepot_core::paths::
// claude_json_path` — the one accessor the CLI and this Tauri layer
// share, so the two surfaces can't drift.
use claudepot_core::paths::claude_json_path;

#[tauri::command]
pub async fn session_move(
    session_id: String,
    from_cwd: String,
    to_cwd: String,
    force_live: bool,
    force_conflict: bool,
    cleanup_source: bool,
) -> Result<crate::dto::MoveSessionReportDto, ErrorDto> {
    let sid = Uuid::parse_str(&session_id).map_err(invalid_session_id)?;
    // `move_session` rewrites transcript files, mutates `.claude.json`,
    // and can touch the live-runtime lockfile. Blocking I/O with no
    // await points — push it off the Tokio worker.
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let opts = claudepot_core::session_move::MoveSessionOpts {
            force_live_session: force_live,
            force_sync_conflict: force_conflict,
            cleanup_source_if_empty: cleanup_source,
            claude_json_path: claude_json_path(),
        };
        let report = claudepot_core::session_move::move_session(
            &cfg,
            sid,
            std::path::Path::new(&from_cwd),
            std::path::Path::new(&to_cwd),
            opts,
        )?;
        Ok(crate::dto::MoveSessionReportDto::from(&report))
    })
    .await
    .map_err(ErrorDto::task_join)?
}

#[tauri::command]
pub async fn session_adopt_orphan(
    slug: String,
    target_cwd: String,
) -> Result<crate::dto::AdoptReportDto, ErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let target = std::path::Path::new(&target_cwd);
        if !target.is_dir() {
            return Err(ErrorDto::with_params(
                codes::SESSION_MOVE_TARGET_CWD_MISSING,
                json!({ "path": target_cwd }),
                format!("target cwd does not exist: {target_cwd}"),
            ));
        }
        let report = claudepot_core::session_move::adopt_orphan_project(
            &cfg,
            &slug,
            target,
            claude_json_path(),
        )?;
        Ok(crate::dto::AdoptReportDto::from(&report))
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Move an orphan project slug dir to the OS Trash. The user can restore
/// it from Trash if they change their mind; the guard that the slug is
/// valid + resolves to a real dir happens in core.
#[tauri::command]
pub async fn session_discard_orphan(
    slug: String,
) -> Result<crate::dto::DiscardReportDto, ErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let report = claudepot_core::session_move::discard_orphan_project(&cfg, &slug)?;
        Ok(crate::dto::DiscardReportDto::from(&report))
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Start an async session move. Returns the op_id immediately; the
/// frontend subscribes to `op-progress::<op_id>` for phase events
/// (S1..S5) and polls [`session_move_status`] for the terminal
/// [`MoveSessionReportSummary`].
///
/// Mirrors the shape of `project_move_start` in `commands_repair.rs`.
/// The legacy synchronous [`session_move`] command stays registered
/// for now — see migration plan in
/// `dev-docs/reports/codex-mini-audit-fix-deferred-design-2026-04-25.md`
/// (A-2 step 7).
// Tauri command argument names must match the renderer's invoke
// payload, so collapsing into a struct would silently rename the
// IPC contract. Keep the explicit list and silence the lint.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn session_move_start(
    session_id: String,
    from_cwd: String,
    to_cwd: String,
    force_live: bool,
    force_conflict: bool,
    cleanup_source: bool,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, ErrorDto> {
    let sid = Uuid::parse_str(&session_id).map_err(invalid_session_id)?;

    let op_id = new_op_id();
    ops.insert(new_running_op(
        &op_id,
        OpKind::SessionMove,
        from_cwd.clone(),
        to_cwd.clone(),
    ));

    let ops_for_task = ops.inner().clone();
    let from_cwd_for_task = from_cwd;
    let to_cwd_for_task = to_cwd;
    spawn_op_thread(
        app,
        ops_for_task,
        op_id.clone(),
        move |sink, app, ops, op_id| {
            let cfg = paths::claude_config_dir();
            let opts = session_move::MoveSessionOpts {
                force_live_session: force_live,
                force_sync_conflict: force_conflict,
                cleanup_source_if_empty: cleanup_source,
                claude_json_path: claude_json_path(),
            };
            let result = session_move::move_session_with_progress(
                &cfg,
                sid,
                std::path::Path::new(&from_cwd_for_task),
                std::path::Path::new(&to_cwd_for_task),
                opts,
                &sink,
            );
            match result {
                Ok(report) => {
                    let summary = MoveSessionReportSummary::from(&report);
                    ops.update(&op_id, |op| {
                        op.session_move_result = Some(summary);
                    });
                    emit_terminal(&app, &ops, &op_id, None);
                }
                Err(e) => {
                    emit_terminal(&app, &ops, &op_id, Some(e.to_string()));
                }
            }
        },
    );

    Ok(op_id)
}

/// Snapshot of an in-flight or just-finished session-move op.
/// Returns `None` after the grace window (currently 5 s).
#[tauri::command]
pub async fn session_move_status(
    op_id: String,
    ops: State<'_, RunningOps>,
) -> Result<Option<RunningOpInfo>, ErrorDto> {
    Ok(ops.get(&op_id))
}

#[cfg(test)]
mod tests {
    /// `session_move_start` integration test stub. The real
    /// `tauri::AppHandle` + `State<RunningOps>` plumbing only exists
    /// inside a built Tauri app; `#[ignore]` per
    /// `.claude/rules/rust-conventions.md`.
    #[test]
    #[ignore]
    fn session_move_start_returns_op_id() {
        // Exercised manually on the test machine via:
        //   cargo build -p claudepot-cli && pnpm tauri dev
        // and watching the network panel for an "op-progress::<id>"
        // channel after Sessions → Move.
    }
}
