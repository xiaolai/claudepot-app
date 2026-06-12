import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { requestIdle, cancelIdle } from "../lib/idle";
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
      .then((next) => {
        // Identity-skip on structurally-equal results. The IPC returns
        // a freshly-deserialized object on every call, so a naive
        // `setSummary(next)` commits a state change React believes is
        // meaningful, re-rendering every consumer (the banner + every
        // AppShell child). The shape is three numbers — cheap to
        // compare in full.
        setSummary((prev) =>
          prev !== null &&
          prev.pending === next.pending &&
          prev.stale === next.stale &&
          prev.running === next.running
            ? prev
            : next,
        );
      })
      .catch((err) => {
        console.warn("usePendingJournals refresh failed", err);
      });
  }, []);

  useEffect(() => {
    // Yield to the shell's first paint before hitting Tauri — the
    // journal banner is a background concern, not a critical-path
    // widget. requestIdle falls back to a timeout on Safari/WebKit
    // (Tauri on macOS) which still doesn't expose rIC.
    const idleHandle = requestIdle(() => refresh(), { fallbackDelayMs: 250 });
    const id = window.setInterval(refresh, POLL_INTERVAL_MS);
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelIdle(idleHandle);
      window.clearInterval(id);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  const count = summary === null ? null : summary.pending + summary.stale;
  return { summary, count, refresh };
}
