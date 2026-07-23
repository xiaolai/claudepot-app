import { useCallback, useEffect } from "react";
import type { Event as TauriEvent } from "@tauri-apps/api/event";

import { api } from "../api";
import type { PendingSwap } from "../api/rotation";
import { useEmit } from "../providers/AppStateProvider";
import { useTauriEvents } from "./useTauriEvent";

/**
 * Listen for rotation orchestrator events. Three channels:
 *
 * - `rotation-suggested` — confirm-mode rule fired; user must
 *   approve. The category routes to BOTH toast (carries the
 *   actionable "Switch" button) and OS banner (P1 default,
 *   focus-gated by the OS dispatcher). Single log entry per event
 *   — the Phase 1 emit() facade owns logging.
 * - `rotation-applied` — auto-mode rule fired and the swap
 *   succeeded (or confirm-mode resolved). Acknowledge-level toast.
 *   In auto-rotation mode, the toast is silenced (the user opted in
 *   to silent rotation); the bell entry still lands.
 * - `rotation-failed` — swap attempt failed. Error toast. Separate
 *   category from `rotationApplied` so a user can mute applied-acks
 *   while still hearing failures.
 * - `rotation-breaker-tripped` — a rule's swap failed enough
 *   consecutive times that its circuit breaker quarantined it. The
 *   rule stops re-firing until the breaker's cooldown probe. Routes
 *   to the `rotationFailed` category (an error-level event) so it
 *   reaches the same toast + bell facade as a plain failure.
 * - `rotation-stalled` — a rule matched but every candidate is also
 *   at/above the threshold, so there is no safe target. Emitted once
 *   per stall episode; routed through `rotationFailed` (same channel
 *   precedent as breaker-tripped) so the user learns "every account
 *   is near cap" instead of only seeing an audit-log row.
 *
 * This hook is wired once at the app root next to
 * `useUsageThresholdNotifications`. It reads the emit() dispatcher
 * from AppStateProvider — no parameter threading.
 */

interface SuggestedPayload {
  swapId: string;
  ruleId: string;
  fromEmail: string;
  toEmail: string;
  fromUuid: string;
  toUuid: string;
  window: string | null;
  utilizationPct: number;
  thresholdPct: number;
}

interface AppliedPayload {
  ruleId: string;
  fromEmail: string;
  toEmail: string;
  /**
   * Whether a Claude Code process was running when the swap landed.
   * A swap applied mid-session only takes effect once CC restarts
   * (CC holds the old credentials in memory), so the toast tells the
   * user to restart. Absent/false → the swap is already live.
   */
  ccRunning?: boolean;
}

interface StalledPayload {
  ruleId: string;
  fromEmail: string;
  window: string | null;
  utilizationPct: number;
  thresholdPct: number;
}

interface FailedPayload {
  ruleId: string;
  fromEmail: string;
  toEmail: string;
  error: string;
}

interface BreakerTrippedPayload {
  ruleId: string;
  fromEmail: string;
  toEmail: string;
  consecutiveFailures: number;
}

export function useRotationEvents(): void {
  const emit = useEmit();

  const handleSuggested = useCallback(
    (p: SuggestedPayload) => {
      const summary = `${p.fromEmail} → ${p.toEmail}`;
      void emit({
        category: "rotationSuggested",
        title: "Auto-rotation suggested",
        body: `${summary} — utilization ${p.utilizationPct.toFixed(1)}% on ${
          p.window ?? "trigger window"
        }`,
        dedupeKey: `rotation:suggested:${p.swapId}`,
        target: { kind: "app", route: { section: "settings" } },
        toastAction: {
          label: "Switch",
          // 30 s — long enough for the user to notice and act, short
          // enough that a stale suggestion auto-dismisses with the
          // dismiss callback firing.
          timeoutMs: 30_000,
          onPress: () => {
            void api.rotationApplyPending(p.swapId).catch((e) =>
              // Re-emit as a failure event so the bell records it.
              emit({
                category: "rotationFailed",
                kind: "error",
                title: "Rotation swap failed",
                body: String(e),
                dedupeKey: `rotation:failed:${p.swapId}`,
              }),
            );
          },
          onCommit: () => {
            void api.rotationDismissPending(p.swapId);
          },
        },
      });
    },
    [emit],
  );

  const handleApplied = useCallback(
    (p: AppliedPayload) => {
      // A swap that lands while CC is running doesn't take effect until
      // CC restarts — surface that so the user isn't surprised the
      // running session is still on the old account.
      const restart = p.ccRunning
        ? " — restart Claude Code to apply"
        : "";
      void emit({
        category: "rotationApplied",
        title: "Auto-rotation applied",
        body: `Switched to ${p.toEmail} (rule ${p.ruleId})${restart}`,
        dedupeKey: `rotation:applied:${p.ruleId}`,
      });
    },
    [emit],
  );

  const handleStalled = useCallback(
    (p: StalledPayload) => {
      void emit({
        category: "rotationFailed",
        kind: "error",
        title: "Auto-rotation stalled",
        body: `Rule "${p.ruleId}" can't rotate — every candidate is at or above ${
          p.thresholdPct
        }% on ${p.window ?? "the trigger window"}. No safe target.`,
        dedupeKey: `rotation:stalled:${p.ruleId}`,
        target: { kind: "app", route: { section: "settings" } },
      });
    },
    [emit],
  );

  const handleFailed = useCallback(
    (p: FailedPayload) => {
      void emit({
        category: "rotationFailed",
        kind: "error",
        title: "Auto-rotation failed",
        body: `${p.toEmail}: ${p.error}`,
        dedupeKey: `rotation:failed:${p.ruleId}`,
      });
    },
    [emit],
  );

  const handleBreakerTripped = useCallback(
    (p: BreakerTrippedPayload) => {
      void emit({
        category: "rotationFailed",
        kind: "error",
        title: "Auto-rotation rule paused",
        body: `Rule "${p.ruleId}" was paused after ${p.consecutiveFailures} consecutive failed swaps. It will retry automatically after a cooldown.`,
        dedupeKey: `rotation:breaker:${p.ruleId}`,
        target: { kind: "app", route: { section: "settings" } },
      });
    },
    [emit],
  );

  // Lifetime subscriptions via the shared multi-channel primitive —
  // handlers are held in a ref, so the unstable useCallback
  // identities above never re-wire the channels.
  useTauriEvents({
    "rotation-suggested": (ev: TauriEvent<SuggestedPayload>) => {
      if (ev.payload) handleSuggested(ev.payload);
    },
    "rotation-applied": (ev: TauriEvent<AppliedPayload>) => {
      if (ev.payload) handleApplied(ev.payload);
    },
    "rotation-failed": (ev: TauriEvent<FailedPayload>) => {
      if (ev.payload) handleFailed(ev.payload);
    },
    "rotation-breaker-tripped": (ev: TauriEvent<BreakerTrippedPayload>) => {
      if (ev.payload) handleBreakerTripped(ev.payload);
    },
    "rotation-stalled": (ev: TauriEvent<StalledPayload>) => {
      if (ev.payload) handleStalled(ev.payload);
    },
  });

  useEffect(() => {
    let active = true;

    // Hydrate any pending swaps the orchestrator queued while the
    // renderer was disconnected (between reloads, before mount,
    // etc.). Each becomes a Switch toast on the same path live
    // events take. The orchestrator's TTL has already evicted any
    // stale entries before we read.
    void api
      .rotationPendingList()
      .then((pendings: PendingSwap[]) => {
        if (!active) return;
        for (const p of pendings) {
          handleSuggested({
            swapId: p.swapId,
            ruleId: p.ruleId,
            fromEmail: p.fromEmail,
            toEmail: p.toEmail,
            // The orchestrator doesn't expose uuids in
            // PendingSwapDto — they're only used for the
            // dedupe key, which the orchestrator already
            // applied on its side. Empty strings are safe in
            // the toast handler (it doesn't read them).
            fromUuid: "",
            toUuid: "",
            window: p.trigger.window,
            utilizationPct: p.trigger.utilizationPct,
            thresholdPct: p.trigger.thresholdPct,
          });
        }
      })
      .catch(() => {
        /* non-tauri env or no pending */
      });

    return () => {
      active = false;
    };
  }, [handleSuggested]);
}
