import { useCallback, useEffect, useState } from "react";
import { api } from "../api";

const POLL_INTERVAL_MS = 30_000;

/**
 * Tracks the count of actionable pending rename journals for the
 * global banner. Polls every 30s, plus on window focus (so a user
 * returning from a terminal CLI move sees up-to-date state).
 *
 * `count === null` means "not loaded yet" (banner renders nothing).
 * `count === 0` means "loaded, zero pending" — banner hides cleanly.
 *
 * Callers can force an immediate refresh by calling `refresh()` —
 * used when a repair/rename operation terminates so the banner
 * reconciles without waiting for the next poll tick.
 */
export function usePendingJournals(): {
  count: number | null;
  refresh: () => void;
} {
  const [count, setCount] = useState<number | null>(null);

  const refresh = useCallback(() => {
    api
      .repairPendingCount()
      .then((n) => setCount(n))
      .catch((err) => {
        // Leave the last-known-good count in place; a transient failure
        // shouldn't blank the banner. Logged so a persistent outage is
        // visible in devtools.
        console.warn("usePendingJournals refresh failed", err);
      });
  }, []);

  useEffect(() => {
    refresh();
    const id = window.setInterval(refresh, POLL_INTERVAL_MS);
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(id);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  return { count, refresh };
}
