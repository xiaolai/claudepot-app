import { useCallback, useEffect, useState } from "react";

import { DISMISSED_ISSUES_KEY as KEY } from "../lib/storageKeys";
const SNOOZE_MS = 24 * 60 * 60 * 1000;
/** Interval for the periodic expiry sweep. Audit issue #8 fix:
 *  pre-fix the store was purged only on mount; a long-lived
 *  session could accumulate stale keys for hours. Hourly is
 *  conservative — the snooze window is 24 h, so even one sweep
 *  per cycle keeps the store bounded. The sweep is O(n) over
 *  dismissed-issue count (typically single digits). */
const PURGE_INTERVAL_MS = 60 * 60 * 1000;

interface Payload {
  [id: string]: number; // unix ms when dismissed
}

function readStore(): Payload {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return typeof parsed === "object" && parsed !== null
      ? (parsed as Payload)
      : {};
  } catch {
    return {};
  }
}

function writeStore(p: Payload): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // best-effort
  }
}

/**
 * Transient per-issue snooze store. Dismissed issues reappear after
 * 24 hours so persistent conditions (drift, sync error) still
 * eventually re-surface if unresolved.
 *
 * Errors should not be dismissable (see useStatusIssues.dismissable).
 * This hook doesn't enforce that — it's the component's job to only
 * render the dismiss button when issue.dismissable is true.
 */
export function useDismissedIssues(): {
  isDismissed: (id: string) => boolean;
  dismiss: (id: string) => void;
  clear: (id: string) => void;
  /** Snapshot of the live (non-expired) dismissed-issue keys. Used by
   *  the App.tsx snooze auto-clear effect to reconcile stale entries
   *  carried over from a previous renderer lifetime against the
   *  currently-live `rawIssues` set. */
  knownKeys: () => string[];
} {
  const [store, setStore] = useState<Payload>(() => readStore());

  // Purge expired entries on mount AND periodically thereafter so
  // long-lived sessions don't accumulate stale keys (audit issue
  // #8 fix). The mount sweep handles state carried from prior
  // renderer lifetimes; the interval sweep handles ongoing decay.
  useEffect(() => {
    const sweep = () => {
      const now = Date.now();
      setStore((prev) => {
        let changed = false;
        const pruned: Payload = {};
        for (const [id, ts] of Object.entries(prev)) {
          if (now - ts < SNOOZE_MS) pruned[id] = ts;
          else changed = true;
        }
        if (!changed) return prev;
        writeStore(pruned);
        return pruned;
      });
    };
    sweep();
    const handle = setInterval(sweep, PURGE_INTERVAL_MS);
    return () => clearInterval(handle);
  }, []);

  const isDismissed = useCallback(
    (id: string) => {
      const ts = store[id];
      if (!ts) return false;
      return Date.now() - ts < SNOOZE_MS;
    },
    [store],
  );

  const dismiss = useCallback((id: string) => {
    setStore((prev) => {
      const next = { ...prev, [id]: Date.now() };
      writeStore(next);
      return next;
    });
  }, []);

  const clear = useCallback((id: string) => {
    setStore((prev) => {
      const next = { ...prev };
      delete next[id];
      writeStore(next);
      return next;
    });
  }, []);

  const knownKeys = useCallback(() => Object.keys(store), [store]);

  return { isDismissed, dismiss, clear, knownKeys };
}
