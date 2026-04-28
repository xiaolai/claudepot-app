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
    SessionPrune,
    SessionSlim,
    /// Gist upload — tracked separately from SessionSlim so the UI
    /// label reads "Sharing" rather than "Slimming".
    SessionShare,
    /// Single-session move: rewrites primary JSONL `cwd`, sidecar dirs,
    /// history.jsonl, .claude.json pointers, source dir.
    SessionMove,
    /// Existing-account re-login: spawns `claude auth login` and re-imports
    /// the resulting blob into the slot. Long-running because OAuth blocks
    /// on the browser.
    AccountLogin,
    /// Browser-OAuth onboarding for a fresh account. Same shape as
    /// AccountLogin but the terminal payload carries a new account uuid.
    AccountRegister,
    /// Per-account `/profile` reconcile loop. Carries per-account events
    /// alongside the standard phase channel.
    VerifyAll,
    /// Manual "Run Now" of an automation. Spawns the helper shim
    /// out-of-band of the OS scheduler and emits phase events
    /// (prepare → spawn → record → done).
    AutomationRun,
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
    /// Populated on successful SessionMove. None while running or on
    /// error. Mirrors the shape of `MoveSessionReportDto` so the
    /// Sessions modal can render the same summary as the legacy
    /// `session_move` IPC.
    pub session_move_result: Option<MoveSessionReportSummary>,
    /// Populated by login ops as they progress through `LoginPhase`s.
    /// Mirrors `current_phase` (string) as a typed field — useful when
    /// the polling backstop kicks in and the UI wants to render a
    /// phase glyph without re-deriving from the channel name.
    pub login_phase: Option<LoginPhaseKind>,
    /// Populated on terminal events for VerifyAll ops. Counts only —
    /// per-account detail comes through `op-progress::<op_id>` events.
    pub verify_results: Option<VerifyResultSummary>,
}

/// Mirror of [`claudepot_core::services::account_service::LoginPhase`] for
/// JSON emission. Same six variants; carried on `RunningOpInfo` so the
/// UI can render a typed phase glyph without parsing strings.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginPhaseKind {
    Spawning,
    WaitingForBrowser,
    ReadingBlob,
    FetchingProfile,
    VerifyingIdentity,
    Persisting,
}

impl From<claudepot_core::services::account_service::LoginPhase> for LoginPhaseKind {
    fn from(p: claudepot_core::services::account_service::LoginPhase) -> Self {
        use claudepot_core::services::account_service::LoginPhase as LP;
        match p {
            LP::Spawning => Self::Spawning,
            LP::WaitingForBrowser => Self::WaitingForBrowser,
            LP::ReadingBlob => Self::ReadingBlob,
            LP::FetchingProfile => Self::FetchingProfile,
            LP::VerifyingIdentity => Self::VerifyingIdentity,
            LP::Persisting => Self::Persisting,
        }
    }
}

impl LoginPhaseKind {
    /// Phase id stable contract: matches the strings the Tauri sink
    /// emits on `op-progress::<op_id>`. Frontend reads by these ids
    /// to flip phase rows in the modal.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Spawning => "spawning",
            Self::WaitingForBrowser => "waiting_for_browser",
            Self::ReadingBlob => "reading_blob",
            Self::FetchingProfile => "fetching_profile",
            Self::VerifyingIdentity => "verifying_identity",
            Self::Persisting => "persisting",
        }
    }
}

/// Counters bundled at the end of a `verify_all` op so the UI can render
/// a one-line summary in the running-op strip without re-aggregating
/// per-account events.
#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyResultSummary {
    pub total: usize,
    pub ok: usize,
    pub drift: usize,
    pub rejected: usize,
    pub network_error: usize,
}

/// Per-account event emitted on `op-progress::<op_id>` for VerifyAll ops.
/// Carries the typed payload that the original `VerifyEvent::Account`
/// produced — sibling to `ProgressEvent`, NOT pipe-delimited into
/// `ProgressEvent.detail`.
///
/// The frontend listens for both `ProgressEvent` (overall phase advance,
/// terminal events) and `VerifyAccountEvent` (per-account row updates).
#[derive(Debug, Clone, Serialize)]
pub struct VerifyAccountEvent {
    pub op_id: String,
    /// Always `"verify_account"` — distinguishes this payload from
    /// `ProgressEvent` on the shared channel.
    pub kind: &'static str,
    pub uuid: String,
    pub email: String,
    pub idx: usize,
    pub total: usize,
    /// "ok" | "drift" | "rejected" | "network_error"
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Mirror of `claudepot_core::session_move::MoveSessionReport` for
/// JSON emission. Same shape (camelCase) as
/// [`crate::dto_session_move::MoveSessionReportDto`] so the frontend
/// can reuse `MoveSessionReport` regardless of which surface fed it.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MoveSessionReportSummary {
    pub session_id: Option<String>,
    pub from_slug: String,
    pub to_slug: String,
    pub jsonl_lines_rewritten: usize,
    pub subagent_files_moved: usize,
    pub remote_agent_files_moved: usize,
    pub history_entries_moved: usize,
    pub history_entries_unmapped: usize,
    pub claude_json_pointers_cleared: u8,
    pub source_dir_removed: bool,
}

impl MoveSessionReportSummary {
    pub fn from_core(r: &claudepot_core::session_move::MoveSessionReport) -> Self {
        Self {
            session_id: r.session_id.map(|s| s.to_string()),
            from_slug: r.from_slug.clone(),
            to_slug: r.to_slug.clone(),
            jsonl_lines_rewritten: r.jsonl_lines_rewritten,
            subagent_files_moved: r.subagent_files_moved,
            remote_agent_files_moved: r.remote_agent_files_moved,
            history_entries_moved: r.history_entries_moved,
            history_entries_unmapped: r.history_entries_unmapped,
            claude_json_pointers_cleared: r.claude_json_pointers_cleared,
            source_dir_removed: r.source_dir_removed,
        }
    }
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

/// Build a fresh `RunningOpInfo` for an op that has just been created
/// and not yet started any phase. Populates the identity fields
/// (`op_id`, `kind`, `old_path`, `new_path`) and seeds every progress /
/// result field with its "nothing happened yet" default.
///
/// Every start-style command used to construct this by hand with 11
/// fields, 8 of which were identical — this helper is the point of
/// gravity so adding a new field doesn't drift across call sites.
pub fn new_running_op(
    op_id: impl Into<String>,
    kind: OpKind,
    old_path: impl Into<String>,
    new_path: impl Into<String>,
) -> RunningOpInfo {
    RunningOpInfo {
        op_id: op_id.into(),
        kind,
        old_path: old_path.into(),
        new_path: new_path.into(),
        current_phase: None,
        sub_progress: None,
        status: OpStatus::Running,
        started_unix_secs: now_unix_secs(),
        last_error: None,
        move_result: None,
        clean_result: None,
        failed_journal_id: None,
        session_move_result: None,
        login_phase: None,
        verify_results: None,
    }
}

/// `LoginProgressSink` adapter — emits each `LoginPhase` transition as
/// a `ProgressEvent` on `op-progress::<op_id>` and mirrors the latest
/// phase into the shared [`RunningOps`] map. Drop-in for the core
/// `register_from_browser_with_progress` / `login_and_reimport_with_progress`
/// surfaces.
pub struct TauriLoginProgressSink {
    pub app: AppHandle,
    pub op_id: String,
    pub ops: RunningOps,
}

impl TauriLoginProgressSink {
    fn channel(&self) -> String {
        format!("op-progress::{}", self.op_id)
    }
}

impl claudepot_core::services::account_service::LoginProgressSink for TauriLoginProgressSink {
    fn phase(&self, phase: claudepot_core::services::account_service::LoginPhase) {
        let kind = LoginPhaseKind::from(phase);
        let phase_str = kind.as_str().to_string();
        let payload = ProgressEvent {
            op_id: self.op_id.clone(),
            phase: phase_str.clone(),
            status: "running".to_string(),
            done: None,
            total: None,
            detail: None,
        };
        let _ = self.app.emit(&self.channel(), payload);
        self.ops.update(&self.op_id, |op| {
            op.current_phase = Some(phase_str);
            op.login_phase = Some(kind);
        });
    }

    fn error(&self, phase: claudepot_core::services::account_service::LoginPhase, msg: &str) {
        let kind = LoginPhaseKind::from(phase);
        let phase_str = kind.as_str().to_string();
        let payload = ProgressEvent {
            op_id: self.op_id.clone(),
            phase: phase_str.clone(),
            status: "error".to_string(),
            done: None,
            total: None,
            detail: Some(msg.to_string()),
        };
        let _ = self.app.emit(&self.channel(), payload);
        // Don't flip the overall op status here — `emit_terminal` is the
        // authoritative terminal hook. The phase-level error is informative
        // only; the op's terminal event stays the single source of truth.
        self.ops.update(&self.op_id, |op| {
            op.current_phase = Some(phase_str);
            op.login_phase = Some(kind);
        });
    }
}

/// `VerifyProgressSink` adapter — emits both `ProgressEvent` (for phase
/// advance + terminal) and the typed `VerifyAccountEvent` (for per-row
/// badge flips) on the same `op-progress::<op_id>` channel. Maintains
/// running counts on `RunningOps::verify_results` so the polling backstop
/// returns the right summary even if the channel listener missed events.
pub struct TauriVerifyProgressSink {
    pub app: AppHandle,
    pub op_id: String,
    pub ops: RunningOps,
}

impl TauriVerifyProgressSink {
    fn channel(&self) -> String {
        format!("op-progress::{}", self.op_id)
    }
}

impl claudepot_core::services::account_service::VerifyProgressSink for TauriVerifyProgressSink {
    fn event(&self, ev: claudepot_core::services::account_service::VerifyEvent) {
        use claudepot_core::services::account_service::{VerifyEvent, VerifyOutcomeKind};
        match ev {
            VerifyEvent::Started { total } => {
                let payload = ProgressEvent {
                    op_id: self.op_id.clone(),
                    phase: "verify".to_string(),
                    status: "running".to_string(),
                    done: Some(0),
                    total: Some(total),
                    detail: None,
                };
                let _ = self.app.emit(&self.channel(), payload);
                self.ops.update(&self.op_id, |op| {
                    op.current_phase = Some("verify".to_string());
                    op.sub_progress = Some((0, total));
                    op.verify_results = Some(VerifyResultSummary {
                        total,
                        ..Default::default()
                    });
                });
            }
            VerifyEvent::Account {
                uuid,
                email,
                idx,
                total,
                outcome,
                detail,
            } => {
                let outcome_str = match outcome {
                    VerifyOutcomeKind::Ok => "ok",
                    VerifyOutcomeKind::Drift => "drift",
                    VerifyOutcomeKind::Rejected => "rejected",
                    VerifyOutcomeKind::NetworkError => "network_error",
                };
                // 1) Per-row typed event — sibling payload, NOT pipe-delimited
                //    into `ProgressEvent.detail`.
                let row_payload = VerifyAccountEvent {
                    op_id: self.op_id.clone(),
                    kind: "verify_account",
                    uuid: uuid.to_string(),
                    email: email.clone(),
                    idx,
                    total,
                    outcome: outcome_str.to_string(),
                    detail: detail.clone(),
                };
                let _ = self.app.emit(&self.channel(), row_payload);

                // 2) Standard sub-progress on the same channel so the
                //    running-ops strip can render `idx/total` without
                //    knowing about the typed row event.
                let progress_payload = ProgressEvent {
                    op_id: self.op_id.clone(),
                    phase: "verify".to_string(),
                    status: "running".to_string(),
                    done: Some(idx),
                    total: Some(total),
                    detail: None,
                };
                let _ = self.app.emit(&self.channel(), progress_payload);

                self.ops.update(&self.op_id, |op| {
                    op.sub_progress = Some((idx, total));
                    if let Some(r) = op.verify_results.as_mut() {
                        match outcome {
                            VerifyOutcomeKind::Ok => r.ok += 1,
                            VerifyOutcomeKind::Drift => r.drift += 1,
                            VerifyOutcomeKind::Rejected => r.rejected += 1,
                            VerifyOutcomeKind::NetworkError => r.network_error += 1,
                        }
                    }
                });
            }
            VerifyEvent::Done => {
                // The terminal `op` event is emitted by `emit_terminal`
                // in the work closure, after the function returns; here
                // we just emit the per-phase `complete` so the modal can
                // flip the final phase row.
                let payload = ProgressEvent {
                    op_id: self.op_id.clone(),
                    phase: "verify".to_string(),
                    status: "complete".to_string(),
                    done: None,
                    total: None,
                    detail: None,
                };
                let _ = self.app.emit(&self.channel(), payload);
            }
        }
    }
}

/// Spawn an OS thread bound to a fresh `TauriProgressSink`. The work
/// closure receives the sink plus cloned `app` / `ops` / `op_id`
/// handles so it can call `emit_terminal` / `ops.update` directly
/// — preserving the escape hatch for ops that need bespoke
/// post-processing on success (e.g. storing a structured summary).
///
/// `std::thread::spawn` rather than `tokio::spawn`: Tauri's sync
/// command path doesn't always have a reactor running, and the work
/// is blocking I/O anyway.
pub fn spawn_op_thread<F>(
    app: AppHandle,
    ops: RunningOps,
    op_id: String,
    work: F,
) where
    F: FnOnce(TauriProgressSink, AppHandle, RunningOps, String) + Send + 'static,
{
    std::thread::spawn(move || {
        let sink = TauriProgressSink {
            app: app.clone(),
            op_id: op_id.clone(),
            ops: ops.clone(),
        };
        work(sink, app, ops, op_id);
    });
}
