//! `LiveActivityService` — owns the lifecycle, listener fan-out,
//! membership-diff, and 1s debounce policy that previously lived
//! inside `commands_activity.rs`. The Tauri layer becomes a
//! pass-through; one `TauriSessionEventListener` translates service
//! events to webview emits and tray rebuilds.
//!
//! The service is deliberately framework-free — it does not depend
//! on `tauri` and runs identically under unit tests with stub
//! `LiveRuntime` instances built via `LiveRuntime::with_dirs`.
//!
//! ### Membership debounce
//!
//! The aggregate bridge fires `on_aggregate` for every tick. It
//! also computes a membership-set diff against `last_membership`
//! and fires `on_membership_changed` only when the set changed.
//! To avoid hammering downstream consumers (the tray rebuild is
//! synchronous on AppKit and expensive), we apply a 1s debounce:
//! at most one `on_membership_changed` per second. Within the
//! debounce window, additional changes are coalesced — exactly one
//! delivery happens after the window elapses, with the latest
//! membership.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::session_live::bus::BusError;
use crate::session_live::metrics_store::MetricsStore;
use crate::session_live::types::{LiveDelta, LiveSessionSummary};
use crate::session_live::LiveRuntime;

/// Minimum interval between two `on_membership_changed` deliveries.
/// Mirrors the prior IPC-layer behaviour exactly.
const MEMBERSHIP_DEBOUNCE: Duration = Duration::from_secs(1);

/// Sink for live-session events. The service fans out to all
/// registered listeners under a short async-mutex critical section.
pub trait SessionEventListener: Send + Sync + 'static {
    /// Aggregate snapshot — fires on every tick.
    fn on_aggregate(&self, sessions: Arc<Vec<LiveSessionSummary>>);
    /// Fires when the set of live session ids changed (post-debounce).
    fn on_membership_changed(&self, sessions: Arc<Vec<LiveSessionSummary>>);
    /// Per-session detail delta — fires for every delta the runtime
    /// publishes on the matching `live::<sid>` channel.
    fn on_detail(&self, session_id: &str, delta: LiveDelta);
}

/// Errors surfaced by the service. Kept narrow — channel-saturation
/// drops still flow through the runtime's existing `resync_required`
/// signalling and are not service-level errors.
#[derive(Debug, thiserror::Error)]
pub enum LiveActivityError {
    #[error("already subscribed to session {0}")]
    AlreadySubscribed(String),
    #[error("not started")]
    NotStarted,
}

/// The service. Owns the runtime, the listener list, and the bridge
/// task handles. Cheap to clone via `Arc`. Build with `new` for
/// production, `with_runtime` for tests.
pub struct LiveActivityService {
    runtime: Arc<LiveRuntime>,
    listeners: AsyncMutex<Vec<Arc<dyn SessionEventListener>>>,
    aggregate_handle: AsyncMutex<Option<JoinHandle<()>>>,
    detail_handles: AsyncMutex<HashMap<String, JoinHandle<()>>>,
    started: AsyncMutex<bool>,
    last_membership: AsyncMutex<BTreeSet<String>>,
}

impl LiveActivityService {
    /// Production constructor — builds a real `LiveRuntime`.
    pub fn new() -> Arc<Self> {
        Self::with_runtime(LiveRuntime::new())
    }

    /// Test / DI constructor — accepts an injected runtime.
    pub fn with_runtime(runtime: Arc<LiveRuntime>) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            listeners: AsyncMutex::new(Vec::new()),
            aggregate_handle: AsyncMutex::new(None),
            detail_handles: AsyncMutex::new(HashMap::new()),
            started: AsyncMutex::new(false),
            last_membership: AsyncMutex::new(BTreeSet::new()),
        })
    }

    /// Enable activity-cards classification on the inner runtime.
    /// Forwards to `LiveRuntime::enable_activity`. Optional — without
    /// it, the runtime keeps its prior behavior (live-strip status
    /// only, no per-event card extraction).
    ///
    /// Idempotent: calling repeatedly with the same `idx` is a no-op
    /// because the runtime stores the index in a `Mutex<Option<…>>`
    /// keyed by replacement, not append.
    pub fn enable_activity(&self, idx: Arc<crate::activity::ActivityIndex>) {
        self.runtime.enable_activity(idx);
    }

    /// Start the runtime + spawn the aggregate bridge task. Idempotent:
    /// repeated calls return `Ok(())` without re-spawning. The caller
    /// supplies the user's `excluded_paths` preference; the service
    /// pushes it into the runtime before the first tick so an
    /// excluded project never appears in even the first aggregate.
    pub async fn start(self: &Arc<Self>, excluded: Vec<String>) -> Result<(), LiveActivityError> {
        let mut started = self.started.lock().await;
        if *started {
            return Ok(());
        }
        // Apply excluded paths BEFORE the runtime starts ticking.
        self.runtime.set_excluded_paths(excluded).await;

        // Spawn the aggregate bridge task. It owns its own copy of
        // the receiver so subsequent `subscribe_aggregate` calls
        // (e.g. from tests) hand out independent cursors.
        let runtime = Arc::clone(&self.runtime);
        let mut rx = runtime.subscribe_aggregate();
        let this = Arc::clone(self);
        let handle = tokio::spawn(async move {
            // Track the last delivery time inside this task so the
            // debounce is local — no shared state, no cross-task
            // contention.
            let mut last_delivery: Option<Instant> = None;
            let mut pending_change: Option<Arc<Vec<LiveSessionSummary>>> = None;
            loop {
                // Wake on either: a new aggregate value, or the
                // debounce window elapsing with a coalesced change
                // pending. Select between the two so a quiet runtime
                // doesn't stall a pending membership delivery.
                let sleep_for = match (pending_change.as_ref(), last_delivery) {
                    (Some(_), Some(t)) => {
                        let elapsed = t.elapsed();
                        if elapsed >= MEMBERSHIP_DEBOUNCE {
                            Duration::from_millis(0)
                        } else {
                            MEMBERSHIP_DEBOUNCE - elapsed
                        }
                    }
                    _ => Duration::from_secs(60 * 60),
                };
                tokio::select! {
                    biased;
                    changed = rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let list_arc = rx.borrow_and_update().clone();
                        // Always deliver on_aggregate.
                        this.dispatch_aggregate(Arc::clone(&list_arc)).await;

                        // Membership diff.
                        let current_ids: BTreeSet<String> = list_arc
                            .iter()
                            .map(|s| s.session_id.clone())
                            .collect();
                        let mut last = this.last_membership.lock().await;
                        if current_ids != *last {
                            *last = current_ids;
                            drop(last);
                            // Apply debounce.
                            let can_deliver_now = match last_delivery {
                                None => true,
                                Some(t) => t.elapsed() >= MEMBERSHIP_DEBOUNCE,
                            };
                            if can_deliver_now {
                                last_delivery = Some(Instant::now());
                                pending_change = None;
                                this.dispatch_membership(Arc::clone(&list_arc))
                                    .await;
                            } else {
                                // Coalesce — keep latest.
                                pending_change = Some(list_arc);
                            }
                        }
                    }
                    _ = tokio::time::sleep(sleep_for) => {
                        if let Some(list_arc) = pending_change.take() {
                            last_delivery = Some(Instant::now());
                            this.dispatch_membership(list_arc).await;
                        }
                    }
                }
            }
        });
        *self.aggregate_handle.lock().await = Some(handle);
        *started = true;
        drop(started);

        // Kick the runtime poll loop. The runtime is internally
        // race-safe across rapid stop→start cycles.
        let _ = Arc::clone(&self.runtime).start();
        Ok(())
    }

    /// Stop the runtime and abort all bridge tasks (aggregate +
    /// per-session detail). Idempotent: repeated calls are no-ops.
    pub async fn stop(&self) {
        self.runtime.stop();
        if let Some(h) = self.aggregate_handle.lock().await.take() {
            h.abort();
        }
        let mut details = self.detail_handles.lock().await;
        for (_, h) in details.drain() {
            h.abort();
        }
        drop(details);
        *self.started.lock().await = false;
        // Reset membership so a subsequent start fires a fresh diff
        // against empty rather than stale data.
        self.last_membership.lock().await.clear();
    }

    /// Register a listener. Listeners receive every event
    /// post-registration. The service does not back-fire prior
    /// snapshots — callers that need an initial state should call
    /// `snapshot()` after `subscribe`.
    pub async fn subscribe(&self, listener: Arc<dyn SessionEventListener>) {
        self.listeners.lock().await.push(listener);
    }

    /// Begin forwarding per-session deltas for `session_id`. Spawns
    /// one task that pumps the runtime's detail receiver into every
    /// registered listener's `on_detail`. Returns
    /// `LiveActivityError::AlreadySubscribed` if a forwarder is
    /// already running for this id (mirrors `BusError::AlreadySubscribed`
    /// at the runtime layer).
    pub async fn subscribe_detail(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<(), LiveActivityError> {
        // Service-level guard so we don't even attempt the runtime
        // call if our own forwarder is already wired up.
        {
            let handles = self.detail_handles.lock().await;
            if handles.contains_key(session_id) {
                return Err(LiveActivityError::AlreadySubscribed(session_id.to_string()));
            }
        }
        let mut rx = self
            .runtime
            .subscribe_detail(session_id)
            .await
            .map_err(|e| match e {
                BusError::AlreadySubscribed => {
                    LiveActivityError::AlreadySubscribed(session_id.to_string())
                }
                BusError::SubscriberGone => LiveActivityError::NotStarted,
            })?;
        let sid = session_id.to_string();
        let this = Arc::clone(self);
        let handle = tokio::spawn(async move {
            while let Some(delta) = rx.recv().await {
                let listeners = this.listeners.lock().await.clone();
                for l in listeners.iter() {
                    l.on_detail(&sid, delta.clone());
                }
            }
        });
        self.detail_handles
            .lock()
            .await
            .insert(session_id.to_string(), handle);
        Ok(())
    }

    /// Tear down the per-session detail forwarder. Idempotent: safe
    /// to call for an unsubscribed id (no-op).
    pub async fn unsubscribe_detail(&self, session_id: &str) {
        if let Some(h) = self.detail_handles.lock().await.remove(session_id) {
            h.abort();
        }
        self.runtime.detail_end_session(session_id).await;
    }

    /// Synchronous aggregate snapshot. Cheap.
    pub fn snapshot(&self) -> Arc<Vec<LiveSessionSummary>> {
        self.runtime.snapshot()
    }

    /// One-session resync answer. Returns `None` if the session is
    /// not currently live.
    pub async fn session_snapshot(&self, id: &str) -> Option<LiveSessionSummary> {
        self.runtime.session_snapshot(id).await
    }

    /// Replace the runtime's excluded-paths filter. Called by the
    /// Tauri layer whenever the user edits
    /// `preferences.activity_excluded_paths`.
    pub async fn set_excluded_paths(&self, paths: Vec<String>) {
        self.runtime.set_excluded_paths(paths).await;
    }

    /// Reference to the durable metrics store. Used by the Trends
    /// view command to query bucketed history.
    pub fn metrics(&self) -> Option<Arc<MetricsStore>> {
        self.runtime.metrics()
    }

    // --- internals ---

    /// Fan out to all listeners under the shared async mutex. The
    /// listeners list is small in practice (1–2); we clone the
    /// `Arc` slice out under the lock and dispatch outside it so
    /// listener bodies don't serialize on the lock.
    async fn dispatch_aggregate(&self, sessions: Arc<Vec<LiveSessionSummary>>) {
        let listeners = self.listeners.lock().await.clone();
        for l in listeners.iter() {
            l.on_aggregate(Arc::clone(&sessions));
        }
    }

    async fn dispatch_membership(&self, sessions: Arc<Vec<LiveSessionSummary>>) {
        let listeners = self.listeners.lock().await.clone();
        for l in listeners.iter() {
            l.on_membership_changed(Arc::clone(&sessions));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_live::registry::ProcessCheck;
    use std::collections::HashSet;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    /// Fake process check: declares a fixed set of PIDs alive.
    #[derive(Default)]
    struct FakeCheck {
        alive: StdMutex<HashSet<u32>>,
    }

    impl FakeCheck {
        fn set_alive(&self, pids: &[u32]) {
            let mut a = self.alive.lock().unwrap();
            a.clear();
            a.extend(pids.iter().copied());
        }
    }

    impl ProcessCheck for FakeCheck {
        fn is_running(&self, pid: u32) -> bool {
            self.alive.lock().unwrap().contains(&pid)
        }
    }

    /// Recording listener. Counts calls and stores the last seen
    /// session id set so tests can assert on it.
    #[derive(Default)]
    struct RecordingListener {
        aggregate_calls: StdMutex<usize>,
        membership_calls: StdMutex<usize>,
        detail_calls: StdMutex<Vec<(String, LiveDelta)>>,
        last_membership: StdMutex<Vec<String>>,
    }

    impl SessionEventListener for RecordingListener {
        fn on_aggregate(&self, _sessions: Arc<Vec<LiveSessionSummary>>) {
            *self.aggregate_calls.lock().unwrap() += 1;
        }
        fn on_membership_changed(&self, sessions: Arc<Vec<LiveSessionSummary>>) {
            *self.membership_calls.lock().unwrap() += 1;
            let ids: Vec<String> = sessions.iter().map(|s| s.session_id.clone()).collect();
            *self.last_membership.lock().unwrap() = ids;
        }
        fn on_detail(&self, session_id: &str, delta: LiveDelta) {
            self.detail_calls
                .lock()
                .unwrap()
                .push((session_id.to_string(), delta));
        }
    }

    fn build_runtime() -> (TempDir, TempDir, Arc<LiveRuntime>, Arc<FakeCheck>) {
        let sessions_td = TempDir::new().unwrap();
        let projects_td = TempDir::new().unwrap();
        let check = Arc::new(FakeCheck::default());
        let runtime = LiveRuntime::with_dirs(
            check.clone(),
            sessions_td.path().to_path_buf(),
            projects_td.path().to_path_buf(),
        );
        (sessions_td, projects_td, runtime, check)
    }

    fn write_pid_file(dir: &std::path::Path, pid: u32, sid: &str, cwd: &str) {
        use std::io::Write;
        let body = format!(
            r#"{{"pid":{pid},"sessionId":"{sid}","cwd":"{cwd}","startedAt":0}}"#,
            pid = pid,
            sid = sid,
            cwd = cwd
        );
        let mut f = std::fs::File::create(dir.join(format!("{pid}.json"))).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    fn write_transcript(projects_dir: &std::path::Path, cwd: &str, sid: &str) {
        use crate::project_sanitize::sanitize_path;
        use std::io::Write;
        let slug = sanitize_path(cwd);
        let dir = projects_dir.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join(format!("{sid}.jsonl"))).unwrap();
        f.write_all(b"").unwrap();
    }

    #[tokio::test]
    async fn membership_change_fires_once_per_change() {
        let (_sessions_td, projects_td, runtime, check) = build_runtime();
        let service = LiveActivityService::with_runtime(runtime.clone());
        let listener = Arc::new(RecordingListener::default());
        service
            .subscribe(listener.clone() as Arc<dyn SessionEventListener>)
            .await;

        // Empty start.
        check.set_alive(&[]);
        runtime.tick().await.unwrap();

        // Add session A.
        write_pid_file(_sessions_td.path(), 1001, "sessA", "/tmp/a");
        write_transcript(projects_td.path(), "/tmp/a", "sessA");
        check.set_alive(&[1001]);
        // Subscribe BEFORE the membership change so the bridge
        // task is alive to observe it.
        service.start(vec![]).await.unwrap();
        // Allow the bridge task to install its receiver.
        tokio::time::sleep(Duration::from_millis(20)).await;
        runtime.tick().await.unwrap();
        // Wait long enough for the bridge to dispatch and the
        // debounce window to allow the next change.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let membership_after_first = *listener.membership_calls.lock().unwrap();

        // Wait beyond the debounce window before the next change
        // so it fires immediately rather than coalescing.
        tokio::time::sleep(MEMBERSHIP_DEBOUNCE + Duration::from_millis(50)).await;

        // Remove A, add B → membership changes.
        std::fs::remove_file(_sessions_td.path().join("1001.json")).unwrap();
        write_pid_file(_sessions_td.path(), 1002, "sessB", "/tmp/b");
        write_transcript(projects_td.path(), "/tmp/b", "sessB");
        check.set_alive(&[1002]);
        runtime.tick().await.unwrap();
        // Wait through dispatch.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let final_count = *listener.membership_calls.lock().unwrap();
        // One delivery per distinct change. The first change went
        // from {} to {sessA}; the second from {sessA} to {sessB}.
        assert!(
            membership_after_first >= 1,
            "first membership change must have fired (got {membership_after_first})"
        );
        assert_eq!(
            final_count, 2,
            "exactly two distinct membership changes must fire two callbacks (got {final_count})"
        );

        service.stop().await;
    }

    #[tokio::test]
    async fn membership_debounce_coalesces_within_1s() {
        let (_sessions_td, projects_td, runtime, check) = build_runtime();
        let service = LiveActivityService::with_runtime(runtime.clone());
        let listener = Arc::new(RecordingListener::default());
        service
            .subscribe(listener.clone() as Arc<dyn SessionEventListener>)
            .await;

        check.set_alive(&[]);
        service.start(vec![]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        // First membership change → fires immediately.
        write_pid_file(_sessions_td.path(), 2001, "sessA", "/tmp/a");
        write_transcript(projects_td.path(), "/tmp/a", "sessA");
        check.set_alive(&[2001]);
        runtime.tick().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let after_first = *listener.membership_calls.lock().unwrap();
        assert_eq!(
            after_first, 1,
            "first change fires immediately (got {after_first})"
        );

        // Second change INSIDE the 1s debounce window — must NOT
        // fire a second callback right away. It must be coalesced.
        write_pid_file(_sessions_td.path(), 2002, "sessB", "/tmp/b");
        write_transcript(projects_td.path(), "/tmp/b", "sessB");
        check.set_alive(&[2001, 2002]);
        runtime.tick().await.unwrap();
        // Within the debounce window — observe count.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mid = *listener.membership_calls.lock().unwrap();
        assert_eq!(
            mid, 1,
            "change inside debounce window must be coalesced (got {mid})"
        );

        // Wait past the window — the coalesced delivery must fire.
        tokio::time::sleep(MEMBERSHIP_DEBOUNCE + Duration::from_millis(100)).await;
        let after_window = *listener.membership_calls.lock().unwrap();
        assert_eq!(
            after_window, 2,
            "coalesced change delivers exactly once after the window (got {after_window})"
        );
        // And the latest membership reflects BOTH sessions.
        let last = listener.last_membership.lock().unwrap().clone();
        let mut sorted = last.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["sessA".to_string(), "sessB".to_string()],
            "post-debounce delivery must carry the latest membership"
        );

        service.stop().await;
    }

    #[tokio::test]
    async fn subscribe_detail_already_subscribed() {
        let (_sessions_td, projects_td, runtime, check) = build_runtime();
        let service = LiveActivityService::with_runtime(runtime.clone());
        let listener = Arc::new(RecordingListener::default());
        service
            .subscribe(listener.clone() as Arc<dyn SessionEventListener>)
            .await;

        // Bring up a single session so the runtime has a slot to
        // subscribe to.
        write_pid_file(_sessions_td.path(), 3001, "sessC", "/tmp/c");
        write_transcript(projects_td.path(), "/tmp/c", "sessC");
        check.set_alive(&[3001]);
        runtime.tick().await.unwrap();

        service.subscribe_detail("sessC").await.unwrap();
        let err = service.subscribe_detail("sessC").await.unwrap_err();
        match err {
            LiveActivityError::AlreadySubscribed(sid) => {
                assert_eq!(sid, "sessC");
            }
            other => panic!("expected AlreadySubscribed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stop_aborts_all_bridges() {
        let (_sessions_td, projects_td, runtime, check) = build_runtime();
        let service = LiveActivityService::with_runtime(runtime.clone());
        let listener = Arc::new(RecordingListener::default());
        service
            .subscribe(listener.clone() as Arc<dyn SessionEventListener>)
            .await;

        write_pid_file(_sessions_td.path(), 4001, "sessD", "/tmp/d");
        write_transcript(projects_td.path(), "/tmp/d", "sessD");
        check.set_alive(&[4001]);

        service.start(vec![]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        runtime.tick().await.unwrap();
        service.subscribe_detail("sessD").await.unwrap();

        // Pre-stop both handle slots are populated.
        assert!(service.aggregate_handle.lock().await.is_some());
        assert_eq!(service.detail_handles.lock().await.len(), 1);

        service.stop().await;

        // Post-stop both slots must be empty.
        assert!(service.aggregate_handle.lock().await.is_none());
        assert_eq!(service.detail_handles.lock().await.len(), 0);
        // And `started` flips back so a future start works.
        assert!(!*service.started.lock().await);
    }
}
