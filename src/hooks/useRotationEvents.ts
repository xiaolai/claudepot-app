import { useCallback, useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { api } from "../api";
import type { PendingSwap } from "../api/rotation";
import { dispatchOsNotification } from "../lib/notify";

/**
 * Listen for rotation orchestrator events. Three channels:
 *
 * - `rotation-suggested` — confirm-mode rule fired; user must
 *   approve. Renders a toast + OS banner offering "Switch" /
 *   "Dismiss." Clicking Switch invokes `rotation_apply_pending`.
 * - `rotation-applied` — auto-mode rule fired and the swap
 *   succeeded (or confirm-mode resolved). Surfaces an info toast so
 *   the user notices their account changed.
 * - `rotation-failed` — swap attempt failed. Surfaces an error toast;
 *   the audit log carries the full reason.
 *
 * This hook is wired once at the app root next to
 * `useUsageThresholdNotifications`.
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

/** Toast options matching `useToasts.pushToast`'s shape — passed
 *  through verbatim. */
interface ToastOpts {
  undoMs?: number;
  durationMs?: number;
  undoLabel?: string;
  onCommit?: () => void;
  dedupeKey?: string;
}

type PushToastFn = (
  kind: "info" | "error",
  text: string,
  onUndo?: () => void,
  opts?: ToastOpts,
) => void;

export function useRotationEvents(pushToast: PushToastFn): void {
  const handleSuggested = useCallback(
    (p: SuggestedPayload) => {
      const summary = `${p.fromEmail} → ${p.toEmail}`;
      const title = "Auto-rotation suggested";
      const body = `${summary} (${p.utilizationPct.toFixed(1)}% on ${
        p.window ?? "trigger window"
      })`;
      // The toast carries the Switch action; auto-dismiss after 30s
      // calls dismissPending so the orchestrator's stash drops it.
      // Re-using the `onUndo` slot keeps us within the existing toast
      // primitive rather than adding a second action shape.
      pushToast(
        "info",
        `Auto-rotation: ${summary} — utilization ${p.utilizationPct.toFixed(1)}%`,
        () => {
          void api
            .rotationApplyPending(p.swapId)
            .catch((e) =>
              pushToast("error", `Rotation swap failed: ${e}`),
            );
        },
        {
          undoLabel: "Switch",
          undoMs: 30_000,
          onCommit: () => {
            void api.rotationDismissPending(p.swapId);
          },
          dedupeKey: `rotation:suggested:${p.swapId}`,
        },
      );
      // OS notification only when the window is unfocused — when
      // it's focused, the in-app toast already carries the signal,
      // and dispatching both is "status spray" per design.md.
      if (typeof document !== "undefined" && document.hasFocus()) {
        return;
      }
      void dispatchOsNotification(title, body, {
        dedupeKey: `rotation:suggested:${p.swapId}`,
        group: "rotation",
        sound: "default",
        target: { kind: "app", route: { section: "settings" } },
      });
    },
    [pushToast],
  );

  const handleApplied = useCallback(
    (p: AppliedPayload) => {
      pushToast(
        "info",
        `Auto-rotation: switched to ${p.toEmail} (rule ${p.ruleId})`,
      );
    },
    [pushToast],
  );

  const handleFailed = useCallback(
    (p: FailedPayload) => {
      pushToast(
        "error",
        `Auto-rotation failed: ${p.toEmail} (${p.error})`,
      );
    },
    [pushToast],
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
