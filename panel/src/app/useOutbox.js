// Reading the outbox, and draining it.
//
// The drain lives at the SHELL, not in the thread, and that placement is
// the promise: "sends when the Mac is back" has to hold whether or not
// the user is still looking at the conversation they typed into. A
// drain inside `Thread` would only run while that thread was open,
// which is the case where the user least needs it.
import { useCallback, useEffect, useRef, useSyncExternalStore } from 'react';

import { OfflineError, api, explainSend } from './api.js';
import * as outbox from './outbox.js';

/** This session's held messages, oldest first. */
export function useQueued(sessionId) {
  const snapshot = useCallback(() => outbox.queuedFor(sessionId), [sessionId]);
  return useSyncExternalStore(outbox.subscribe, snapshot);
}

/** How many messages are held across every session. */
export function useTotalQueued() {
  return useSyncExternalStore(outbox.subscribe, outbox.totalQueued);
}

/**
 * Send what is held, as soon as there is somewhere to send it.
 *
 * Gated on the SESSION being live and addressable, not on the host
 * merely answering. A queue whose session ended keeps its messages and
 * waits — posting them into a pid that has since been recycled is the
 * one outcome worse than not sending at all.
 *
 * Sequential, oldest first, because these are instructions to the same
 * conversation and the order they were typed in is the order they mean
 * something in.
 */
export function useOutboxDrain(sessions, conn, onSent) {
  const total = useTotalQueued();
  // A ref, not state: two drains running at once would send the same
  // entry twice, and `sending` state is still stale on the second of
  // two effects that fire in one commit.
  const busy = useRef(false);

  useEffect(() => {
    if (conn === 'offline' || !sessions || total === 0 || busy.current) return undefined;
    busy.current = true;
    let stopped = false;
    let sent = false;

    (async () => {
      try {
        for (const s of sessions) {
          if (stopped) return;
          if (!s.live || !s.addressable) continue;
          for (const m of outbox.queuedFor(s.session_id)) {
            if (stopped) return;
            // Already refused once. It needs a person, not another try.
            if (m.failed) continue;
            try {
              // `m.id` IS the idempotency key, minted when the message
              // was held. A drain that sends and then loses the answer
              // replays as the same intent rather than as a second one.
              await api.sendPrompt(s.session_id, m.text, m.id);
              outbox.cancel(s.session_id, m.id);
              sent = true;
            } catch (e) {
              // The Mac went away again mid-drain. Leave everything
              // where it is; this is the state the queue is for.
              if (e instanceof OfflineError) return;
              outbox.markFailed(s.session_id, m.id, explainSend(e));
            }
          }
        }
      } finally {
        busy.current = false;
        if (sent && !stopped) onSent?.();
      }
    })();

    return () => {
      stopped = true;
    };
  }, [sessions, conn, total, onSent]);
}
