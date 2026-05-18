import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { RunningOpInfo } from "../types";

const POLL_INTERVAL_MS = 3_000;

/**
 * Tuple-equality over the fields that matter to the consumer surface
 * (status strip, progress modal). Done field-by-field rather than a
 * generic deep-equal so the cost is bounded and obvious; new fields
 * on RunningOpInfo only matter here if a UI surface needs them, in
 * which case they get added explicitly.
 */
function opsEqual(a: RunningOpInfo[], b: RunningOpInfo[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.op_id !== y.op_id ||
      x.kind !== y.kind ||
      x.old_path !== y.old_path ||
      x.new_path !== y.new_path
    ) {
      return false;
    }
  }
  return true;
}

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
        // Identity-skip on structurally-equal results. Without this,
        // each 3 s poll commits a fresh `[]` reference (the IPC always
        // deserializes a new array), every `useState` consumer re-
        // renders, and the cascade reaches into ProjectEnvPanel /
        // PermissionPanel where it triggers redundant `.env*` re-reads
        // and a visible loading flash. See dev-docs… well, this
        // comment is the dev-doc.
        setOps((prev) => (opsEqual(prev, next) ? prev : next));
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
