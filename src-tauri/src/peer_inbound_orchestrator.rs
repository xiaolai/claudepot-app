//! Closes the remote-control window when its deadline passes.
//!
//! Bridges `claudepot_core::peer::inbound` to the Tauri runtime. Like
//! `permission_orchestrator` it holds no managed state — the grant
//! lives on disk and is cheap to reload — and returns after one file
//! read when nothing is granted.
//!
//! ## The lock only covers this process, and that is not the whole story
//!
//! `permission-grants.json` is written by the GUI alone, so an
//! intra-process mutex fully excludes its writers. This file is
//! different: `claudepot session inbound` writes it too, and
//! `session send` calls `tick` on every invocation. A mutex here
//! serializes the GUI's own writers and nothing more.
//!
//! The interleaving that matters is a GUI revert landing between the
//! CLI's `open` and its save. The result is `crossSessionInbound:
//! accept` with no grant record — and because `decide` deliberately
//! refuses to revert a value it has no grant for (that value might be
//! the user's own choice), nothing would ever close it.
//!
//! That state is not silently tolerated: it is exactly what
//! `InboundState::is_unmanaged_open` reports, and the UI renders it as
//! open-and-unmanaged rather than as closed. Detect-and-tell beats
//! guess-and-revert here, because the alternative is overwriting a
//! setting the user may have chosen on purpose.

use chrono::Utc;
use claudepot_core::peer::inbound::{self, Decision};
use std::sync::MutexGuard;
use tauri::{AppHandle, Emitter};

/// Serializes this process's readers and writers: [`tick`] and the
/// `peer_inbound_*` commands.
///
/// **Delegates to core's lock rather than owning one.** It used to be a
/// `static` here, which made it invisible to `remote::server`'s
/// `/api/inbound` handlers — they touch the same two files and were not
/// serialized against these writers at all. Two mutexes guarding one
/// pair of files is not a lock; see `peer::inbound::file_guard`.
pub fn inbound_file_guard() -> MutexGuard<'static, ()> {
    inbound::file_guard()
}

/// Payload for [`crate::events::PEER_INBOUND_CLOSED`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct InboundClosed {
    /// `expired` — the deadline passed and we reverted.
    /// `superseded` — the setting was hand-changed; the record was
    /// dropped and the setting left alone.
    pub reason: &'static str,
}

/// Called from `usage_snapshot::run_tick`.
pub async fn tick(app: &AppHandle) {
    let _guard = inbound_file_guard();

    let decision = match inbound::tick(Utc::now()) {
        Ok(d) => d,
        Err(e) => {
            // Loud: whatever went wrong, a window may still be open.
            tracing::error!(
                error = %e,
                "peer_inbound_orchestrator: could not reconcile the remote-control \
                 window; crossSessionInbound may still be open"
            );
            return;
        }
    };

    let reason = match decision {
        Decision::Idle | Decision::Active { .. } => return,
        Decision::Revert { .. } => "expired",
        Decision::Superseded { .. } => "superseded",
    };

    tracing::info!(
        reason,
        "peer_inbound_orchestrator: remote-control window closed"
    );
    if let Err(e) = app.emit(crate::events::PEER_INBOUND_CLOSED, InboundClosed { reason }) {
        tracing::warn!(error = %e, "peer_inbound_orchestrator: emit failed");
    }
}
