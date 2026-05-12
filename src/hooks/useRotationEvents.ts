import { useCallback, useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { api } from "../api";
import type { PendingSwap } from "../api/rotation";
import { useEmit } from "../providers/AppStateProvider";

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
}

interface FailedPayload {
  ruleId: string;
  fromEmail: string;
  toEmail: string;
  error: string;
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
      void emit({
        category: "rotationApplied",
        title: "Auto-rotation applied",
        body: `Switched to ${p.toEmail} (rule ${p.ruleId})`,
        dedupeKey: `rotation:applied:${p.ruleId}`,
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

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    const wire = <T>(channel: string, handler: (p: T) => void) => {
      void listen<T>(channel, (ev) => {
        if (!active || !ev.payload) return;
        handler(ev.payload);
      })
        .then((fn) => {
          if (!active) fn();
          else unlisteners.push(fn);
        })
        .catch(() => {
          /* non-tauri env */
        });
    };

    wire<SuggestedPayload>("rotation-suggested", handleSuggested);
    wire<AppliedPayload>("rotation-applied", handleApplied);
    wire<FailedPayload>("rotation-failed", handleFailed);

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
      unlisteners.forEach((fn) => fn());
    };
  }, [handleSuggested, handleApplied, handleFailed]);
}
