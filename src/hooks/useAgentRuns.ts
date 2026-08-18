import { useEffect, useRef, useState } from "react";
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
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    let handle: number | undefined;
    // Monotonic request id. `setInterval` could start a second poll
    // while the first was still in flight, and a slow earlier response
    // resolving last would overwrite fresher state — including a stale
    // empty list rendering idle over a live run. Chained timeouts plus
    // this guard make that unrepresentable.
    let seq = 0;

    const tick = async () => {
      const mine = ++seq;
      try {
        const next = await api.agentsRunningList();
        if (!alive.current || mine !== seq) return;
        setRuns(next);
        setError(false);
      } catch {
        if (!alive.current || mine !== seq) return;
        // Keep the last known list rather than blanking it, but flag the
        // failure: consumers render "can't determine", never idle, and
        // must not present the stale list as current fact.
        setError(true);
      } finally {
        if (alive.current && mine === seq) {
          setLoaded(true);
          // Schedule the NEXT poll only after this one settled, so two
          // can never be in flight at once.
          handle = window.setTimeout(() => void tick(), pollMs);
        }
      }
    };

    void tick();
    return () => {
      alive.current = false;
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
