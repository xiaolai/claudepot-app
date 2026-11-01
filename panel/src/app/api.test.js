import test from 'node:test';
import assert from 'node:assert/strict';
import { ApiError, OfflineError, READ_DEADLINE_MS, api, readToken, writeToken } from './api.js';

/**
 * The fetch layer, which every screen goes through and nothing tested.
 *
 * These are the behaviours the rest of the panel assumes without ever
 * checking: that the bearer is attached, that an idempotency key
 * reaches the header the server refuses mutations without, that a
 * non-2xx becomes an `ApiError` carrying the server's stable slug, and
 * — the one the composer's error copy depends on — that a network
 * failure is classified `OfflineError` while an abort is re-thrown
 * untouched.
 */

/** Minimal `window` with a real-enough localStorage and fetch. */
function harness({ status = 200, payload = {}, throws = null } = {}) {
  const calls = [];
  const store = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => store.set(k, String(v)),
      removeItem: (k) => store.delete(k),
    },
    crypto: { randomUUID: () => 'uuid-fixed' },
  };
  globalThis.fetch = async (path, init) => {
    calls.push({ path, init });
    if (throws) throw throws;
    const text = payload === null ? '' : JSON.stringify(payload);
    return {
      ok: status >= 200 && status < 300,
      status,
      text: async () => text,
    };
  };
  return calls;
}

test('the bearer token is attached when one is stored', async () => {
  const calls = harness({ payload: { sessions: [] } });
  writeToken('tok-123');
  assert.equal(readToken(), 'tok-123');
  await api.sessions();
  assert.equal(calls[0].init.headers.Authorization, 'Bearer tok-123');
  // Bearer, not cookie — sending credentials would add a CSRF surface.
  assert.equal(calls[0].init.credentials, 'omit');
  assert.equal(calls[0].init.cache, 'no-store');
});

test('no Authorization header when signed out', async () => {
  const calls = harness({ payload: {} });
  writeToken(null);
  await api.sessions();
  assert.equal(calls[0].init.headers.Authorization, undefined);
});

test('a mutation carries its idempotency key and a JSON body', async () => {
  const calls = harness({ payload: { ok: true } });
  writeToken('tok');
  await api.sendPrompt('sess-1', 'hello', 'key-abc');
  const { init, path } = calls[0];
  assert.match(path, /\/api\/sessions\/sess-1\/prompt$/);
  assert.equal(init.headers['Idempotency-Key'], 'key-abc');
  assert.equal(init.headers['Content-Type'], 'application/json');
  assert.deepEqual(JSON.parse(init.body), { text: 'hello' });
});

test('a non-2xx becomes an ApiError carrying the server slug', async () => {
  harness({ status: 409, payload: { error: 'live_session' } });
  writeToken('tok');
  await assert.rejects(
    () => api.sendPrompt('s', 'x', 'k'),
    (e) => {
      assert.ok(e instanceof ApiError);
      assert.equal(e.status, 409);
      assert.equal(e.code, 'live_session');
      return true;
    },
  );
});

test('an error body that is not JSON still yields a usable code', async () => {
  // A proxy or a crash can return HTML. The screen must still be able
  // to say something specific rather than throwing on the parse.
  harness({ status: 502, payload: null });
  writeToken('tok');
  await assert.rejects(
    () => api.sessions(),
    (e) => e instanceof ApiError && e.status === 502 && e.code === 'http_502',
  );
});

test('a network failure is Offline, not an ApiError', async () => {
  // The composer distinguishes these: offline says "Offline.", an
  // ApiError says what the server refused.
  harness({ throws: new TypeError('Failed to fetch') });
  writeToken('tok');
  await assert.rejects(() => api.sessions(), (e) => e instanceof OfflineError);
});

test('an abort is re-thrown untouched, not reported as offline', async () => {
  // Every poll aborts on unmount. Classifying that as offline would
  // paint the whole panel disconnected on a routine navigation.
  const abort = new Error('aborted');
  abort.name = 'AbortError';
  harness({ throws: abort });
  writeToken('tok');
  await assert.rejects(
    () => api.sessions(),
    (e) => e.name === 'AbortError' && !(e instanceof OfflineError),
  );
});

test('a 204 is null rather than a parse failure', async () => {
  harness({ status: 204, payload: null });
  writeToken('tok');
  assert.equal(await api.sessions(), null);
});

/**
 * A read with no deadline is a screen that says "Loading…" for good: `fetch`
 * waits as long as the connection stays open, and now that a poll waits for
 * the request before it, one stuck read would also stop every poll after it.
 */
function hangingFetch() {
  const calls = [];
  globalThis.fetch = (path, init) =>
    new Promise((_, reject) => {
      calls.push({ path, init });
      if (init.signal?.aborted) reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
      init.signal?.addEventListener('abort', () =>
        reject(Object.assign(new Error('aborted'), { name: 'AbortError' })),
      );
    });
  return calls;
}

test('a read that never answers becomes OfflineError at its deadline', async (t) => {
  harness();
  const calls = hangingFetch();
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const read = api.sessions();
  const seen = assert.rejects(read, OfflineError);
  t.mock.timers.tick(READ_DEADLINE_MS - 1);
  assert.equal(calls[0].init.signal.aborted, false, 'still inside the deadline');
  t.mock.timers.tick(1);
  assert.equal(calls[0].init.signal.aborted, true);
  await seen;
});

test('a slow read inside its deadline still returns', async (t) => {
  harness();
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let answer;
  globalThis.fetch = () => new Promise((resolve) => { answer = resolve; });
  const read = api.sessions();
  t.mock.timers.tick(READ_DEADLINE_MS - 1);
  answer({ ok: true, status: 200, text: async () => '{"sessions":[]}' });
  assert.deepEqual(await read, { sessions: [] });
});

test('the caller can still abort a read, and that stays an abort', async () => {
  harness();
  hangingFetch();
  const ctrl = new AbortController();
  const read = api.sessions(ctrl.signal);
  ctrl.abort();
  await assert.rejects(read, (e) => e.name === 'AbortError');
});

test('a signal already aborted stops the read', async () => {
  harness();
  hangingFetch();
  const ctrl = new AbortController();
  ctrl.abort();
  await assert.rejects(api.sessions(ctrl.signal), (e) => e.name === 'AbortError');
});

// "Cannot reach this Mac — nothing was sent" would be false: the prompt may
// have arrived and only the answer got lost.
test('a mutation has no deadline, because giving up would misreport it', async (t) => {
  harness();
  const calls = hangingFetch();
  t.mock.timers.enable({ apis: ['setTimeout'] });
  api.sendPrompt('s', 'hi', 'key');
  t.mock.timers.tick(READ_DEADLINE_MS * 10);
  assert.equal(calls[0].init.signal, undefined);
});
