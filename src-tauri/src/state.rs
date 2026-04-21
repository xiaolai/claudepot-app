//! App-wide state held across Tauri command invocations.
//!
//! Today there is one piece of shared state: the Notify handle for an
//! in-flight `claude auth login` subprocess, so a Cancel button click can
//! reach back and kill it. Kept in a std Mutex because accesses are
//! non-async and very brief.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Tracks the currently running `claude auth login` flow, if any.
/// `None` when idle; `Some(notify)` when a login is in progress and
/// calling `notify.notify_one()` will abort it.
#[derive(Default)]
pub struct LoginState {
    pub active: Mutex<Option<Arc<Notify>>>,
}

/// Holds the latest cancel-token value received from the webview for
/// in-flight `project_move_dry_run` calls. Rapid typing replaces the
/// value; older in-flight calls read it before + after their expensive
/// work and bail out early when they're no longer the freshest.
#[derive(Default)]
pub struct DryRunRegistry {
    pub latest: AtomicU64,
}

/// Wraps `LiveRuntime` so Tauri commands can reach it via
/// `State<LiveSessionState>`. The runtime itself is `Arc`-wrapped, so
/// cloning the handle is cheap. `Mutex<bool>` guards the start/stop
/// lifecycle flag — hot-path reads (`snapshot`, `subscribe_aggregate`)
/// go through the runtime's internal `Arc` state directly.
pub struct LiveSessionState {
    pub runtime: Arc<claudepot_core::session_live::LiveRuntime>,
    /// Has `session_live_start` been called yet? The runtime's own
    /// `start` is idempotent, but tracking the flag here lets the
    /// command skip the spawn cost when the webview invokes it
    /// multiple times in quick succession (React StrictMode
    /// double-mount, dev hot reload).
    pub started: Mutex<bool>,
    /// Handles to the bridge tasks spawned by `session_live_start`
    /// (aggregate → live-all emit, per-session → live::<sid>). Each
    /// `session_live_stop` aborts and clears them so a subsequent
    /// `start` doesn't accumulate ghost emitters. The session-detail
    /// handle is keyed by session_id so `session_live_unsubscribe`
    /// can cancel one subscriber without tearing down the aggregate
    /// bridge.
    pub bridge_tasks: Mutex<BridgeTasks>,
}

/// Handles owned by the state so lifecycle commands can cancel them.
#[derive(Default)]
pub struct BridgeTasks {
    pub aggregate: Option<tokio::task::JoinHandle<()>>,
    pub details: std::collections::HashMap<String, tokio::task::JoinHandle<()>>,
}

impl BridgeTasks {
    pub fn abort_all(&mut self) {
        if let Some(h) = self.aggregate.take() {
            h.abort();
        }
        for (_, h) in self.details.drain() {
            h.abort();
        }
    }
}

impl Default for LiveSessionState {
    fn default() -> Self {
        Self {
            runtime: claudepot_core::session_live::LiveRuntime::new(),
            started: Mutex::new(false),
            bridge_tasks: Mutex::new(BridgeTasks::default()),
        }
    }
}
