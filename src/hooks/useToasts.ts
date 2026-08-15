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
   * Optional dedupe key. If a new toast is pushed with the same
   * `dedupeKey` as an existing one, the older toast is dismissed
   * silently (without running its onCommit) before the new one is
   * shown. Used for deferred actions that supersede each other —
   * e.g. rapid-fire Desktop switches.
   */
  dedupeKey?: string;
};

/**
 * The bell popover is the durable record of what happened; the
 * status bar used to replay a dismissed toast there for six seconds
 * as truncated, pointer-inert, aria-hidden text. Two surfaces for one
 * event, and the second could not be acted on. See rules/design.md's
 * signal budget.
 */

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timersRef = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());
  // Deferred commits live OUTSIDE the toast state so `dismissToast`
  // can run them without reading `toasts` (stale closure) and without
  // side effects inside a state updater (StrictMode double-invokes
  // updaters — a commit in there would fire twice).
  const commitsRef = useRef<Map<number, () => void>>(new Map());
  /** Toast ids whose manual dismissal cancels rather than commits. */
  const cancelOnDismissRef = useRef<Set<number>>(new Set());

  // Clear all pending timers on unmount. Pending commits are dropped,
  // not fired — teardown is not a user decision to commit.
  useEffect(() => {
    const timers = timersRef.current;
    const commits = commitsRef.current;
    const cancels = cancelOnDismissRef.current;
    return () => {
      for (const t of timers.values()) clearTimeout(t);
      timers.clear();
      commits.clear();
      // Cleared alongside the commits it annotates. Ids are never
      // reused, so a stale entry cannot mis-flag a later toast — this
      // is a leak, not a correctness bug. It still goes, because the
      // next reader has to work that out to know it is safe.
      cancels.clear();
    };
  }, []);

  const removeToast = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
    timersRef.current.delete(id);
  }, []);

  /**
   * Dismiss a toast. By default this COMMITS the toast's deferred
   * action first: the auto-dismiss timer and the manual close (X)
   * button both land here, and both mean "the user did not undo" —
   * an X-click must not silently cancel the action the toast said
   * was happening (audit F1: closing "Switching Desktop to X…"
   * dropped the switch). Only the Undo button passes
   * `skipCommit: true`; dedupe-supersede clears the commit without
   * ever reaching this path. Deleting from the map before invoking
   * makes re-entry (double-click, timer + X race) a no-op.
   */
  const dismissToast = useCallback(
    (id: number, opts?: { skipCommit?: boolean; byTimer?: boolean }) => {
      const commit = commitsRef.current.get(id);
      commitsRef.current.delete(id);
      // A destructive deferred action commits only when its window
      // actually elapses. A user closing the toast is cancelling, not
      // consenting — see `cancelOnDismiss`.
      const userCancelled =
        cancelOnDismissRef.current.has(id) && !opts?.byTimer;
      cancelOnDismissRef.current.delete(id);
      if (commit && !opts?.skipCommit && !userCancelled) commit();
      const timer = timersRef.current.get(id);
      if (timer) {
        clearTimeout(timer);
        timersRef.current.delete(id);
      }
      setToasts((t) =>
        t.map((x) => (x.id === id ? { ...x, exiting: true } : x)),
      );
      // Audit T4-4: the 150 ms exit-animation timer used to be a bare
      // setTimeout outside `timersRef`. If the component unmounted
      // during the exit window the timer would still fire
      // `removeToast`, calling `setState` on a dead component (React
      // 18: warning; future strict modes: error). Stash the exit timer
      // in the same map so the unmount cleanup clears it like any
      // other.
      const exitTimer = setTimeout(() => {
        timersRef.current.delete(id);
        removeToast(id);
      }, 150);
      timersRef.current.set(id, exitTimer);
    },
    [removeToast],
  );

  /**
   * Push a toast. Options:
   *   - `onUndo` — renders an Undo button. The toast sticks around for
   *     the `undoMs` window (default 3000 ms) before auto-dismissing.
   *     Undo windows are intentionally short: they are action-commit
   *     timers, not notifications.
   *   - `durationMs` — override the auto-dismiss timer for
   *     notifications (no `onUndo`). Defaults differ by kind:
   *     info → 10 000 ms (long enough to read without piling up),
   *     error → `Infinity` (sticky: the copy is often the diagnostic,
   *     so it must not vanish before the user acts on it). Pass a
   *     finite number to make an error auto-dismiss, or `Infinity` to
   *     make an info toast sticky.
   *   - `onCommit` — a callback fired exactly once when the toast
   *     leaves by any path EXCEPT Undo: auto-dismiss, the manual
   *     close (X) button, or a programmatic dismiss. This is the
   *     idiomatic way to schedule a deferred action: the commit and
   *     the dismissal are the same event, so "Undo is clickable ↔
   *     action hasn't fired". Clicking Undo — or being superseded by
   *     a same-`dedupeKey` toast — cancels the commit.
   *
   * **Visual primitive only.** Logging is owned by `emit()` in
   * `src/lib/notifications/dispatch.ts`. The public `pushToast`
   * exposed by `AppStateProvider` wraps emit() with a default
   * `category=configEdited`; this hook's `pushToast` is the
   * internal toast-rendering primitive that emit() drives.
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
        /**
         * Make the manual close (X) CANCEL the pending commit instead
         * of firing it. Off by default, so the existing deferred
         * actions (desktop switch) keep committing on dismiss.
         *
         * Set this for **destructive** deferred actions. The default
         * makes dismissing indistinguishable from confirming: a user
         * who has just been told "you have a few seconds to undo"
         * and then tidies the toast away destroys the thing
         * immediately, losing the window they were promised. For a
         * destructive action the safe reading of "close" is "I am
         * done looking at this", not "yes, proceed now".
         */
        cancelOnDismiss?: boolean;
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
            // Superseded, not dismissed: the stale commit must never
            // fire ("latest wins"), so drop it silently.
            commitsRef.current.delete(s.id);
            cancelOnDismissRef.current.delete(s.id);
          }
          return prev.filter((t) => t.dedupeKey !== opts.dedupeKey);
        });
      }
      if (opts?.onCommit) {
        commitsRef.current.set(id, opts.onCommit);
        if (opts.cancelOnDismiss) cancelOnDismissRef.current.add(id);
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
          dedupeKey: opts?.dedupeKey,
        },
      ]);
      // Logging removed in Phase 3: emit() in src/lib/notifications/
      // dispatch.ts owns the routed log entry. The public pushToast
      // exposed by AppStateProvider wraps emit() with a default
      // category; every notification flows through that pipeline.
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
          // `dismissToast` runs the commit (single run-once site via
          // `commitsRef`) before dismissing. This makes "toast
          // visible ⇔ Undo still effective" an invariant, eliminating
          // the prior race between a parallel action timer and the
          // toast lifetime.
          // `byTimer` marks this as the window genuinely elapsing, so
          // a destructive deferred action commits here and nowhere
          // else.
          dismissToast(id, { byTimer: true });
        }, delay);
        timersRef.current.set(id, timer);
      }
    },
    [dismissToast],
  );


  return {
    toasts,
    pushToast,
    dismissToast,
  };
}
