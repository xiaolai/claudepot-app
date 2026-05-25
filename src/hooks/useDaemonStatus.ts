import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { DaemonStatus } from "../api/cc-daemon";

const POLL_INTERVAL_MS = 60_000;

/**
 * Polls `claude daemon status` for the supervisor + background-worker
 * count. 60s cadence matches the Sidebar live-Activity strip's rhythm
 * — bg sessions change on a human-decision timescale (a user types
 * `/bg` or removes a session in `claude agents`), so faster polling
 * burns IPC for no visible gain.
 *
 * Returns `null` until the first poll completes; consumers should
 * treat `null` as "not loaded yet," distinct from a successful poll
 * that returned `bgWorkers: 0` (a healthy idle daemon).
 *
 * Consumers (Activities Live StatCard, Sidebar bg badge) should
 * render-if-nonzero — the strip's existence is data, not chrome.
 */
export function useDaemonStatus(): {
  status: DaemonStatus | null;
  refresh: () => void;
} {
  const [status, setStatus] = useState<DaemonStatus | null>(null);

  const refresh = useCallback(() => {
    api
      .ccDaemonStatus()
      .then((next) => {
        setStatus((prev) => {
          // A transient parse failure must not clear a valid
          // last-known-good snapshot — that would flicker the
          // Sidebar badge / Live tile off and on. Keep the good
          // value until a fresh successful scrape arrives.
          if (
            next.parseStatus.kind !== "ok" &&
            prev !== null &&
            prev.parseStatus.kind === "ok"
          ) {
            return prev;
          }
          // Identity-skip when nothing meaningful changed — keep
          // referential equality so memoized consumers don't churn
          // on every poll. Three fields are load-bearing for the UI:
          // running, bg workers, parse-status kind. The other
          // DaemonStatus fields (pid, uptime, paths) aren't observed
          // by any current renderer; comparing them would churn
          // re-renders for invisible state.
          if (
            prev !== null &&
            prev.running === next.running &&
            prev.bgWorkers === next.bgWorkers &&
            prev.parseStatus.kind === next.parseStatus.kind
          ) {
            return prev;
          }
          return next;
        });
      })
      .catch((err) => {
        // Tauri IPC down or backend not yet ready — leave the prior
        // value alone, log for diagnostics. A persistent failure
        // surfaces as `status === null` forever, which the consumer
        // already treats as "no badge."
        console.warn("useDaemonStatus refresh failed", err);
      });
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [refresh]);

  return { status, refresh };
}
