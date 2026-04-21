//! `LiveRuntime` — composes registry + tail + status + bus.
//!
//! One runtime instance per Claudepot process. Owns:
//!   * the two buses (aggregate + per-session detail)
//!   * a map of per-session state (tail, status machine, seq counter)
//!   * a poll task that ticks on a 500 ms cadence and reacts to
//!     FSEvents (notify crate) via a small event coalescer
//!
//! M1 ships this WITHOUT the notify fast-path — pure polling. The
//! notify watcher is a follow-on optimization; poll at 500 ms already
//! gives < 1 s status latency and is trivially correct. The trade:
//! ~60 cheap readdir+stat calls per minute on a 6-session machine
//! (measured negligible).
//!
//! Startup seed: for each PID record encountered on the first tick,
//! we open the transcript `at_eof` (not `at_start`) — we do NOT
//! replay history. This matches the plan's "live view only" framing;
//! the historical Sessions browser remains authoritative for
//! after-the-fact inspection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{mpsc, watch, Mutex};

use crate::paths;
use crate::project_sanitize::sanitize_path;
use crate::session::{parse_line_into, SessionEvent};
use crate::session_live::bus::{AggregateBus, BusError, DetailBus};
use crate::session_live::metrics_store::MetricsStore;
use crate::session_live::redact::redact_secrets_opt;
use crate::session_live::registry::{self, ProcessCheck, SysinfoCheck};
use crate::session_live::status::StatusMachine;
use crate::session_live::tail::FileTail;
use crate::session_live::types::{
    LiveDelta, LiveDeltaKind, LiveSessionSummary, PidRecord, Status,
};

/// How often the runtime polls the PID registry and tails each
/// transcript. 500 ms matches the plan's M1 target of sub-second
/// status latency.
const TICK_INTERVAL: Duration = Duration::from_millis(500);

/// Runtime lifecycle holder. Cheap to clone (all heavy state is
/// behind `Arc`). Call `start` once after construction; call `stop`
/// to cancel the poll task. Subsequent `start` calls after `stop`
/// rebuild a fresh task.
pub struct LiveRuntime {
    check: Arc<dyn ProcessCheck>,
    sessions_dir: PathBuf,
    projects_dir: PathBuf,
    aggregate: AggregateBus,
    detail: DetailBus,
    state: Arc<Mutex<HashMap<String, SessionState>>>,
    /// Durable metrics — `None` means the store failed to open; the
    /// runtime still functions, Trends just sees no new data. Never
    /// fatal. Lives behind `Arc` so the aggregate handle can fan it
    /// out to background tasks if needed.
    metrics: Option<Arc<MetricsStore>>,
    /// Paths that the user has asked the runtime to ignore (via
    /// `preferences.activity_excluded_paths`). Compared as prefix
    /// matches against `PidRecord.cwd`. Mutable so the Tauri
    /// command layer can update it when the user edits the pref
    /// without tearing down the runtime.
    excluded_paths: Arc<Mutex<Vec<String>>>,
    /// Cancellation flag set by `stop`; the poll loop checks it.
    running: Arc<AtomicBool>,
}

/// Per-session mutable state the runtime carries between ticks.
struct SessionState {
    pid: u32,
    session_id: String,
    cwd: String,
    transcript_path: PathBuf,
    tail: FileTail,
    machine: StatusMachine,
    started_at_ms: i64,
    seq: u64,
    /// Last-published summary, used to suppress redundant deltas.
    last_status: Status,
    last_current_action: Option<String>,
    last_task_summary: Option<String>,
    last_errored: bool,
    last_stuck: bool,
    /// What the metrics store last saw for this session — used to
    /// gate writes to transition + heartbeat. `None` means no tick
    /// has been written yet; the next tick always writes (so every
    /// new session is represented in the store regardless of which
    /// Status it happens to land on first).
    last_metrics_status: Option<Status>,
    last_metrics_errored: bool,
    last_metrics_stuck: bool,
    /// Last-seen model id in metrics; drives ModelChanged emission.
    last_metrics_model: Option<String>,
    /// Ticks elapsed since the last metrics write for this session.
    /// Resets to 0 on every write; forces a heartbeat write when it
    /// crosses HEARTBEAT_TICKS (defined in `tick`).
    ticks_since_metrics: u64,
}

impl LiveRuntime {
    /// Production constructor — uses real `~/.claude/sessions` +
    /// `~/.claude/projects` and the `sysinfo`-backed process check.
    pub fn new() -> Arc<Self> {
        let check: Arc<dyn ProcessCheck> = Arc::new(SysinfoCheck::new());
        let cfg = paths::claude_config_dir();
        let metrics = MetricsStore::open_default()
            .ok()
            .map(Arc::new);
        if metrics.is_none() {
            tracing::warn!(
                target = "session_live::runtime",
                "activity metrics store unavailable — Trends view will show no data"
            );
        }
        Arc::new(Self {
            check,
            sessions_dir: cfg.join("sessions"),
            projects_dir: cfg.join("projects"),
            aggregate: AggregateBus::new(),
            detail: DetailBus::new(),
            state: Arc::new(Mutex::new(HashMap::new())),
            metrics,
            excluded_paths: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Test constructor — inject arbitrary dirs and a `ProcessCheck`.
    /// The metrics store is disabled in this mode; tests that want
    /// metrics coverage construct a `MetricsStore` directly against
    /// a tempdir and call it out-of-band.
    pub fn with_dirs(
        check: Arc<dyn ProcessCheck>,
        sessions_dir: PathBuf,
        projects_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            check,
            sessions_dir,
            projects_dir,
            aggregate: AggregateBus::new(),
            detail: DetailBus::new(),
            state: Arc::new(Mutex::new(HashMap::new())),
            metrics: None,
            excluded_paths: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Replace the excluded-paths list. Called by the Tauri command
    /// layer whenever the user edits the `activity_excluded_paths`
    /// preference. Any PID record whose `cwd` is prefix-matched by
    /// an entry in this list is skipped by `tick()` — it never
    /// appears in the aggregate snapshot and no transcript tail is
    /// opened.
    pub async fn set_excluded_paths(&self, paths: Vec<String>) {
        let mut w = self.excluded_paths.lock().await;
        *w = paths;
    }

    /// Reference to the metrics store. Consumed by the Tauri command
    /// layer for the Trends view queries.
    pub fn metrics(&self) -> Option<Arc<MetricsStore>> {
        self.metrics.clone()
    }

    /// Spawn the poll loop. Idempotent: calling start twice when
    /// already running is a no-op. Handle is returned for tests
    /// that want to await completion after `stop`; production
    /// callers can discard it.
    pub fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            // Already running. Spawn a trivially-complete task for
            // caller API symmetry.
            return tokio::spawn(async {});
        }
        let this = Arc::clone(&self);
        tokio::spawn(async move {
            while this.running.load(Ordering::SeqCst) {
                if let Err(e) = this.tick().await {
                    tracing::warn!(
                        target = "session_live::runtime",
                        error = %e,
                        "tick failed (continuing)"
                    );
                }
                tokio::time::sleep(TICK_INTERVAL).await;
            }
        })
    }

    /// Stop the poll loop. Sets the cancellation flag; the running
    /// task will exit on its next sleep wakeup (≤ 500 ms).
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Aggregate subscription for surfaces that want the full list
    /// (tray, sidebar strip, status bar).
    pub fn subscribe_aggregate(
        &self,
    ) -> watch::Receiver<Arc<Vec<LiveSessionSummary>>> {
        self.aggregate.subscribe()
    }

    /// Detail subscription for one session — used by the live pane.
    /// Single-subscriber contract per `DetailBus::subscribe`.
    pub async fn subscribe_detail(
        &self,
        session_id: &str,
    ) -> Result<mpsc::Receiver<LiveDelta>, BusError> {
        self.detail.subscribe(session_id).await
    }

    /// Explicitly end a per-session detail subscription. Called by
    /// the Tauri `session_live_unsubscribe` command so the backend-
    /// side slot is torn down when the frontend listener goes away.
    pub async fn detail_end_session(&self, session_id: &str) {
        self.detail.end_session(session_id).await;
    }

    /// Current aggregate snapshot — cheap, synchronous.
    pub fn snapshot(&self) -> Arc<Vec<LiveSessionSummary>> {
        self.aggregate.snapshot()
    }

    /// One-session snapshot for resync after a `resync_required`
    /// signal. Returns `None` if the session is not currently live.
    pub async fn session_snapshot(&self, session_id: &str) -> Option<LiveSessionSummary> {
        let state = self.state.lock().await;
        state
            .get(session_id)
            .map(|s| summary_from_state(s, Utc::now().timestamp_millis()))
    }

    /// Run exactly one poll cycle. Public for tests; the production
    /// path is the looped call inside `start`.
    pub async fn tick(&self) -> std::io::Result<()> {
        let outcome = registry::poll_dir(&self.sessions_dir, &*self.check)?;
        // Sweep stale PID files (non-WSL only — the poller respects
        // that guard internally).
        registry::sweep_stale(&outcome);

        // Filter the registry output against the excluded-paths
        // preference BEFORE we do anything else — excluded sessions
        // must never appear in the aggregate, never get a tail
        // opened, never fire a delta. Prefix match on `cwd`.
        let excluded = self.excluded_paths.lock().await.clone();
        let mut filtered = outcome;
        if !excluded.is_empty() {
            filtered.live.retain(|r| {
                !excluded.iter().any(|p| r.cwd.starts_with(p))
            });
        }
        let outcome = filtered;

        let now_ms = Utc::now().timestamp_millis();
        let mut state = self.state.lock().await;

        // 1. Drop sessions that disappeared from the registry.
        let live_ids: std::collections::HashSet<String> = outcome
            .live
            .iter()
            .map(|r| r.session_id.clone())
            .collect();
        let gone: Vec<String> = state
            .keys()
            .filter(|k| !live_ids.contains(*k))
            .cloned()
            .collect();
        for sid in &gone {
            // Emit an Ended delta if anyone was listening.
            let seq = state.get(sid).map(|s| s.seq).unwrap_or(0) + 1;
            let _ = self
                .detail
                .publish_delta(LiveDelta {
                    session_id: sid.clone(),
                    seq,
                    produced_at_ms: now_ms,
                    kind: LiveDeltaKind::Ended,
                    resync_required: false,
                })
                .await;
            self.detail.end_session(sid).await;
            state.remove(sid);
        }

        // 2. Attach newly-appeared sessions.
        for rec in &outcome.live {
            if state.contains_key(&rec.session_id) {
                continue;
            }
            match SessionState::try_attach(rec, &self.projects_dir) {
                Ok(s) => {
                    state.insert(rec.session_id.clone(), s);
                }
                Err(e) => {
                    // Transcript missing (session just started and the
                    // first line hasn't hit disk yet, or the project
                    // slug resolution failed) — try again next tick.
                    tracing::debug!(
                        target = "session_live::runtime",
                        session_id = %rec.session_id,
                        error = %e,
                        "attach deferred (transcript not ready)"
                    );
                }
            }
        }

        // 3. Tail each live session's transcript and ingest new lines.
        for rec in &outcome.live {
            let Some(s) = state.get_mut(&rec.session_id) else {
                continue;
            };
            // Apply authoritative status from the PID file when CC
            // publishes it (BG_SESSIONS feature on).
            let pid_status = rec.status.as_deref().map(Status::from_pid_field);
            s.machine
                .set_pid_status(pid_status, rec.waiting_for.clone());
            let pid_waiting_for_snap = rec.waiting_for.clone();

            let progress = match s.tail.poll() {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!(
                        target = "session_live::runtime",
                        path = %s.transcript_path.display(),
                        error = %e,
                        "tail poll failed (will retry)"
                    );
                    continue;
                }
            };
            if progress.rotated {
                s.machine = StatusMachine::new();
            }
            let mut events: Vec<SessionEvent> = Vec::new();
            for (i, line) in progress.new_lines.iter().enumerate() {
                parse_line_into(&mut events, line, i + 1);
            }
            for e in &events {
                s.machine.ingest(e);
            }

            // Compute the new snapshot and emit deltas for any
            // user-visible transitions.
            let snap = s.machine.snapshot();
            let new_status = snap.status;
            let new_action = snap.current_action.clone();
            let new_errored = snap.errored;
            let new_stuck = snap.stuck;

            if new_status != s.last_status
                || (new_status == Status::Waiting
                    && pid_waiting_for_snap != snapshot_waiting_for(s))
            {
                s.seq += 1;
                let _ = self
                    .detail
                    .publish_delta(LiveDelta {
                        session_id: s.session_id.clone(),
                        seq: s.seq,
                        produced_at_ms: now_ms,
                        kind: LiveDeltaKind::StatusChanged {
                            status: new_status,
                            waiting_for: pid_waiting_for_snap.clone(),
                        },
                        resync_required: false,
                    })
                    .await;
                s.last_status = new_status;
            }
            if new_action != s.last_current_action {
                s.last_current_action = new_action;
            }
            // Emit TaskSummaryChanged when CC wrote a new
            // `task-summary` entry. Use the raw task_summary field
            // (pre-redaction — it's already a human description,
            // not a tool arg) so subscribers can render it verbatim
            // in the live-pane current-action card. Redaction on
            // the content happens at the DTO boundary in
            // summary_from_state → current_action.
            if snap.task_summary != s.last_task_summary {
                if let Some(summary_text) = &snap.task_summary {
                    let redacted =
                        crate::session_live::redact::redact_secrets(summary_text);
                    s.seq += 1;
                    let _ = self
                        .detail
                        .publish_delta(LiveDelta {
                            session_id: s.session_id.clone(),
                            seq: s.seq,
                            produced_at_ms: now_ms,
                            kind: LiveDeltaKind::TaskSummaryChanged {
                                summary: redacted,
                            },
                            resync_required: false,
                        })
                        .await;
                }
                s.last_task_summary = snap.task_summary.clone();
            }
            if new_errored != s.last_errored || new_stuck != s.last_stuck {
                s.seq += 1;
                let _ = self
                    .detail
                    .publish_delta(LiveDelta {
                        session_id: s.session_id.clone(),
                        seq: s.seq,
                        produced_at_ms: now_ms,
                        kind: LiveDeltaKind::OverlayChanged {
                            errored: new_errored,
                            stuck: new_stuck,
                        },
                        resync_required: false,
                    })
                    .await;
                s.last_errored = new_errored;
                s.last_stuck = new_stuck;
            }
            // ModelChanged emission — fires when the model id
            // observed in the latest assistant fragment differs
            // from the last one we announced. Without this, the
            // frontend's liveModel override (wired for task parity)
            // would be dead code.
            if snap.model != s.last_metrics_model {
                if let Some(new_model) = snap.model.clone() {
                    s.seq += 1;
                    let _ = self
                        .detail
                        .publish_delta(LiveDelta {
                            session_id: s.session_id.clone(),
                            seq: s.seq,
                            produced_at_ms: now_ms,
                            kind: LiveDeltaKind::ModelChanged {
                                model: new_model,
                            },
                            resync_required: false,
                        })
                        .await;
                }
                s.last_metrics_model = snap.model.clone();
            }
        }

        // 4. Republish the aggregate list. Idempotent — watch
        // subscribers see the latest even if they missed intermediate.
        let list: Vec<LiveSessionSummary> = state
            .values()
            .map(|s| summary_from_state(s, now_ms))
            .collect();
        drop(state);

        // 5. Record to the durable metrics store. Two write paths:
        //
        //   a) On transition (status / errored / stuck change) —
        //      the edge the Trends view really cares about.
        //   b) On heartbeat (every HEARTBEAT_TICKS ticks) — preserves
        //      per-bucket density so `active_series` still sees each
        //      long-running session represented in every time bucket.
        //      Without this, a session busy for 30 min would only
        //      land rows at its transitions and vanish from buckets
        //      in between.
        //
        // Transition-only by itself broke active_series carry-forward
        // semantics (reported by the audit). The heartbeat is the
        // fix: bounded writes (~1/30s/session at a 500ms tick), but
        // every bucket longer than the heartbeat interval still
        // sees each live session.
        const HEARTBEAT_TICKS: u64 = 60; // 30s at a 500ms cadence
        let mut to_write: Vec<LiveSessionSummary> = Vec::new();
        let mut state_for_marks = self.state.lock().await;
        for row in &list {
            let Some(ss) = state_for_marks.get_mut(&row.session_id) else {
                continue;
            };
            let first_tick = ss.last_metrics_status.is_none();
            let transitioned = ss
                .last_metrics_status
                .map(|s| s != row.status)
                .unwrap_or(false)
                || row.errored != ss.last_metrics_errored
                || row.stuck != ss.last_metrics_stuck;
            let is_heartbeat = ss.ticks_since_metrics >= HEARTBEAT_TICKS;
            if transitioned || is_heartbeat || first_tick {
                ss.last_metrics_status = Some(row.status);
                ss.last_metrics_errored = row.errored;
                ss.last_metrics_stuck = row.stuck;
                ss.ticks_since_metrics = 0;
                to_write.push(row.clone());
            } else {
                ss.ticks_since_metrics = ss.ticks_since_metrics.saturating_add(1);
            }
        }
        drop(state_for_marks);
        if let Some(ref m) = self.metrics {
            if !to_write.is_empty() {
                if let Err(e) = m.record_tick(now_ms, &to_write) {
                    tracing::debug!(
                        target = "session_live::runtime",
                        error = %e,
                        "metrics tick write failed (non-fatal)"
                    );
                }
            }
        }

        self.aggregate.publish(list);
        Ok(())
    }
}

/// Transcript path: `<projects_dir>/<sanitized_cwd>/<sessionId>.jsonl`.
/// Mirrors CC's `sessionStoragePortable.ts::sanitizePath` convention.
fn transcript_path(projects_dir: &Path, cwd: &str, session_id: &str) -> PathBuf {
    let slug = sanitize_path(cwd);
    projects_dir.join(slug).join(format!("{session_id}.jsonl"))
}

fn snapshot_waiting_for(s: &SessionState) -> Option<String> {
    s.machine.snapshot().waiting_for
}

impl SessionState {
    fn try_attach(rec: &PidRecord, projects_dir: &Path) -> std::io::Result<Self> {
        let path = transcript_path(projects_dir, &rec.cwd, &rec.session_id);
        // at_eof: don't replay historical content on attach. The live
        // view is forward-only; history lives in the static Sessions
        // browser.
        let tail = FileTail::at_eof(&path)?;
        let mut machine = StatusMachine::new();
        // Prime with the PID-file status if present (BG_SESSIONS on).
        if let Some(raw) = rec.status.as_deref() {
            machine.set_pid_status(
                Some(Status::from_pid_field(raw)),
                rec.waiting_for.clone(),
            );
        }
        Ok(Self {
            pid: rec.pid,
            session_id: rec.session_id.clone(),
            cwd: rec.cwd.clone(),
            transcript_path: path,
            tail,
            machine,
            started_at_ms: rec.started_at_ms,
            seq: 0,
            last_status: Status::Idle,
            last_current_action: None,
            last_task_summary: None,
            last_errored: false,
            last_stuck: false,
            // `None` means no write yet — the first tick always
            // lands a row regardless of which Status this session
            // arrives on. Using a real Status as a sentinel
            // collides with the steady-state value of that status
            // (a legitimate Waiting session would write every tick).
            last_metrics_status: None,
            last_metrics_errored: false,
            last_metrics_stuck: false,
            last_metrics_model: None,
            ticks_since_metrics: 0,
        })
    }
}

/// Derive a `LiveSessionSummary` from the per-session state. Every
/// user-content string — including the path fields — passes through
/// the redactor before we hand it to the DTO layer. A cwd like
/// `/private/customer-secrets/foo` should not leak through the
/// peripheral surface even though the path itself isn't a token.
fn summary_from_state(s: &SessionState, now_ms: i64) -> LiveSessionSummary {
    let snap = s.machine.snapshot();
    let idle_ms = snap
        .last_activity_ts
        .map(|t| (now_ms - t.timestamp_millis()).max(0))
        .unwrap_or(now_ms - s.started_at_ms);
    LiveSessionSummary {
        session_id: s.session_id.clone(),
        pid: s.pid,
        cwd: crate::session_live::redact::redact_secrets(&s.cwd),
        transcript_path: Some(crate::session_live::redact::redact_secrets(
            &s.transcript_path.to_string_lossy(),
        )),
        status: snap.status,
        current_action: snap.current_action.map(|a| redact_secrets_opt(Some(&a))),
        model: snap.model,
        waiting_for: snap.waiting_for,
        errored: snap.errored,
        stuck: snap.stuck,
        idle_ms,
        seq: s.seq,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
