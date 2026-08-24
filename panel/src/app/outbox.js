// Messages typed while the Mac was unreachable.
//
// The design's composer holds a message when offline and shows the
// count — "N held — sends when the Mac is back". The panel used to
// disable the composer instead, which is honest but throws away the one
// thing a phone is actually good for: you thought of it on the train.
//
// Three properties, and each is why this is a module with its own
// storage rather than component state:
//
//   - **It survives leaving the thread.** A queue held in `Thread` dies
//     when you tap Back, which makes "sends when the Mac is back" false
//     the moment the user navigates. localStorage survives the reload
//     too, and that matters more here than it would elsewhere: an iOS
//     home-screen app is evicted from memory whenever the OS likes.
//   - **It never fires into the wrong conversation.** Draining is gated
//     on the session still being live and addressable, not merely on the
//     host answering. A session that ended while you were offline keeps
//     its queue and says so, rather than posting into whatever a
//     recycled pid is now running.
//   - **Every entry is cancellable.** A queued message is going to be
//     sent later, unattended, by a phone in a pocket. Without a way to
//     take it back the only exit is to be somewhere else when it fires.
//
// Each entry carries the idempotency key it was minted with, so a drain
// interrupted midway — the tab closes, the host 500s — replays as the
// same intent rather than sending twice.

const KEY = 'claudepot.outbox';

/** Per session. A queue longer than this is a mistake, not a plan. */
const PER_SESSION = 20;

/** Sessions tracked at once. Oldest queue dropped first. */
const MAX_SESSIONS = 40;

/** In-memory mirror, so a render never parses JSON. */
let cache = null;
const listeners = new Set();

function read() {
  if (cache) return cache;
  try {
    const raw = window.localStorage.getItem(KEY);
    const parsed = raw ? JSON.parse(raw) : null;
    cache = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
  } catch {
    // Recovers silently, like the panel's other client-side preferences
    // and unlike the host's revocation list: the cost of losing this is
    // retyping a sentence, so failing loud would be the worse trade.
    cache = {};
  }
  return cache;
}

function write(next) {
  cache = next;
  try {
    window.localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* Private mode, or the quota. The queue lives for this page load. */
  }
  listeners.forEach((fn) => fn());
}

/** Subscribe to every change. Returns the unsubscribe. */
export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

// One frozen array for "nothing queued", so `queuedFor` returns the same
// reference every time and a subscriber comparing snapshots does not see
// a change on every read.
const EMPTY = Object.freeze([]);

/** This session's queue, oldest first. Never null. */
export function queuedFor(sessionId) {
  return read()[sessionId] ?? EMPTY;
}

/** How many messages are held, across every session. */
export function totalQueued() {
  const all = read();
  let n = 0;
  for (const id of Object.keys(all)) n += all[id].length;
  return n;
}

/**
 * Hold a message for a session.
 *
 * The idempotency key is minted HERE rather than at drain time. A drain
 * that sends, loses the response, and is retried after a reload must
 * present the same key or the host treats it as a second intent — which
 * is the one failure this queue could cause that typing the message
 * again could not.
 */
export function enqueue(sessionId, text, key) {
  const all = { ...read() };
  const mine = [...(all[sessionId] ?? []), { id: key, text, at: Date.now() }];
  all[sessionId] = mine.slice(-PER_SESSION);

  const ids = Object.keys(all);
  if (ids.length > MAX_SESSIONS) {
    // Oldest queue first, by its newest entry — a session you queued
    // into a minute ago outranks one you queued into last week.
    const stamp = (id) => all[id][all[id].length - 1]?.at ?? 0;
    for (const id of ids.sort((a, b) => stamp(a) - stamp(b)).slice(0, ids.length - MAX_SESSIONS)) {
      delete all[id];
    }
  }
  write(all);
}

/** Drop one entry — the user took it back, or it was sent. */
export function cancel(sessionId, id) {
  const all = read();
  const mine = all[sessionId];
  if (!mine) return;
  const next = { ...all };
  const left = mine.filter((m) => m.id !== id);
  if (left.length) next[sessionId] = left;
  else delete next[sessionId];
  write(next);
}

/**
 * Record that the host refused this message, and stop retrying it.
 *
 * Without this a permanently-refused entry is re-sent on every poll —
 * a failing request every four seconds, forever, from a phone in a
 * pocket. `OfflineError` is deliberately NOT routed here: "the Mac is
 * not answering" is the state this queue exists for, and retrying it is
 * the feature rather than a loop.
 */
export function markFailed(sessionId, id, reason) {
  const all = read();
  const mine = all[sessionId];
  if (!mine) return;
  write({ ...all, [sessionId]: mine.map((m) => (m.id === id ? { ...m, failed: reason } : m)) });
}

/** Arm a failed entry for one more attempt. */
export function retry(sessionId, id) {
  const all = read();
  const mine = all[sessionId];
  if (!mine) return;
  write({
    ...all,
    [sessionId]: mine.map((m) => {
      if (m.id !== id) return m;
      const { failed, ...rest } = m;
      return rest;
    }),
  });
}

/** Drop a whole session's queue. */
export function clearFor(sessionId) {
  const all = read();
  if (!all[sessionId]) return;
  const next = { ...all };
  delete next[sessionId];
  write(next);
}

/** Test seam. Not called by the app. */
export function _reset() {
  cache = null;
  try {
    window.localStorage.removeItem(KEY);
  } catch {
    /* nothing to clear */
  }
  listeners.forEach((fn) => fn());
}
