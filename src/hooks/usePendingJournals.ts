import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { PendingJournalsSummary } from "../types";

const POLL_INTERVAL_MS = 30_000;

/**
 * Tracks per-status counts of pending rename journals for the global
 * banner. Polls every 30s + on window focus so a user returning from
 * a CLI rename sees up-to-date state (plan §7.5).
 *
 * `summary === null` means "not loaded yet" (banner renders nothing).
 * `summary.pending === 0 && summary.stale === 0` means "all clear" —
 * banner hides.
 *
 * `count` is a convenience derived field (pending + stale) preserved
 * for backward compat with banner display-text code.
 *
 * Callers force an immediate refresh via `refresh()` after a
 * repair/rename op terminates.
 */
export function usePendingJournals(): {
  summary: PendingJournalsSummary | null;
  count: number | null;
  refresh: () => void;
} {
  const [summary, setSummary] = useState<PendingJournalsSummary | null>(null);

  const refresh = useCallback(() => {
    api
      .repairStatusSummary()
      .then((s) => setSummary(s))
      .catch((err) => {
        console.warn("usePendingJournals refresh failed", err);
      });
  }, []);

  useEffect(() => {
    // Yield to the shell's first paint before hitting Tauri — the
    // journal banner is a background concern, not a critical-path
    // widget. requestIdleCallback with a timeout fallback keeps us
    // responsive on Safari/WebKit (Tauri on macOS) which still doesn't
    // expose rIC.
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
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      cIC(idleHandle);
      window.clearInterval(id);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  const count = summary === null ? null : summary.pending + summary.stale;
  return { summary, count, refresh };
}
