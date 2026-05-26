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
    ///
    /// Mirrors the poison-recovery policy used by the SQLite stores
    /// (`account.rs`, `env_vault/store.rs`, `keys/store.rs`,
    /// `session_live/metrics_store.rs`) and `notification_log`: if the
    /// mutex is poisoned by an earlier worker-thread panic, recover via
    /// `into_inner()` and clear the slot anyway. The previous
    /// `if let Ok(mut g) = self.slot.lock()` shape silently no-oped on
    /// poison, leaving the in-process login lock held forever — every
    /// subsequent `account_login_start` would then reject with "login
    /// already in progress" for the process lifetime. The whole point
    /// of `clear()` existing is to make sure that lock gets released
    /// even when something blows up upstream, so the policy is to
    /// recover and clear.
    pub fn clear(&self) {
        let mut g = match self.slot.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!(
                    "LoginStateHandle::clear: slot mutex was poisoned by an earlier panic; \
                     clearing anyway so the login slot doesn't stay held forever"
                );
                poisoned.into_inner()
            }
        };
        g.take();
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

    /// Poison-recovery: if a worker thread panics while holding the
    /// slot's mutex, the slot is poisoned for the process lifetime.
    /// `clear()` must still drain the slot anyway — otherwise every
    /// future `account_login_start` would reject with "login already
    /// in progress." This test simulates the panic-while-holding by
    /// poisoning the mutex explicitly via a panicking child thread.
    #[test]
    fn login_state_handle_clear_recovers_from_poisoned_mutex() {
        let state = LoginState::default();
        let handle = state.clone_handle();

        // Install something in the slot so we can confirm clear()
        // actually emptied it (rather than the slot just happening to
        // be empty).
        {
            let mut g = state.active.lock().unwrap();
            *g = Some(Arc::new(Notify::new()));
        }

        // Poison the mutex: spawn a thread, lock, panic.
        let slot_for_panic = Arc::clone(&state.active);
        let join = std::thread::spawn(move || {
            let _g = slot_for_panic.lock().unwrap();
            panic!("intentional panic to poison the mutex");
        });
        let _ = join.join();
        // Confirm the mutex is actually poisoned now.
        assert!(
            state.active.is_poisoned(),
            "test setup: mutex must be poisoned"
        );

        // The fix: clear() recovers via `into_inner()` and drains the
        // slot, so a subsequent login_start can take it. Before the
        // fix, this assertion failed — the `if let Ok(...)` branch was
        // skipped silently and the slot stayed occupied.
        handle.clear();
        let g = state.active.lock();
        // The mutex remains poisoned (recovery doesn't unpoison it),
        // but we can still read the slot through `into_inner` on a
        // poisoned guard. Use the same recovery pattern in the
        // assertion.
        let inner = match g {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        assert!(
            inner.is_none(),
            "clear() must drain the slot even on poison"
        );
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

/// Live count of "available updates" reflected in the tray. Separate
/// from [`TrayAlertState`] so the activity-alert and updates-alert
/// channels don't trample each other — `tray::refresh_alert_chrome`
/// sums both. The poller writes here after each cycle (see
/// `claudepot_core::updates::poller::CheckCycleOutcome`).
///
/// Counter scheme: 1 per surface (CLI / Desktop) with an available
/// update AND `notify_on_available` set. So 0..=2.
#[derive(Default)]
pub struct UpdatesAlertState(pub Mutex<u32>);

impl UpdatesAlertState {
    pub fn get(&self) -> u32 {
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

/// Last-known severity from the `claude doctor` scrape. Surfaced in
/// the tray menu so a closed window still gets a glanceable CC
/// health signal. Separate from [`TrayAlertState`] / [`UpdatesAlertState`]
/// because the policy is different — health does NOT escalate the
/// alert-template tray icon (the dot template is owned by alerting
/// sessions / available updates), only the menu copy. Cut 3 scope:
/// menu entry only; an icon variant for health is a follow-up.
///
/// Initial state is `Unknown`, not `Healthy` — we don't want the
/// menu to claim "All systems go" until the first scrape returns a
/// verdict. The first refresh arrives within seconds via the
/// renderer's pill; the background poller in `cc_doctor_watcher`
/// is the closed-window fallback.
pub struct TrayHealthState(pub Mutex<HealthRecord>);

#[derive(Clone, Copy)]
pub struct HealthRecord {
    pub kind: HealthRecordKind,
    /// Count of sections flagged at warning/error. Lets the menu
    /// copy say "Health: 4 issues" without re-reading the snapshot.
    pub flagged_sections: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HealthRecordKind {
    Unknown,
    Healthy,
    Warning,
    Error,
}

impl Default for TrayHealthState {
    fn default() -> Self {
        Self(Mutex::new(HealthRecord {
            kind: HealthRecordKind::Unknown,
            flagged_sections: 0,
        }))
    }
}

impl TrayHealthState {
    pub fn get(&self) -> HealthRecord {
        match self.0.lock() {
            Ok(g) => *g,
            Err(p) => *p.into_inner(),
        }
    }

    pub fn set(&self, kind: HealthRecordKind, flagged_sections: u32) {
        let rec = HealthRecord {
            kind,
            flagged_sections,
        };
        match self.0.lock() {
            Ok(mut g) => *g = rec,
            Err(p) => *p.into_inner() = rec,
        }
    }
}
