// Loading, paging and following one transcript.
//
// Extracted from `Thread` because the component had grown to hold six
// concerns at once — initial load, polling, read-marking, scroll
// tracking, sending, pagination — and the two defects that lived here
// were both invisible inside that: a cursor handed back one short, and a
// poll with nothing to cancel it.
//
// The cancellation is the part worth reading. Every request carries an
// `AbortController` *and* a generation counter, because aborting is not
// enough on its own: `fetch` rejects asynchronously, so a response that
// already resolved can still be sitting in a microtask when the session
// changes. The generation check is what stops one session's events being
// appended to another's thread.
import { useCallback, useEffect, useRef, useState } from 'react';

import { ApiError, OfflineError, api } from './api.js';

/** Rows fetched per page. */
export const PAGE = 60;

/** How often to look for new events while the thread is on screen. */
const POLL_MS = 4000;

export function useTranscript(sessionId) {
  const [events, setEvents] = useState([]);
  const [cursor, setCursor] = useState(0);
  const [total, setTotal] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);

  // Bumped on every session change. A response tagged with an older
  // generation is dropped rather than applied.
  const generation = useRef(0);
  const inflight = useRef(null);

  useEffect(() => {
    generation.current += 1;
    const mine = generation.current;
    const ctrl = new AbortController();
    inflight.current?.abort();
    inflight.current = ctrl;

    setEvents([]);
    setCursor(0);
    setTotal(null);
    setLoading(true);
    setError(null);

    api
      .transcriptTail(sessionId, { limit: PAGE, signal: ctrl.signal })
      .then((page) => {
        if (generation.current !== mine) return;
        setEvents(page.events);
        setCursor(page.next_cursor);
        setTotal(page.total);
      })
      .catch((e) => {
        if (generation.current !== mine || e?.name === 'AbortError') return;
        setError(e instanceof OfflineError ? 'offline' : e instanceof ApiError ? e.code : 'error');
      })
      .finally(() => {
        if (generation.current === mine) setLoading(false);
      });

    return () => {
      ctrl.abort();
      if (inflight.current === ctrl) inflight.current = null;
      // Bumped on unmount too, not only on a session change: every
      // guard in this hook is `generation.current !== mine`, and a
      // generation that never moves means an unmounted component's
      // in-flight response still passes the check.
      generation.current += 1;
    };
  }, [sessionId]);

  // Follow along. `cursor` is the server's own count, handed back
  // untouched — decrementing it drops an event per poll, which is what
  // the first version did.
  useEffect(() => {
    if (error || total === null) return undefined;
    const mine = generation.current;

    let polling = null;

    const tick = async () => {
      if (document.visibilityState !== 'visible') return;
      const ctrl = new AbortController();
      polling = ctrl;
      try {
        const page = await api.transcriptSince(sessionId, cursor, {
          limit: PAGE,
          signal: ctrl.signal,
        });
        // The session may have changed, or the component gone, while
        // this was in flight.
        if (generation.current !== mine) return;

        // The cursor advances **before** the empty check, and that
        // ordering is the whole point. A page can carry raw events that
        // render as nothing — a `stop_hook_summary` is dropped, and a
        // real transcript has two per assistant turn. Returning early on
        // an empty render would leave the cursor where it was and poll
        // the same range forever, never reaching the next real message.
        setCursor(page.next_cursor);
        setTotal(page.total);

        if (!page.events.length) return;
        setEvents((prev) => {
          // The cursor makes duplicates impossible in principle; the
          // filter makes them impossible in fact, because two ticks can
          // overlap if one is slow.
          const known = new Set(prev.map((e) => e.index));
          const fresh = page.events.filter((e) => !known.has(e.index));
          return fresh.length ? [...prev, ...fresh] : prev;
        });
      } catch {
        // The session list owns the connection banner; a failed poll
        // here just means the next one tries again.
      } finally {
        if (polling === ctrl) polling = null;
      }
    };

    const h = window.setInterval(tick, POLL_MS);
    return () => {
      window.clearInterval(h);
      // An in-flight poll outlives the interval otherwise, and would
      // resolve into a component that is gone.
      polling?.abort();
      polling = null;
    };
  }, [sessionId, cursor, total, error]);

  /** Pull the page ending just below the oldest event held. */
  const loadEarlier = useCallback(async () => {
    const mine = generation.current;
    const first = events[0]?.index;
    if (first === undefined || first === 0) return;
    try {
      const page = await api.transcriptBefore(sessionId, first, { limit: PAGE });
      if (generation.current !== mine || !page.events.length) return;
      setEvents((prev) => {
        const known = new Set(prev.map((e) => e.index));
        return [...page.events.filter((e) => !known.has(e.index)), ...prev];
      });
      setTotal((t) => t ?? page.total);
    } catch {
      // Leaving the button in place is the retry.
    }
  }, [sessionId, events]);

  const hasEarlier = events.length > 0 && events[0].index > 0;

  return { events, total, loading, error, hasEarlier, loadEarlier };
}
