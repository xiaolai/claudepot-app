import test from 'node:test';
import assert from 'node:assert/strict';
import { requestSlot } from './poll.js';

/** A request the test answers by hand, and that rejects when aborted. */
function pending() {
  const calls = [];
  const run = (signal) =>
    new Promise((resolve, reject) => {
      const call = { signal, resolve, reject };
      calls.push(call);
      signal.addEventListener('abort', () => {
        reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
      });
    });
  return { run, calls };
}

/** Fails rather than hangs: a request that should have settled and never did. */
function settled(promise) {
  let timer;
  const late = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('never settled')), 1000);
  });
  return Promise.race([promise, late]).finally(() => clearTimeout(timer));
}

// The defect this exists for: every poll aborted the one before it, so a host
// that took longer than the interval to answer was never heard, and nothing
// said so.
test('a tick leaves a slow request alone, and the slow reply arrives', async () => {
  const slot = requestSlot();
  const { run, calls } = pending();
  const first = slot.tick(run);
  const second = await settled(slot.tick(run));
  const third = await settled(slot.tick(run));
  assert.equal(calls.length, 1, 'a tick must not start a second request');
  assert.equal(calls[0].signal.aborted, false, 'a tick must not abort the first');
  assert.equal(second.status, 'dropped');
  assert.equal(third.status, 'dropped');
  calls[0].resolve('slow');
  assert.deepEqual(await settled(first), { status: 'fulfilled', value: 'slow' });
});

test('a tick starts again once the last request has settled', async () => {
  const slot = requestSlot();
  const { run, calls } = pending();
  const first = slot.tick(run);
  calls[0].resolve(1);
  await first;
  const second = slot.tick(run);
  assert.equal(calls.length, 2);
  calls[1].resolve(2);
  assert.equal((await second).value, 2);
});

test('a failed request frees the slot and reports why', async () => {
  const slot = requestSlot();
  const { run, calls } = pending();
  const first = slot.tick(run);
  const why = new Error('boom');
  calls[0].reject(why);
  assert.deepEqual(await first, { status: 'rejected', reason: why });
  slot.tick(run);
  assert.equal(calls.length, 2);
});

// A person asked, or something just changed: the answer already on its way
// describes the world before that.
test('now replaces the request in flight, and the old answer is dropped', async () => {
  const slot = requestSlot();
  const { run, calls } = pending();
  const old = slot.tick(run);
  const fresh = slot.now(run);
  assert.equal(calls[0].signal.aborted, true);
  assert.equal((await old).status, 'dropped');
  calls[1].resolve('fresh');
  assert.equal((await fresh).value, 'fresh');
});

// `fetch` rejects asynchronously, so an answer can already be resolved when
// the request is replaced. It still must not be applied.
test('an answer that ignores its abort is still dropped', async () => {
  const slot = requestSlot();
  const old = slot.tick(() => Promise.resolve('stale'));
  const fresh = slot.now(() => Promise.resolve('fresh'));
  assert.equal((await old).status, 'dropped');
  assert.equal((await fresh).value, 'fresh');
});

test('stop abandons the request and leaves the slot usable', async () => {
  const slot = requestSlot();
  const { run, calls } = pending();
  const old = slot.tick(run);
  slot.stop();
  assert.equal(calls[0].signal.aborted, true);
  assert.equal((await old).status, 'dropped');
  slot.tick(run);
  assert.equal(calls.length, 2);
});

test('stop with nothing in flight does nothing', () => {
  assert.doesNotThrow(() => requestSlot().stop());
});
