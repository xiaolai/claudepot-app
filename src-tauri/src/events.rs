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

/// A permission grant was auto-reverted (or skipped because the
/// user hand-changed the setting).
pub const PERMISSION_REVERTED: &str = "permission-reverted";

/// A grant's auto-revert kept failing and its circuit breaker
/// quarantined it.
pub const PERMISSION_BREAKER_TRIPPED: &str = "permission-breaker-tripped";

/// CLI-active account crossed a configured usage threshold.
pub const USAGE_THRESHOLD_CROSSED: &str = "usage-threshold-crossed";

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

/// Desktop slot adopted an account's session files.
pub const DESKTOP_ADOPTED: &str = "desktop-adopted";

/// Desktop slot cleared.
pub const DESKTOP_CLEARED: &str = "desktop-cleared";

/// Claude Desktop process started or stopped.
pub const DESKTOP_RUNNING_CHANGED: &str = "desktop-running-changed";

/// Full live-session roster snapshot.
pub const LIVE_ALL: &str = "live-all";

/// Per-session live delta channel: `live::<session_id>`.
pub fn live_channel(session_id: &str) -> String {
    format!("live::{session_id}")
}

#[cfg(test)]
mod tests {
    //! Wire-contract lock: the renderer subscribes by exact string,
    //! so any drift in these values is a frontend-visible break.
    //! Update `src/` listeners in the same change if one of these
    //! assertions ever needs to move.

    use super::*;

    #[test]
    fn test_channel_constants_match_frontend_contract() {
        assert_eq!(OP_TERMINAL, "cp-op-terminal");
        assert_eq!(UPDATES_CYCLE_COMPLETE, "updates::cycle-complete");
        assert_eq!(SERVICE_STATUS_UPDATED, "service-status::updated");
        assert_eq!(ROTATION_SUGGESTED, "rotation-suggested");
        assert_eq!(ROTATION_APPLIED, "rotation-applied");
        assert_eq!(ROTATION_FAILED, "rotation-failed");
        assert_eq!(ROTATION_BREAKER_TRIPPED, "rotation-breaker-tripped");
        assert_eq!(PERMISSION_REVERTED, "permission-reverted");
        assert_eq!(PERMISSION_BREAKER_TRIPPED, "permission-breaker-tripped");
        assert_eq!(USAGE_THRESHOLD_CROSSED, "usage-threshold-crossed");
        assert_eq!(MEMORY_CHANGED, "memory:changed");
        assert_eq!(AGENT_EVENT_DISPATCHED, "agent-event-dispatched");
        assert_eq!(AGENT_EVENT_FAILED, "agent-event-failed");
        assert_eq!(AGENT_EVENT_BURST_CAPPED, "agent-event-burst-capped");
        assert_eq!(CONFIG_TREE_PATCH, "config-tree-patch");
        assert_eq!(DESKTOP_ADOPTED, "desktop-adopted");
        assert_eq!(DESKTOP_CLEARED, "desktop-cleared");
        assert_eq!(DESKTOP_RUNNING_CHANGED, "desktop-running-changed");
        assert_eq!(LIVE_ALL, "live-all");
    }

    #[test]
    fn test_per_instance_builders_use_double_colon_namespace() {
        assert_eq!(op_progress_channel("op-abc"), "op-progress::op-abc");
        assert_eq!(live_channel("sid-1"), "live::sid-1");
    }
}
