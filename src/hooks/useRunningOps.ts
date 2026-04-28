import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { RunningOpInfo } from "../types";

const POLL_INTERVAL_MS = 3_000;

/**
 * Polls the backend for currently-tracked ops so the RunningOpStrip
 * stays in sync even if a Tauri event is lost. Polling is cheap —
 * pure HashMap lookup behind the Tauri command. Stops when the list
 * is empty to avoid a useless tick loop, and auto-rearms whenever
 * the next refresh sees ops appear (e.g. one was started from the
 * CLI, another window, or a `_start` Tauri command in this window
 * that already calls refresh() to pick the op up).
 *
 * Callers can force an immediate refresh after firing a `_start`
 * command; the poll-based fallback exists for event drops, not as
 * the primary update path.
 */
export function useRunningOps(): {
  ops: RunningOpInfo[];
  refresh: () => void;
} {
  const [ops, setOps] = useState<RunningOpInfo[]>([]);
  const intervalRef = useRef<number | null>(null);

  const refresh = useCallback(() => {
    api
      .runningOpsList()
      .then((list) => {
        setOps(list);
        // Idle-aware polling: stop the interval when nothing is in
        // flight, restart it the moment something appears. The
        // self-reference is safe — useCallback with [] gives `refresh`
        // a stable identity, and JS resolves the name at call time.
        if (list.length === 0) {
          if (intervalRef.current !== null) {
            window.clearInterval(intervalRef.current);
            intervalRef.current = null;
          }
        } else if (intervalRef.current === null) {
          intervalRef.current = window.setInterval(refresh, POLL_INTERVAL_MS);
        }
      })
      .catch((err) => {
        console.warn("useRunningOps refresh failed", err);
      });
  }, []);

  useEffect(() => {
    // Defer the first tick past first paint (see usePendingJournals).
    // The running-op strip only lights up when an op is in flight —
    // we can wait one idle slot for it on cold start.
    const rIC: (cb: () => void) => number =
      (window as typeof window & {
        requestIdleCallback?: (cb: () => void) => number;
      }).requestIdleCallback ??
      ((cb) => window.setTimeout(cb, 250));
    const cIC: (h: number) => void =
      (window as typeof window & {
        cancelIdleCallback?: (h: number) => void;
      }).cancelIdleCallback ?? window.clearTimeout;

    const idleHandle = rIC(() => refresh());
    return () => {
      cIC(idleHandle);
      if (intervalRef.current !== null) {
        window.clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [refresh]);

  return { ops, refresh };
}
