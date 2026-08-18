import { useEffect, useState } from "react";
import { api } from "../api";
import type { AgentRunStatus } from "../types";

/**
 * Which agent runs are in flight, observed from the backend.
 *
 * Replaces the renderer-local `busy` flag the pane used to treat as
 * "running". That flag was set on click and cleared by a five-minute
 * timer, so it lied three ways: a run past five minutes read as idle, a
 * reload lost it entirely, and a cron-fired run — the entire reason the
 * `agent` noun exists — never set it at all. See
 * `dev-docs/agents-run-visibility-plan.md` §1.2.
 *
 * `error` is a third state on purpose. A poll that fails must not render
 * as "nothing is running": that is the same defect as reporting a failed
 * scan as "nothing scheduled for deletion". Callers render "can't
 * determine", never idle.
 */
export function useAgentRuns(pollMs = 5000) {
  const [runs, setRuns] = useState<AgentRunStatus[]>([]);
  const [error, setError] = useState(false);
  // Distinguishes "first poll has not returned" from "polled, nothing
  // running" — otherwise the pane briefly renders idle over a live run
  // on every mount.
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    // Per-effect cancellation, NOT a shared ref. A ref survives effect
    // re-runs, so a `pollMs` change would let the new effect set it back
    // to true and revive an in-flight request from the old one — which
    // would then write state and schedule its own next tick, running two
    // poll chains at once.
    let cancelled = false;
    let handle: number | undefined;
    // Monotonic id: a slow response resolving after a newer one must not
    // overwrite fresher state.
    let seq = 0;

    const tick = async () => {
      const mine = ++seq;
      try {
        // Bounded. Without this an invoke that never settles means the
        // `finally` never runs, no next poll is scheduled, `loaded`
        // stays false forever — and since unknown now LOCKS Run Now,
        // one hung IPC call would make every agent permanently
        // unrunnable.
        const next = await Promise.race([
          api.agentsRunningList(),
          new Promise<never>((_, reject) =>
            window.setTimeout(
              () => reject(new Error("agents_running_list timed out")),
              Math.max(2000, pollMs * 2),
            ),
          ),
        ]);
        if (cancelled || mine !== seq) return;
        setRuns(next);
        setError(false);
      } catch {
        if (cancelled || mine !== seq) return;
        // Keep the last known list rather than blanking it, but flag the
        // failure: consumers render "can't determine", never idle.
        setError(true);
      } finally {
        if (!cancelled && mine === seq) {
          setLoaded(true);
          // Chain the NEXT poll only after this one settled, so two can
          // never be in flight at once.
          handle = window.setTimeout(() => void tick(), pollMs);
        }
      }
    };

    void tick();
    return () => {
      cancelled = true;
      if (handle !== undefined) window.clearTimeout(handle);
    };
  }, [pollMs]);

  return { runs, error, loaded };
}

/** One agent's run status, if the poll has seen it.
 *
 *  `undefined` means "the poll has not reported this agent" — distinct
 *  from a status whose `in_flight` is null, which means "polled, nothing
 *  running". The card needs both distinctions to avoid rendering an
 *  unknown as idle. */
export function statusFor(
  runs: AgentRunStatus[],
  agentId: string,
): AgentRunStatus | undefined {
  return runs.find((r) => r.agent_id === agentId);
}
