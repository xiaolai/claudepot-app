//! App-wide state held across Tauri command invocations.
//!
//! Today there is one piece of shared state: the Notify handle for an
//! in-flight `claude auth login` subprocess, so a Cancel button click can
//! reach back and kill it. Kept in a std Mutex because accesses are
//! non-async and very brief.

use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as AsyncMutex, Notify};

/// Process-wide async mutex that serializes every Desktop-mutating
/// Tauri command (adopt, clear, use, launch, quit, sync).
/// Layered on top of the core `desktop_lock` flock so in-process
/// concurrency doesn't repeatedly hit the filesystem.
///
/// See plan v2 §Operation locking + Codex D2-1: the GUI can fire
/// multiple Tauri commands concurrently (tray + main window), and the
/// SQLite mutex alone does not guard disk mutations.
#[derive(Default)]
pub struct DesktopOpState(pub AsyncMutex<()>);

/// Process-wide async mutex that serializes tray-initiated CLI
/// switches. Without it, two rapid tray clicks (A→B, then A→C before
/// the first swap finishes) can both snapshot the same pre-switch
/// active account, leaking a stale `from_email` into the second
/// switch's `tray-cli-switched` payload — Undo would jump to A
/// instead of B. Locking around the (snapshot + cli_use + emit)
/// sequence makes each tray switch see the result of the prior one.
///
/// Tray-scoped on purpose: in-window CLI swaps go through a different
/// surface where the user can only have one click in flight at a
/// time anyway.
#[derive(Default)]
pub struct CliOpState(pub AsyncMutex<()>);

/// Tracks the currently running `claude auth login` flow, if any.
/// `None` when idle; `Some(notify)` when a login is in progress and
/// calling `notify.notify_one()` will abort it.
///
/// The slot is wrapped in `Arc<Mutex<...>>` so spawned worker threads
/// can hold a cheap clone-handle that talks to the SAME slot without
/// keeping a `tauri::State` borrow alive across the spawn boundary.
/// See [`LoginStateHandle`] below.
pub struct LoginState {
    pub active: Arc<Mutex<Option<Arc<Notify>>>>,
}

impl Default for LoginState {
    fn default() -> Self {
        Self {
            active: Arc::new(Mutex::new(None)),
        }
    }
}

impl LoginState {
    /// Build a sharable handle to the slot so spawned worker threads can
    /// release the lock when they finish — without holding a
    /// `tauri::State<'_, LoginState>` borrow across thread spawn.
    pub fn clone_handle(&self) -> LoginStateHandle {
        LoginStateHandle {
            slot: Arc::clone(&self.active),
        }
    }
}

/// Handle that points back to the same `LoginState::active` slot.
/// `clear()` releases the in-process login lock so the next start can
/// run — used by the spawned worker thread on terminal.
#[derive(Clone)]
pub struct LoginStateHandle {
    slot: Arc<Mutex<Option<Arc<Notify>>>>,
}

impl LoginStateHandle {
    /// Drop the active login slot. Idempotent — safe to call multiple
    /// times even if no login was running.
    pub fn clear(&self) {
        if let Ok(mut g) = self.slot.lock() {
            g.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Concurrency guard: once a login is registered, the next start
    /// must observe `slot.is_some()` and reject. The handle's `clear()`
    /// must release the slot so a follow-up start can take it.
    #[test]
    fn login_state_handle_clear_releases_slot() {
        let state = LoginState::default();
        let handle = state.clone_handle();

        // Simulate a start: install a Notify in the slot.
        let n = Arc::new(Notify::new());
        {
            let mut g = state.active.lock().unwrap();
            assert!(g.is_none(), "fresh state should be empty");
            *g = Some(n.clone());
        }

        // Second-start guard: peek at the slot via the handle's same
        // pointer; it must still be Some.
        assert!(
            state.active.lock().unwrap().is_some(),
            "slot must be occupied between start and clear"
        );

        // Worker thread reaches terminal — clear via the handle.
        handle.clear();
        assert!(
            state.active.lock().unwrap().is_none(),
            "clear() must drain the slot"
        );

        // Clear is idempotent.
        handle.clear();
        assert!(state.active.lock().unwrap().is_none());
    }
}

/// Holds the dry-run service that arbitrates concurrent
/// `project_move_dry_run` calls under last-call-wins semantics. The
/// service itself is `Arc<DryRunService>`; this newtype just lets
/// Tauri manage it via `State<'_, DryRunState>` without colliding
/// with other `Arc<...>` managed types.
pub struct DryRunState(pub Arc<claudepot_core::project_dry_run_service::DryRunService>);

impl DryRunState {
    pub fn new() -> Self {
        Self(claudepot_core::project_dry_run_service::DryRunService::new())
    }
}

impl Default for DryRunState {
    fn default() -> Self {
        Self::new()
    }
}

/// Wraps `LiveActivityService` so Tauri commands can reach it via
/// `State<LiveSessionState>`. The service owns the runtime, listener
/// fan-out, membership-debounce, and bridge-task lifecycle — every
/// piece of policy that previously lived in `commands_activity.rs`.
pub struct LiveSessionState {
    pub service: Arc<claudepot_core::services::live_activity_service::LiveActivityService>,
}

impl LiveSessionState {
    pub fn new(
        service: Arc<claudepot_core::services::live_activity_service::LiveActivityService>,
    ) -> Self {
        Self { service }
    }
}

/// Live count of "alerting" sessions (errored or stuck) reflected in
/// the tray. The frontend pushes the count via `tray_set_alert_count`;
/// `tray::rebuild` reads it back so the count survives full menu
/// rebuilds (account changes, sync events). Default 0.
///
/// Wrapped in a `std::sync::Mutex` because reads are sync and very
/// brief — both the command handler and `rebuild()` only hold the
/// guard long enough to clone a `u32`.
#[derive(Default)]
pub struct TrayAlertState(pub Mutex<u32>);

impl TrayAlertState {
    pub fn get(&self) -> u32 {
        // Recover from poison rather than panic — a poisoned alert
        // counter is a UI-only concern and must not propagate.
        match self.0.lock() {
            Ok(g) => *g,
            Err(p) => *p.into_inner(),
        }
    }

    pub fn set(&self, count: u32) {
        match self.0.lock() {
            Ok(mut g) => *g = count,
            Err(p) => *p.into_inner() = count,
        }
    }
}
