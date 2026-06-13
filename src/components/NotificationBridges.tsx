import { ActivityNotificationsBridge } from "../hooks/useActivityNotifications";
import { useCardNotifications } from "../sections/events/useCardNotifications";
import { useOpDoneNotifications } from "../hooks/useOpDoneNotifications";
import { useUsageThresholdNotifications } from "../hooks/useUsageThresholdNotifications";
import { useRotationEvents } from "../hooks/useRotationEvents";
import { useAgentEventToasts } from "../hooks/useAgentEventToasts";
import { useBackgroundChangeEmits } from "../hooks/useBackgroundChangeEmits";

/**
 * Null-rendering leaf that hosts every ambient notification hook the
 * shell used to call inline. Mounting them in a leaf keeps their
 * internal subscriptions (most importantly the raw live-session list
 * inside ActivityNotificationsBridge) from re-rendering the whole
 * shell tree — AppShell consumes only the primitive
 * `useActivityAlertCount()` snapshot.
 *
 * Hooks hosted here:
 *
 * - Activity notifications — OS notifications for user-enabled
 *   trigger classes (error burst, idle-after-work, stuck, waiting).
 * - Card-level OS notifications — Warn+ CardEmitted deltas gated by
 *   `notify_on_error`; coalesces same-title bursts (≥3 in 60 s →
 *   one summary). See src/sections/events/useCardNotifications.ts.
 * - Op-completion OS notifications — long-running op (verify_all,
 *   project rename, session prune/slim/share/move, account
 *   login/register, clean projects, agent run) terminating while
 *   the window is unfocused, gated by `notify_on_op_done`.
 * - Usage-threshold OS notifications — `usage-threshold-crossed`
 *   from the Rust-side `usage_watcher` task; the watcher enforces
 *   the once-per-(window × threshold) per-cycle policy.
 * - Auto-rotation events — confirm-mode rules surface as a toast
 *   with a Switch action; auto-mode swaps as info toasts; failures
 *   as errors. See src/hooks/useRotationEvents.ts.
 * - Agent-event orchestrator toasts — firing failures and the
 *   first-tick catch-up cap. The successful-fire dispatched event is
 *   intentionally NOT subscribed — runs land in the run-history
 *   panel with structured output, so a toast per fire would spam.
 * - Background-change emits — `memory:changed` and
 *   `config-tree-patch` routed into the notification log so they
 *   reach the bell popover regardless of the active section.
 *   P3 ambient — log-only, no toast/banner spray.
 */
export function NotificationBridges() {
  useCardNotifications();
  useOpDoneNotifications();
  useUsageThresholdNotifications();
  useRotationEvents();
  useAgentEventToasts();
  useBackgroundChangeEmits();
  return <ActivityNotificationsBridge />;
}
