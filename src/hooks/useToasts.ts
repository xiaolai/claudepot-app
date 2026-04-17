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
};

let toastCounter = 0;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
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
    setToasts((t) => t.filter((x) => x.id !== id));
    timersRef.current.delete(id);
  }, []);

  const dismissToast = useCallback((id: number) => {
    const timer = timersRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timersRef.current.delete(id);
    }
    setToasts((t) => t.map((x) => (x.id === id ? { ...x, exiting: true } : x)));
    setTimeout(() => removeToast(id), 150);
  }, [removeToast]);

  /**
   * Push a toast. Options:
   *   - `onUndo` — renders an Undo button. The toast sticks around for
   *     the `undoMs` window (default 3000 ms) before auto-dismissing.
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
        undoLabel?: string;
        onCommit?: () => void;
      },
    ) => {
      toastCounter += 1;
      const id = toastCounter;
      const wrappedUndo = onUndo
        ? () => {
            onUndo();
          }
        : undefined;
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
        },
      ]);
      if (kind === "info") {
        const delay = onUndo ? opts?.undoMs ?? 3000 : 4000;
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

  return { toasts, pushToast, dismissToast };
}
