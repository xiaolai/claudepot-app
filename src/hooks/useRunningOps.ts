import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { RunningOpInfo } from "../types";

const POLL_INTERVAL_MS = 3_000;

/**
 * Polls the backend for currently-tracked ops so the RunningOpStrip
 * stays in sync even if a Tauri event is lost. Polling is cheap —
 * pure HashMap lookup behind the Tauri command — and is the only
 * channel through which we discover ops started outside this window
 * (CLI process, second Tauri window). Pausing when the local list
 * is empty would mean a CLI-started op stays invisible until the
 * user does something to trigger refresh() manually, so we keep the
 * 3 s tick running unconditionally. The first tick is still
 * deferred past first paint to avoid competing with the boot frame.
 *
 * Callers can force an immediate refresh after firing a `_start`
 * command; the poll-based fallback exists for event drops AND for
 * cross-process discovery, not as the primary update path.
 */
export function useRunningOps(): {
  ops: RunningOpInfo[];
  refresh: () => void;
} {
  const [ops, setOps] = useState<RunningOpInfo[]>([]);

  const refresh = useCallback(() => {
    api
      .runningOpsList()
      .then((next) => {
        // Idle-path fast-skip: when both the prior and current list
        // are empty, the freshly-deserialized `[]` is structurally
        // identical to the prior state and committing it would force
        // an AppShell re-render every 3 s, cascading into every
        // consumer (the v0.1.37 "Projects detail shake" symptom).
        // For non-empty lists we always commit so progress updates
        // (current_phase, sub_progress, status, last_error, the
        // result payloads) reach the progress modal and status
        // strip — RunningOpInfo carries far more than the identity
        // fields, and the polling backstop is the event-drop
        // fallback path. React's reconciler is the right place to
        // decide whether children need to re-render.
        setOps((prev) =>
          prev.length === 0 && next.length === 0 ? prev : next,
        );
      })
      .catch((err) => {
        console.warn("useRunningOps refresh failed", err);
      });
  }, []);

  useEffect(() => {
    // Defer the first tick past first paint. The running-op strip
    // only lights up when an op is in flight — we can wait one idle
    // slot for it on cold start.
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
    const id = window.setInterval(refresh, POLL_INTERVAL_MS);
    return () => {
      cIC(idleHandle);
      window.clearInterval(id);
    };
  }, [refresh]);

  return { ops, refresh };
}
