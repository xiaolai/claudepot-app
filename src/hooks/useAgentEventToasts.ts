import { useCallback, useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { useEmit } from "../providers/AppStateProvider";

/**
 * Listen for `agent_event_orchestrator` events. Two subscribed
 * channels:
 *
 * - `agent-event-failed` — an event-triggered agent's dispatch
 *   returned an error. Routes to the `agentEventFailed` category
 *   (P2, error-level toast). Separate category from `agentRan`
 *   so a user can mute success acks while still hearing failures.
 * - `agent-event-burst-capped` — the orchestrator's first-tick
 *   catch-up dropped excess fires under the bounded-burst cap
 *   (PRD D6). One emission per affected tick — the orchestrator
 *   coalesces — so this is low-volume by design.
 *
 * `agent-event-dispatched` is deliberately NOT subscribed: a
 * successful event-agent run lands in `RunHistoryPanel` with its
 * structured output, and a toast per fire would spam every
 * settled-session narration. The run-history surface is the
 * success path.
 *
 * Wired once at App.tsx next to `useRotationEvents`.
 */

interface FailedPayload {
  agentId: string;
  sessionId: string;
  error: string;
}

interface BurstCappedPayload {
  cap: number;
  dropped: number;
}

/** Truncate a UUID-ish id to its first 8 chars for compact toast
 *  bodies. The agents.json record + the run-history panel carry
 *  the full id; the toast just needs a recognizable prefix. */
function short(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}

export function useAgentEventToasts(): void {
  const emit = useEmit();

  const handleFailed = useCallback(
    (p: FailedPayload) => {
      void emit({
        category: "agentEventFailed",
        kind: "error",
        title: "Agent fire failed",
        body: `${short(p.agentId)} on session ${short(p.sessionId)} — ${p.error}`,
        // Dedupe per (agent, session) so a re-fire that keeps
        // failing doesn't pile identical toasts. The orchestrator
        // already records each failure in the bell. No `target`:
        // the `NotificationTarget.section` enum doesn't carry the
        // Agents-section id (kept as "automations" for localStorage
        // compatibility), so click-routing would have to widen
        // the union — out of scope here.
        dedupeKey: `agent-event-failed:${p.agentId}:${p.sessionId}`,
      });
    },
    [emit],
  );

  const handleBurstCapped = useCallback(
    (p: BurstCappedPayload) => {
      const noun = p.dropped === 1 ? "session" : "sessions";
      void emit({
        category: "agentEventBurstCapped",
        title: "Agent first-tick cap applied",
        // grill X16: the cap is per-agent — every event agent
        // gets its bounded catch-up the first time it
        // participates in a tick this process (boot-time AND
        // late-added agents alike). The dropped sessions stay
        // OUT of the ledger so they will re-evaluate on later
        // ticks.
        body: `${p.dropped} settled ${noun} were held back on a fresh agent's first contact (cap ${p.cap}). They will fire on later ticks.`,
        // The orchestrator emits at most once per first-contact
        // burst per tick. The dedupe key collapses a near-
        // instant repeat (e.g. a relaunch followed by a tick).
        dedupeKey: `agent-event-burst-capped:${p.cap}:${p.dropped}`,
      });
    },
    [emit],
  );

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    const wire = <T>(channel: string, handler: (p: T) => void) => {
      void listen<T>(channel, (ev) => {
        if (!active || !ev.payload) return;
        handler(ev.payload);
      })
        .then((fn) => {
          if (!active) fn();
          else unlisteners.push(fn);
        })
        .catch((err: unknown) => {
          // grill X18: previously this `.catch` swallowed every
          // failure mode. A non-Tauri renderer (vitest under jsdom,
          // a future preview build) raises a ReferenceError-shaped
          // failure on the `__TAURI_IPC__` global; that is the
          // expected silent-skip case. A *real* `listen()` failure
          // (renderer reload race mid-mount, a renamed channel on
          // a future addition, an unexpected throw inside the
          // plugin's invoke layer) looks identical here and used
          // to be invisible — debugging a missing toast meant
          // staring at the bell-icon log with no upstream signal.
          //
          // Discriminate: a non-Tauri renderer (vitest under jsdom,
          // a future preview build) raises a failure whose message
          // mentions `__TAURI_*` or a missing tauri binding. That
          // is the expected silent-skip case. Everything else gets
          // a `console.warn` so a real channel-typo or runtime
          // failure is debuggable. We deliberately avoid touching
          // `process` here — the renderer is browser-context and
          // doesn't carry `@types/node`; the message-shape filter
          // is sufficient for the test surface (vitest's mock
          // never throws this catch path).
          const msg =
            err instanceof Error ? err.message : String(err);
          const isNonTauri =
            /__TAURI/i.test(msg) ||
            (/tauri/i.test(msg) && /undefined|not (a )?function/i.test(msg));
          if (!isNonTauri) {
            // eslint-disable-next-line no-console
            console.warn(
              `useAgentEventToasts: listen(${channel}) failed:`,
              err,
            );
          }
        });
    };

    wire<FailedPayload>("agent-event-failed", handleFailed);
    wire<BurstCappedPayload>("agent-event-burst-capped", handleBurstCapped);

    return () => {
      active = false;
      unlisteners.forEach((fn) => fn());
    };
  }, [handleFailed, handleBurstCapped]);
}
