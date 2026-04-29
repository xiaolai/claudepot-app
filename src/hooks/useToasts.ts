import { useCallback, useEffect, useRef, useState } from "react";

export type Toast = {
  id: number;
  kind: "info" | "error";
  text: string;
  exiting: boolean;
  /** Optional undo callback — shown as a button on the toast. */
  onUndo?: () => void;
  /** Label of the undo button. Defaults to "Undo". */
  undoLabel?: string;
  /**
   * Internal: fires when the toast auto-dismisses *without* the user
   * clicking Undo. Consumers use this to commit deferred actions so
   * the Undo-vs-commit race is eliminated by construction.
   */
  onCommit?: () => void;
  /**
   * Optional dedupe key. If a new toast is pushed with the same
   * `dedupeKey` as an existing one, the older toast is dismissed
   * silently (without running its onCommit) before the new one is
   * shown. Used for deferred actions that supersede each other —
   * e.g. rapid-fire Desktop switches.
   */
  dedupeKey?: string;
};

/**
 * Last toast that fully dismissed (post-exit-animation removal). Used
 * by the status bar's echo segment so the user can re-read what just
 * scrolled by without the message blocking the UI. The status bar
 * gates display on `at` — recent dismissals show, older ones fade out
 * naturally. `null` means no echo (either none has happened yet, or
 * the consumer chose to clear it).
 */
export type DismissedToast = {
  text: string;
  kind: "info" | "error";
  /** epoch-ms when the toast finished its exit animation. */
  at: number;
};

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [lastDismissed, setLastDismissed] =
    useState<DismissedToast | null>(null);
  const timersRef = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  // Clear all pending timers on unmount
  useEffect(() => {
    const timers = timersRef.current;
    return () => {
      for (const t of timers.values()) clearTimeout(t);
      timers.clear();
    };
  }, []);

  const removeToast = useCallback((id: number) => {
    // Capture the toast's text + kind before it leaves so the status
    // bar can echo it. We snapshot from the array INSIDE the setter so
    // we read the live value without taking a stale closure on
    // `toasts`.
    setToasts((t) => {
      const leaving = t.find((x) => x.id === id);
      if (leaving) {
        setLastDismissed({
          text: leaving.text,
          kind: leaving.kind,
          at: Date.now(),
        });
      }
      return t.filter((x) => x.id !== id);
    });
    timersRef.current.delete(id);
  }, []);

  const dismissToast = useCallback((id: number) => {
    const timer = timersRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timersRef.current.delete(id);
    }
    setToasts((t) => t.map((x) => (x.id === id ? { ...x, exiting: true } : x)));
    // Audit T4-4: the 150 ms exit-animation timer used to be a bare
    // setTimeout outside `timersRef`. If the component unmounted during
    // the exit window the timer would still fire `removeToast`,
    // calling `setState` on a dead component (React 18: warning;
    // future strict modes: error). Stash the exit timer in the same
    // map so the unmount cleanup clears it like any other.
    const exitTimer = setTimeout(() => {
      timersRef.current.delete(id);
      removeToast(id);
    }, 150);
    timersRef.current.set(id, exitTimer);
  }, [removeToast]);

  /**
   * Push a toast. Options:
   *   - `onUndo` — renders an Undo button. The toast sticks around for
   *     the `undoMs` window (default 3000 ms) before auto-dismissing.
   *     Undo windows are intentionally short: they are action-commit
   *     timers, not notifications.
   *   - `durationMs` — override the auto-dismiss timer for
   *     notifications (no `onUndo`). Default 10 000 ms for both info
   *     and error kinds — long enough to read without locking the UI
   *     under a persistent banner. Pass `Infinity` for sticky
   *     notifications that the user must close manually.
   *   - `onCommit` — a callback fired iff the toast auto-dismisses
   *     WITHOUT the user clicking Undo. This is the idiomatic way to
   *     schedule a deferred action: the commit and the dismissal are
   *     the same event, so "Undo is clickable ↔ action hasn't fired".
   *     Clicking Undo cancels the commit.
   */
  const pushToast = useCallback(
    (
      kind: Toast["kind"],
      text: string,
      onUndo?: () => void,
      opts?: {
        undoMs?: number;
        durationMs?: number;
        undoLabel?: string;
        onCommit?: () => void;
        dedupeKey?: string;
      },
    ) => {
      toastCounter += 1;
      const id = toastCounter;
      const wrappedUndo = onUndo
        ? () => {
            onUndo();
          }
        : undefined;
      // Dedupe: cancel any prior toast with the same dedupeKey. We
      // must also clear its timer so its onCommit doesn't still fire.
      // Callers rely on this for "latest wins" semantics (rapid-fire
      // Desktop switches must not all commit).
      if (opts?.dedupeKey) {
        setToasts((prev) => {
          const stale = prev.filter((t) => t.dedupeKey === opts.dedupeKey);
          for (const s of stale) {
            const timer = timersRef.current.get(s.id);
            if (timer) {
              clearTimeout(timer);
              timersRef.current.delete(s.id);
            }
          }
          return prev.filter((t) => t.dedupeKey !== opts.dedupeKey);
        });
      }
      setToasts((t) => [
        ...t,
        {
          id,
          kind,
          text,
          exiting: false,
          onUndo: wrappedUndo,
          undoLabel: opts?.undoLabel,
          onCommit: opts?.onCommit,
          dedupeKey: opts?.dedupeKey,
        },
      ]);
      // Auto-dismiss policy:
      //   onUndo  → short (undoMs, default 3 s) — undo is an action
      //             commit timer, not a notification.
      //   error   → sticky by default (Infinity). Errors carry copy
      //             worth screenshotting / dictating into a bug
      //             report; the auto-dismiss is the wrong default
      //             when the message is the diagnostic. The toast
      //             carries a close button + dedupeKey, so accidental
      //             accumulation is bounded by user dismissal.
      //   info    → durationMs (default 10 s) — short enough that
      //             stale acknowledgements don't pile up.
      //   Callers can override with explicit `durationMs` (Infinity
      //   for sticky, a number for finite).
      const delay = onUndo
        ? opts?.undoMs ?? 3000
        : opts?.durationMs ?? (kind === "error" ? Infinity : 10_000);
      if (Number.isFinite(delay)) {
        const timer = setTimeout(() => {
          // If the user never clicked Undo, run the commit callback
          // just before dismissing. This makes "toast visible ⇔ Undo
          // still effective" an invariant, eliminating the prior race
          // between a parallel action timer and the toast lifetime.
          opts?.onCommit?.();
          dismissToast(id);
        }, delay);
        timersRef.current.set(id, timer);
      }
    },
    [dismissToast],
  );

  /** Clear the echo. Used when the status bar's fade window elapses
   *  so the segment unmounts cleanly rather than re-rendering empty. */
  const clearLastDismissed = useCallback(() => setLastDismissed(null), []);

  return {
    toasts,
    pushToast,
    dismissToast,
    lastDismissed,
    clearLastDismissed,
  };
}
