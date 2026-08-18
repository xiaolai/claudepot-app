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

    const tick = async () => {
      try {
        const next = await api.agentsRunningList();
        if (!alive.current) return;
        setRuns(next);
        setError(false);
      } catch {
        if (!alive.current) return;
        // Keep the last known list rather than blanking it: a transient
        // IPC failure should not make a live run vanish from the UI.
        setError(true);
      } finally {
        if (alive.current) setLoaded(true);
      }
    };

    void tick();
    handle = window.setInterval(() => void tick(), pollMs);
    return () => {
      alive.current = false;
      if (handle !== undefined) window.clearInterval(handle);
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
