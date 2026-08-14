//! Event-channel names — the single home for every channel string
//! the backend emits to the webview.
//!
//! ## Convention for NEW events
//!
//! `domain::event` (double-colon namespace), with the per-instance
//! extension `domain::<id>` where a channel is scoped to one entity
//! (see [`op_progress_channel`] / [`live_channel`]). Pick the domain
//! from the owning module (`updates`, `service-status`, `live`, …).
//!
//! ## Legacy names are FROZEN
//!
//! Every constant below is a frontend wire contract — the renderer
//! subscribes by exact string (grep `src/` for the value before
//! touching one). Historical names predate the convention and span
//! three other schemes (bare kebab-case, a `cp-` prefix family, one
//! `memory:changed` single-colon form); they stay as-is. Migrate a
//! legacy name only when its listeners are being reworked anyway,
//! and update both sides in the same change.
//!
//! Channels emitted from files owned by other surfaces (tray /
//! app-menu / traffic-light chrome: `tray-cli-switched`,
//! `cp-activity-open-session`, `cp-quit-requested`,
//! `traffic-light-metrics`) are equally frozen; they keep their
//! literals at the emit site until those files are next reworked.

/// Per-op progress channel: `op-progress::<op_id>`. Carries
/// `ProgressEvent` (and, for VerifyAll, `VerifyAccountEvent`)
/// payloads; the op-progress modal subscribes by op_id.
pub fn op_progress_channel(op_id: &str) -> String {
    format!("op-progress::{op_id}")
}

/// Global op-terminal channel — one emission per op completion,
/// for notification-style consumers that don't know op_ids up-front.
pub const OP_TERMINAL: &str = "cp-op-terminal";

/// Updates watcher finished a check cycle; the Updates panel
/// re-reads `updates_status_get`.
pub const UPDATES_CYCLE_COMPLETE: &str = "updates::cycle-complete";

/// status.claude.com summary refreshed (success or failure) — a
/// refresh ping, payload-free.
pub const SERVICE_STATUS_UPDATED: &str = "service-status::updated";

/// Rotation rule fired in confirm mode; renderer shows the
/// suggestion toast.
pub const ROTATION_SUGGESTED: &str = "rotation-suggested";

/// Auto-mode rotation swap completed.
pub const ROTATION_APPLIED: &str = "rotation-applied";

/// Rotation swap attempt failed.
pub const ROTATION_FAILED: &str = "rotation-failed";

/// A rotation rule's swap kept failing and its circuit breaker
/// quarantined it.
pub const ROTATION_BREAKER_TRIPPED: &str = "rotation-breaker-tripped";

/// A rotation rule matched but found no safe target — every alternate
/// candidate is also at or above the threshold. Emitted once per
/// transition into the stalled state so the user learns "every account
/// is near cap" instead of only seeing an audit-log row.
pub const ROTATION_STALLED: &str = "rotation-stalled";

/// A permission grant was auto-reverted (or skipped because the
/// user hand-changed the setting).
pub const PERMISSION_REVERTED: &str = "permission-reverted";

/// A grant's auto-revert kept failing and its circuit breaker
/// quarantined it.
pub const PERMISSION_BREAKER_TRIPPED: &str = "permission-breaker-tripped";

/// CLI-active account crossed a configured usage threshold.
pub const USAGE_THRESHOLD_CROSSED: &str = "usage-threshold-crossed";

/// Credentials for some account were just healed — by the background
/// token-refresh orchestrator or a UI-driven verify — so the webview
/// should re-pull usage. Without this, a freshly-live token's numbers
/// only reach the in-memory cache + the snapshot file (neither of
/// which the GUI reads reactively), and the card keeps showing the
/// "token expired" placeholder until the next focus/manual refresh.
pub const USAGE_REFETCH: &str = "usage::refetch";

/// A CLAUDE.md / memory file changed on disk (legacy single-colon
/// form — frozen; see module doc).
pub const MEMORY_CHANGED: &str = "memory:changed";

/// An event-triggered agent run was dispatched.
pub const AGENT_EVENT_DISPATCHED: &str = "agent-event-dispatched";

/// An event-triggered agent dispatch failed.
pub const AGENT_EVENT_FAILED: &str = "agent-event-failed";

/// Event-agent dispatches were dropped by the per-tick burst cap.
pub const AGENT_EVENT_BURST_CAPPED: &str = "agent-event-burst-capped";

/// A watched config-tree file changed; payload is the tree patch.
pub const CONFIG_TREE_PATCH: &str = "config-tree-patch";

// `desktop-adopted`, `desktop-cleared` and `desktop-running-changed`
// used to live here. All three were emitted from the Tauri *command*
// the renderer had just invoked, so the caller already had the outcome
// in the command's return value and nobody ever subscribed. Deleted
// rather than wired up: an event whose only purpose is to re-announce
// what the caller just learned is noise, and a channel with no
// subscriber reads to the next author as a contract they must not
// break.
//
// The tray channels below are different — nothing returns to a caller
// there, because the click happened outside the webview.

/// Tray → Desktop swap succeeded. Payload: [`TrayDesktopSwitched`].
///
/// The tray performs the swap with `no_launch=true` and cannot render
/// anything itself, so this is the *only* route by which the main
/// window learns the Desktop binding moved.
pub const TRAY_DESKTOP_SWITCHED: &str = "tray-desktop-switched";

/// Tray → Desktop swap failed. Payload: the error message.
pub const TRAY_DESKTOP_SWITCH_FAILED: &str = "tray-desktop-switch-failed";

/// Tray → Launch Claude Desktop failed. Payload: the error message.
pub const TRAY_DESKTOP_LAUNCH_FAILED: &str = "tray-desktop-launch-failed";

/// Tray → Desktop flag reconcile finished. Payload: how many account
/// flags flipped, so the renderer can stay quiet when nothing changed.
pub const DESKTOP_RECONCILED: &str = "desktop-reconciled";

/// Payload of [`TRAY_DESKTOP_SWITCHED`].
///
/// Carries the account so the toast can name it. The emit used to be
/// `()`, which left the renderer unable to say *which* account it had
/// switched to even once it started listening — mirroring
/// `tray-cli-switched`, whose payload has always named the target.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrayDesktopSwitched {
    pub to_email: String,
}

/// Full live-session roster snapshot.
pub const LIVE_ALL: &str = "live-all";

/// Per-session live delta channel: `live::<session_id>`.
pub fn live_channel(session_id: &str) -> String {
    format!("live::{session_id}")
}

#[cfg(test)]
mod tests {
    //! # Why the old "wire-contract lock" test is gone
    //!
    //! It read `assert_eq!(DESKTOP_ADOPTED, "desktop-adopted")` — every
    //! constant compared to its own literal, twenty-one times. That
    //! catches a rename and nothing else, while its docstring claimed
    //! to protect a contract with the renderer.
    //!
    //! It could not, and did not. Seven of the channels it "locked"
    //! had **zero** subscribers in `src/`: four tray/Desktop ones where
    //! that meant swap failures vanished silently, and three that were
    //! pure dead weight. A tautology cannot notice the other end of the
    //! contract is missing, which is the only failure that had actually
    //! occurred.
    //!
    //! The real check is cross-boundary and now lives in
    //! `cargo xtask verify-docs`, which greps `src/` for every channel
    //! this file declares. What remains here is the part that is
    //! genuinely local: the naming convention for per-instance
    //! channels, and the shape of payloads the renderer destructures.

    use super::*;

    /// Payload shapes the renderer destructures by field name. Unlike
    /// the channel strings, these are not greppable from the frontend,
    /// so a serde rename would be invisible until runtime.
    #[test]
    fn tray_desktop_switched_serializes_the_account_it_switched_to() {
        let json = serde_json::to_value(TrayDesktopSwitched {
            to_email: "someone@example.com".into(),
        })
        .expect("serialize");
        assert_eq!(
            json.get("to_email").and_then(|v| v.as_str()),
            Some("someone@example.com"),
            "useTrayBridge reads `to_email`; renaming the field silently \
             degrades the toast to the unknown-account branch"
        );
    }

    #[test]
    fn test_per_instance_builders_use_double_colon_namespace() {
        assert_eq!(op_progress_channel("op-abc"), "op-progress::op-abc");
        assert_eq!(live_channel("sid-1"), "live::sid-1");
    }
}
