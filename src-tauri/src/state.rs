//! App-wide state held across Tauri command invocations.
//!
//! Today there is one piece of shared state: the Notify handle for an
//! in-flight `claude auth login` subprocess, so a Cancel button click can
//! reach back and kill it. Kept in a std Mutex because accesses are
//! non-async and very brief.

use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Tracks the currently running `claude auth login` flow, if any.
/// `None` when idle; `Some(notify)` when a login is in progress and
/// calling `notify.notify_one()` will abort it.
#[derive(Default)]
pub struct LoginState {
    pub active: Mutex<Option<Arc<Notify>>>,
}
