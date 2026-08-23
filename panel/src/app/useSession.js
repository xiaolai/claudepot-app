// Signed-in state and the session list poll.
//
// Both were inline in `Panel`, which meant the auth lifecycle and the
// polling loop shared one component with routing and view selection.
// Four concerns in one function is where a race hides: the poll has to
// stop on sign-out, the sign-out has to abort the poll, and the mount
// probe has to not fight either of them.
import { useCallback, useEffect, useRef, useState } from 'react';

import { ApiError, OfflineError, api, readToken, writeToken } from './api.js';

/** Poll interval for the sessions list, in ms. */
const POLL_MS = 4000;

/**
 * Whether this browser holds a token the host still accepts.
 *
 * `checked` is the third state and the one that matters: without it a
 * revoked device renders an empty session list, which looks exactly like
 * a machine with nothing running.
 */
export function useAuth() {
  const [token, setToken] = useState(() => readToken());
  const [checked, setChecked] = useState(false);

  const signOut = useCallback(() => {
    writeToken(null);
    setToken(null);
  }, []);

  const signIn = useCallback((t) => {
    writeToken(t);
    setToken(t);
    setChecked(true);
  }, []);

  useEffect(() => {
    let cancelled = false;
    if (!token) {
      setChecked(true);
      return undefined;
    }
    api
      .me()
      .then(() => !cancelled && setChecked(true))
      .catch((e) => {
        if (cancelled) return;
        if (e instanceof ApiError && e.status === 401) signOut();
        setChecked(true);
      });
    return () => {
      cancelled = true;
    };
  }, [token, signOut]);

  return { token, checked, signIn, signOut };
}

/**
 * The session list, polled while the page is visible.
 *
 * `conn` is derived here rather than passed around: whether the host is
 * reachable is a property of this poll and nothing else on the surface
 * makes a request often enough to know.
 */
export function useSessions({ enabled, onUnauthorized }) {
  const [sessions, setSessions] = useState(null);
  const [approvals, setApprovals] = useState([]);
  const [conn, setConn] = useState('online');
  const inflight = useRef(null);

  const refresh = useCallback(async () => {
    if (!readToken()) return;
    inflight.current?.abort();
    const ctrl = new AbortController();
    inflight.current = ctrl;
    try {
      // Both on one tick, and settled independently: a server too old
      // to know about approvals, or one whose queue directory is
      // unreadable, must not blank the session list beside it.
      const [list, waiting] = await Promise.allSettled([
        api.sessions(ctrl.signal),
        api.approvals(ctrl.signal),
      ]);
      if (inflight.current !== ctrl) return;
      if (list.status === 'rejected') throw list.reason;
      setSessions(list.value?.sessions ?? []);
      setApprovals(waiting.status === 'fulfilled' ? (waiting.value?.approvals ?? []) : []);
      setConn('online');
    } catch (e) {
      if (e?.name === 'AbortError' || inflight.current !== ctrl) return;
      if (e instanceof OfflineError) setConn('offline');
      else if (e instanceof ApiError && e.status === 401) onUnauthorized?.();
      else setConn('online');
    }
  }, [onUnauthorized]);

  useEffect(() => {
    if (!enabled) {
      setSessions(null);
      setApprovals([]);
      return undefined;
    }
    refresh();
    // Skip the tick while the page is hidden. Each list request makes
    // the host re-stat every transcript and re-parse the ones a live
    // session just appended to — measured at ~0.34s against a real
    // 1.1 GB index — and a phone in a pocket has no use for the result.
    // The visibility listener refreshes on the way back, so nothing is
    // stale by the time it is looked at.
    const id = window.setInterval(() => {
      if (document.visibilityState === 'visible') refresh();
    }, POLL_MS);
    const onVisible = () => document.visibilityState === 'visible' && refresh();
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      window.clearInterval(id);
      document.removeEventListener('visibilitychange', onVisible);
      inflight.current?.abort();
      inflight.current = null;
    };
  }, [enabled, refresh]);

  return { sessions, approvals, conn, refresh };
}

/**
 * Theme preference. `system` follows the device, which is what a phone
 * user expects; the two explicit values are an override, not a default.
 */
export function useTheme() {
  const [pref, setPref] = useState(() => {
    try {
      return window.localStorage.getItem('claudepot.theme') || 'system';
    } catch {
      return 'system';
    }
  });
  useEffect(() => {
    try {
      if (pref === 'system') window.localStorage.removeItem('claudepot.theme');
      else window.localStorage.setItem('claudepot.theme', pref);
    } catch {
      /* Non-fatal: the preference lives for this page load only. */
    }
  }, [pref]);
  return [pref, setPref];
}

/**
 * How a transcript renders tool calls.
 *
 * `grouped` (the default) folds each consecutive run of tool ticks into
 * one row; `shown` lists them the way the desktop does.
 *
 * Grouped is the default because of what a transcript is actually made
 * of. Measured across five real sessions on this machine, tool ticks
 * were **59–91%** of all rows — so listing them individually means a
 * phone screen of one-line ticks with the conversation scattered
 * through it. The desktop can afford that; a 390px column cannot.
 *
 * Not a *hide*: the group row keeps the count and expands, so "Claude
 * is doing something and here is roughly what" survives. Removing tool
 * calls from the transcript entirely would make a working session and a
 * stalled one look identical.
 *
 * localStorage rather than the server: this is how one device likes to
 * read, not a property of the machine. A phone and a laptop looking at
 * the same Claudepot should be allowed to disagree.
 */
export function useToolDisplay() {
  const [pref, setPref] = useState(() => {
    try {
      return window.localStorage.getItem('claudepot.tools') === 'shown' ? 'shown' : 'grouped';
    } catch {
      return 'grouped';
    }
  });
  useEffect(() => {
    try {
      if (pref === 'grouped') window.localStorage.removeItem('claudepot.tools');
      else window.localStorage.setItem('claudepot.tools', pref);
    } catch {
      /* Non-fatal: the preference lives for this page load only. */
    }
  }, [pref]);
  return [pref, setPref];
}
