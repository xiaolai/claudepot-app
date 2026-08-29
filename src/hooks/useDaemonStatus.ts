import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { DaemonStatus } from "../api/cc-daemon";

const POLL_INTERVAL_MS = 60_000;

/**
 * Polls CC's daemon roster for the live background-worker count (no
 * supervisor flag — see `claudepot-core::cc_daemon`). 60s cadence matches the Sidebar live-Activity strip's rhythm
 * — bg sessions change on a human-decision timescale (a user types
 * `/bg` or removes a session in `claude agents`), so faster polling
 * burns IPC for no visible gain.
 *
 * This interval used to drive a `claude daemon status` subprocess on
 * every tick, which on an older Claude Code build was billed as a
 * headless model prompt (issue #94). The backend reads a file now, so
 * the cadence costs IPC and nothing else — but note the shape of that
 * bug before changing it: an unbounded `setInterval` with no circuit
 * breaker is only safe because what it calls cannot spend money.
 *
 * Returns `null` until the first poll completes; consumers should
 * treat `null` as "not loaded yet," distinct from a successful poll
 * that returned `bgWorkers: 0` (a healthy idle daemon).
 *
 * `SidebarBgBadge` is the only consumer; it renders-if-nonzero — the
 * strip's existence is data, not chrome.
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
          // A transient read failure must not clear a valid
          // last-known-good snapshot — that would flicker the
          // Sidebar badge off and on. Keep the good value until a
          // fresh successful read arrives.
          if (
            next.parseStatus.kind !== "ok" &&
            prev !== null &&
            prev.parseStatus.kind === "ok"
          ) {
            return prev;
          }
          // Identity-skip when nothing meaningful changed — keep
          // referential equality so memoized consumers don't churn
          // on every poll. Two fields are load-bearing for the UI:
          // the live worker count and the parse-status kind.
          // `rosterPath` is diagnostic and constant for a given
          // config dir; comparing it would churn re-renders for
          // invisible state.
          if (
            prev !== null &&
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
