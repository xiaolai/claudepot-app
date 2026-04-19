//! Background-operation plumbing for long-running Tauri commands.
//!
//! Each start-style command (`repair_resume_start`, `repair_rollback_start`,
//! and later `project_move_start`) spawns a tokio task that calls into
//! `claudepot-core`, emits per-phase events on a per-operation channel,
//! and records its lifecycle in [`RunningOps`] so the UI can poll status
//! as a backstop if events drop.
//!
//! Spec: plan §2.4 (op-scoped events), §5.3 (channel discipline).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use claudepot_core::project_progress::{PhaseStatus, ProgressSink};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// What kind of long-running op is this? Used by the UI to render
/// the right verb in the running-op strip.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    RepairResume,
    RepairRollback,
    MoveProject,
    CleanProjects,
}

/// Post-op summary surfaced to the UI on success, so we can render
/// snapshot paths (plan §7.7 H6) and other structured outcomes
/// without parsing the detail string.
#[derive(Debug, Clone, Serialize, Default)]
pub struct MoveResultSummary {
    pub actual_dir_moved: bool,
    pub cc_dir_renamed: bool,
    pub jsonl_files_scanned: usize,
    pub jsonl_files_modified: usize,
    pub config_had_collision: bool,
    pub config_snapshot_path: Option<String>,
    pub memory_dir_moved: bool,
    pub warnings: Vec<String>,
}

impl MoveResultSummary {
    pub fn from_core(r: &claudepot_core::project::MoveResult) -> Self {
        Self {
            actual_dir_moved: r.actual_dir_moved,
            cc_dir_renamed: r.cc_dir_renamed,
            jsonl_files_scanned: r.jsonl_files_scanned,
            jsonl_files_modified: r.jsonl_files_modified,
            config_had_collision: r.config_had_collision,
            config_snapshot_path: r
                .config_snapshot_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            memory_dir_moved: r.memory_dir_moved,
            warnings: r.warnings.clone(),
        }
    }
}

/// Overall status of a running op, independent of phase.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    /// Task is still running — no terminal event yet.
    Running,
    /// Core returned Ok. UI may dismiss the modal after a grace period.
    Complete,
    /// Core returned Err. Detail in `last_error`.
    Error,
}

/// Per-op event payload emitted on `op-progress::<op_id>` channels.
/// Shape mirrors the TS `OperationProgressEvent` in the plan §2.4.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub op_id: String,
    pub phase: String,
    /// "running" | "complete" | "error" — a phase status, not the overall
    /// op status. Subtle: `status=complete` on phase P9 is still just a
    /// phase-level signal; the overall terminal event has phase="op" and
    /// status=complete/error.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Snapshot of a running op, returned by `project_move_status` /
/// `repair_op_status` polling calls. Cheap — pure map lookup.
#[derive(Debug, Clone, Serialize)]
pub struct RunningOpInfo {
    pub op_id: String,
    pub kind: OpKind,
    pub old_path: String,
    pub new_path: String,
    pub current_phase: Option<String>,
    pub sub_progress: Option<(usize, usize)>,
    pub status: OpStatus,
    pub started_unix_secs: u64,
    pub last_error: Option<String>,
    /// Populated on successful MoveProject / RepairResume / RepairRollback.
    /// None while running or on error.
    pub move_result: Option<MoveResultSummary>,
    /// Populated on successful CleanProjects. Carries the structured
    /// CleanResult so the modal can render counters + snapshot paths
    /// without a separate status poll.
    pub clean_result: Option<CleanResultSummary>,
    /// Journal id of a failed move, so the UI can deep-link "Open Repair"
    /// and surface this exact entry. Populated opportunistically on error
    /// — matches the newest journal whose `old_path == old_path` (the
    /// journal is created during the move, so it will exist when we look).
    pub failed_journal_id: Option<String>,
}

/// Mirror of `claudepot_core::project_types::CleanResult` for JSON
/// emission from the Tauri layer. Stored on `RunningOpInfo` so the
/// terminal status poll returns the complete result, not just a
/// success flag.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CleanResultSummary {
    pub orphans_found: usize,
    pub orphans_removed: usize,
    pub orphans_skipped_live: usize,
    pub unreachable_skipped: usize,
    pub bytes_freed: u64,
    pub claude_json_entries_removed: usize,
    pub history_lines_removed: usize,
    pub claudepot_artifacts_removed: usize,
    pub snapshot_paths: Vec<String>,
    pub protected_paths_skipped: usize,
}

impl CleanResultSummary {
    pub fn from_core(r: &claudepot_core::project_types::CleanResult) -> Self {
        Self {
            orphans_found: r.orphans_found,
            orphans_removed: r.orphans_removed,
            orphans_skipped_live: r.orphans_skipped_live,
            unreachable_skipped: r.unreachable_skipped,
            bytes_freed: r.bytes_freed,
            claude_json_entries_removed: r.claude_json_entries_removed,
            history_lines_removed: r.history_lines_removed,
            claudepot_artifacts_removed: r.claudepot_artifacts_removed,
            snapshot_paths: r
                .snapshot_paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            protected_paths_skipped: r.protected_paths_skipped,
        }
    }
}

/// Shared map of live ops. Wrapped in `Arc<Mutex<_>>` so commands and
/// spawned tasks can both mutate. Completed ops linger for a short
/// grace period so a slow listener still sees the terminal event.
#[derive(Default, Clone)]
pub struct RunningOps {
    inner: Arc<Mutex<HashMap<String, RunningOpInfo>>>,
}

impl RunningOps {
    pub fn new() -> Self {
        Self::default()
    }

    /// Low-audit guard: recover from a poisoned Mutex rather than
    /// panicking. If an earlier panic poisoned the map, the ops
    /// pipeline would propagate the panic forever — this turns a
    /// single transient panic into a logged-and-continue condition.
    fn guard(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, RunningOpInfo>> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::error!("RunningOps mutex was poisoned; recovering");
                poisoned.into_inner()
            }
        }
    }

    pub fn insert(&self, op: RunningOpInfo) {
        self.guard().insert(op.op_id.clone(), op);
    }

    pub fn get(&self, op_id: &str) -> Option<RunningOpInfo> {
        self.guard().get(op_id).cloned()
    }

    pub fn list(&self) -> Vec<RunningOpInfo> {
        self.guard().values().cloned().collect()
    }

    pub fn update<F: FnOnce(&mut RunningOpInfo)>(&self, op_id: &str, f: F) {
        if let Some(op) = self.guard().get_mut(op_id) {
            f(op);
        }
    }

    /// Remove an op from the map after the grace window — keeps the
    /// terminal event visible to a slow listener. Call after emitting
    /// the op's final complete/error event.
    ///
    /// Uses `std::thread::spawn` rather than `tokio::spawn` so the
    /// helper is safe to call from commands that run outside a tokio
    /// runtime (plain sync `#[tauri::command]` handlers dispatched on
    /// Tauri's own thread pool).
    pub fn remove_after_grace(&self, op_id: String) {
        let map = self.inner.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            if let Ok(mut guard) = map.lock() {
                guard.remove(&op_id);
            }
        });
    }
}

/// ProgressSink that emits events on `op-progress::<op_id>` channels
/// AND mirrors the latest phase / sub_progress into the shared
/// [`RunningOps`] map so polling calls see consistent state.
pub struct TauriProgressSink {
    pub app: AppHandle,
    pub op_id: String,
    pub ops: RunningOps,
}

impl TauriProgressSink {
    fn channel(&self) -> String {
        format!("op-progress::{}", self.op_id)
    }
}

impl ProgressSink for TauriProgressSink {
    fn phase(&self, phase: &str, status: PhaseStatus) {
        let (status_str, detail) = match &status {
            PhaseStatus::Running => ("running", None),
            PhaseStatus::Complete => ("complete", None),
            PhaseStatus::Error(msg) => ("error", Some(msg.clone())),
        };
        let payload = ProgressEvent {
            op_id: self.op_id.clone(),
            phase: phase.to_string(),
            status: status_str.to_string(),
            done: None,
            total: None,
            detail: detail.clone(),
        };
        let _ = self.app.emit(&self.channel(), payload);
        // Only mirror per-phase updates when the phase actually
        // advanced — Running events are cheap but we can skip them for
        // now since core emits Complete per phase.
        self.ops.update(&self.op_id, |op| {
            op.current_phase = Some(phase.to_string());
            if matches!(status, PhaseStatus::Error(_)) {
                op.status = OpStatus::Error;
                op.last_error = detail;
            }
        });
    }

    fn sub_progress(&self, phase: &str, done: usize, total: usize) {
        let payload = ProgressEvent {
            op_id: self.op_id.clone(),
            phase: phase.to_string(),
            status: "running".to_string(),
            done: Some(done),
            total: Some(total),
            detail: None,
        };
        let _ = self.app.emit(&self.channel(), payload);
        self.ops.update(&self.op_id, |op| {
            op.sub_progress = Some((done, total));
        });
    }
}

/// Emit the terminal event for an op. Call once, after the core
/// function returns. `error` should be None on success.
pub fn emit_terminal(
    app: &AppHandle,
    ops: &RunningOps,
    op_id: &str,
    error: Option<String>,
) {
    let status_str = if error.is_some() { "error" } else { "complete" };
    let payload = ProgressEvent {
        op_id: op_id.to_string(),
        phase: "op".to_string(),
        status: status_str.to_string(),
        done: None,
        total: None,
        detail: error.clone(),
    };
    let channel = format!("op-progress::{op_id}");
    let _ = app.emit(&channel, payload);
    ops.update(op_id, |op| {
        op.status = if error.is_some() {
            OpStatus::Error
        } else {
            OpStatus::Complete
        };
        if let Some(msg) = error {
            op.last_error = Some(msg);
        }
    });
    ops.remove_after_grace(op_id.to_string());
}

/// Current unix seconds — helper to avoid pulling `SystemTime`
/// boilerplate into every caller.
pub fn now_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Convenience — generate a fresh op id. UUID v4 is plenty unique for
/// concurrent ops on a single machine.
pub fn new_op_id() -> String {
    format!("op-{}", uuid::Uuid::new_v4())
}
