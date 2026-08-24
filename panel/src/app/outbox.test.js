import test from 'node:test';
import assert from 'node:assert/strict';

/**
 * The offline queue.
 *
 * This is the only thing in the panel that sends a message the user is
 * not present for, so its edges are worth pinning: the idempotency key
 * has to survive the round trip that a retry replays, a refused entry
 * has to stop being retried, and both caps have to drop the right end
 * of the list. The drain itself is exercised end-to-end by nothing —
 * these cover the store it drains.
 */

/** Minimal `window` with a real-enough localStorage. */
function harness(seed) {
  const store = new Map();
  if (seed !== undefined) store.set('claudepot.outbox', seed);
  globalThis.window = {
    localStorage: {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => store.set(k, String(v)),
      removeItem: (k) => store.delete(k),
    },
  };
  return store;
}

/** A fresh module instance, because the store caches in a module local. */
async function load(seed) {
  const store = harness(seed);
  const mod = await import(`./outbox.js?t=${Math.random()}`);
  return { mod, store };
}

test('a held message survives a reload', async () => {
  const { mod, store } = await load();
  mod.enqueue('s1', 'do the thing', 'key-1');

  // A second module instance reads the same localStorage — which is
  // what an evicted home-screen app does when the OS restarts it.
  const fresh = await load(store.get('claudepot.outbox'));
  assert.deepEqual(
    fresh.mod.queuedFor('s1').map((m) => [m.id, m.text]),
    [['key-1', 'do the thing']],
  );
});

test('the idempotency key is the entry id, so a replayed drain is one intent', async () => {
  const { mod } = await load();
  mod.enqueue('s1', 'a', 'key-a');
  const [entry] = mod.queuedFor('s1');
  // The drain presents `m.id` as the idempotency key. If enqueue ever
  // stops minting it here, a drain that sends and loses the answer
  // would send again under a new key and the message would land twice.
  assert.equal(entry.id, 'key-a');
});

test('order is preserved — these are instructions to one conversation', async () => {
  const { mod } = await load();
  mod.enqueue('s1', 'first', 'k1');
  mod.enqueue('s1', 'second', 'k2');
  mod.enqueue('s1', 'third', 'k3');
  assert.deepEqual(
    mod.queuedFor('s1').map((m) => m.text),
    ['first', 'second', 'third'],
  );
});

test('a session with nothing queued returns the same empty array every time', async () => {
  const { mod } = await load();
  // Referential stability: a subscriber comparing snapshots must not
  // see a change on every read, or it re-renders forever.
  assert.equal(mod.queuedFor('nobody'), mod.queuedFor('nobody'));
  assert.equal(mod.queuedFor('nobody').length, 0);
});

test('the per-session cap drops the OLDEST, not the newest', async () => {
  const { mod } = await load();
  for (let i = 0; i < 25; i += 1) mod.enqueue('s1', `m${i}`, `k${i}`);
  const held = mod.queuedFor('s1');
  assert.equal(held.length, 20);
  // What you typed most recently is what you still meant.
  assert.equal(held[0].text, 'm5');
  assert.equal(held[19].text, 'm24');
});

test('the session cap drops the least recently queued session', async () => {
  const { mod } = await load();
  for (let i = 0; i < 41; i += 1) mod.enqueue(`s${i}`, 'x', `k${i}`);
  assert.equal(mod.queuedFor('s0').length, 0, 's0 was the oldest and should be gone');
  assert.equal(mod.queuedFor('s40').length, 1, 's40 was the newest and should be kept');
});

test('cancel removes one entry and leaves the rest', async () => {
  const { mod } = await load();
  mod.enqueue('s1', 'keep', 'k1');
  mod.enqueue('s1', 'drop', 'k2');
  mod.cancel('s1', 'k2');
  assert.deepEqual(
    mod.queuedFor('s1').map((m) => m.text),
    ['keep'],
  );
});

test('cancelling the last entry drops the session key rather than leaving an empty list', async () => {
  const { mod, store } = await load();
  mod.enqueue('s1', 'only', 'k1');
  mod.cancel('s1', 'k1');
  assert.deepEqual(JSON.parse(store.get('claudepot.outbox')), {});
});

test('a refused entry is marked, not removed — the user decides', async () => {
  const { mod } = await load();
  mod.enqueue('s1', 'a', 'k1');
  mod.markFailed('s1', 'k1', 'That session is no longer running.');
  const [entry] = mod.queuedFor('s1');
  assert.equal(entry.failed, 'That session is no longer running.');
  assert.equal(entry.text, 'a', 'the text must survive so it can be retried or read');
});

test('retry clears the mark, so the drain picks it up again', async () => {
  const { mod } = await load();
  mod.enqueue('s1', 'a', 'k1');
  mod.markFailed('s1', 'k1', 'nope');
  mod.retry('s1', 'k1');
  assert.equal('failed' in mod.queuedFor('s1')[0], false);
});

test('totalQueued counts across sessions', async () => {
  const { mod } = await load();
  mod.enqueue('s1', 'a', 'k1');
  mod.enqueue('s1', 'b', 'k2');
  mod.enqueue('s2', 'c', 'k3');
  assert.equal(mod.totalQueued(), 3);
});

test('subscribers are told on every change', async () => {
  const { mod } = await load();
  let n = 0;
  const off = mod.subscribe(() => {
    n += 1;
  });
  mod.enqueue('s1', 'a', 'k1');
  mod.markFailed('s1', 'k1', 'nope');
  mod.cancel('s1', 'k1');
  off();
  mod.enqueue('s1', 'b', 'k2');
  assert.equal(n, 3, 'three changes while subscribed, none after unsubscribing');
});

test('a corrupt file costs retyping, not a boot failure', async () => {
  const { mod } = await load('{not json at all');
  assert.deepEqual(mod.queuedFor('s1'), []);
  // And it still works afterwards, rather than throwing on every write.
  mod.enqueue('s1', 'a', 'k1');
  assert.equal(mod.queuedFor('s1').length, 1);
});

test('a file holding the wrong SHAPE is treated as corrupt, not trusted', async () => {
  // An array parses fine, so a `typeof === 'object'` check alone lets it
  // through — and then `Object.keys` walks its INDICES and reads
  // `.length` off each element. Seeded with one string that is exactly
  // what `totalQueued` reported before the `Array.isArray` guard: 4
  // messages held, none of which exist.
  const { mod } = await load('["junk"]');
  assert.equal(mod.totalQueued(), 0);
  mod.enqueue('s1', 'a', 'k1');
  assert.equal(mod.totalQueued(), 1);
});
