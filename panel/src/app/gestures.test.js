/**
 * The swipe decision, which is the whole judgement.
 *
 * jsdom cannot deliver a real touch, so the handlers themselves are
 * only provable on a phone. What is testable — and what actually
 * decides whether the gesture is usable or infuriating — is the
 * predicate: a transcript is a tall scrolling column, so nearly every
 * touch in it is meant as a scroll, and a swipe that fires on those
 * closes the conversation someone was reading.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { decideSwipe, swipeHandlers, SWIPE } from './gestures.js';

/** jsdom is not available here (`node --test`), so the two scrollable
 *  cases below stand up the minimum DOM shape `scrollableAt` walks:
 *  an element chain with `scrollWidth`/`clientWidth`/`scrollLeft` and a
 *  computed `overflowX`. That is the entire surface it touches. */
function stubDom() {
  const made = [];
  globalThis.getComputedStyle = (el) => ({ overflowX: el.__overflowX ?? 'visible' });
  const el = (over = {}) => {
    const e = {
      nodeType: 1,
      scrollWidth: 0,
      clientWidth: 0,
      scrollLeft: 0,
      parentElement: null,
      ...over,
    };
    made.push(e);
    return e;
  };
  return { el, made };
}

const flick = (over) => ({ dx: 0, dy: 0, ms: 120, ...over });

test('decideSwipe', async (t) => {
  await t.test('recognises a clean horizontal flick in both directions', () => {
    assert.equal(decideSwipe(flick({ dx: 120 })), 'right');
    assert.equal(decideSwipe(flick({ dx: -120 })), 'left');
  });

  await t.test('ignores a scroll', () => {
    // The common case by a wide margin: a vertical drag through a
    // transcript, with the small sideways wobble a thumb always adds.
    assert.equal(decideSwipe(flick({ dx: 12, dy: 300 })), null);
    assert.equal(decideSwipe(flick({ dx: -20, dy: 240 })), null);
  });

  await t.test('ignores a diagonal, however long', () => {
    // Far past the distance threshold, but the reader was scrolling.
    assert.equal(decideSwipe(flick({ dx: 200, dy: 200 })), null);
    assert.equal(decideSwipe(flick({ dx: 200, dy: 130 })), null);
  });

  await t.test('ignores a short drag', () => {
    assert.equal(decideSwipe(flick({ dx: SWIPE.DISTANCE_PX - 1 })), null);
    assert.equal(decideSwipe(flick({ dx: SWIPE.DISTANCE_PX })), 'right');
  });

  await t.test('ignores a slow drag', () => {
    // A long press that wandered sideways is not a flick. Long-press is
    // the copy gesture on this surface (see controls.css), so this
    // boundary is protecting a real interaction.
    assert.equal(decideSwipe(flick({ dx: 200, ms: SWIPE.DURATION_MS + 1 })), null);
    assert.equal(decideSwipe(flick({ dx: 200, ms: SWIPE.DURATION_MS })), 'right');
  });

  await t.test('is exactly at the ratio boundary, not near it', () => {
    // dx must exceed dy * RATIO. Pinned so a later tweak to either
    // constant is a deliberate change rather than a drift.
    const dy = 50;
    assert.equal(decideSwipe(flick({ dx: dy * SWIPE.RATIO - 1, dy })), null);
    assert.equal(decideSwipe(flick({ dx: dy * SWIPE.RATIO + 1, dy })), 'right');
  });
});

test('swipeHandlers', async (t) => {
  await t.test('returns nothing when disabled, so the wide layout binds no touch at all', () => {
    assert.deepEqual(swipeHandlers({ enabled: false, onSwipeRight: () => {} }), {});
  });

  await t.test('binds the four touch events when enabled', () => {
    const h = swipeHandlers({ onSwipeRight: () => {} });
    assert.deepEqual(Object.keys(h).sort(), [
      'onTouchCancel',
      'onTouchEnd',
      'onTouchMove',
      'onTouchStart',
    ]);
  });

  await t.test('fires a right swipe end-to-end', () => {
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    h.onTouchStart({ touches: [{ clientX: 20, clientY: 100 }], target: root });
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 110 }], currentTarget: root });
    assert.equal(fired, 1);
  });

  await t.test('a second finger cancels the drag', () => {
    // Two fingers is a pinch or a two-finger scroll. Without this a
    // pinch-zoom on a diagram could end as a swipe.
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    h.onTouchStart({ touches: [{ clientX: 20, clientY: 100 }], target: root });
    h.onTouchMove({ touches: [{}, {}] });
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 110 }], currentTarget: root });
    assert.equal(fired, 0);
  });

  await t.test('a multi-touch start is not a swipe', () => {
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    h.onTouchStart({ touches: [{}, {}], target: root });
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 100 }], currentTarget: root });
    assert.equal(fired, 0);
  });

  await t.test('a cancelled touch does not fire', () => {
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    h.onTouchStart({ touches: [{ clientX: 20, clientY: 100 }], target: root });
    h.onTouchCancel();
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 110 }], currentTarget: root });
    assert.equal(fired, 0);
  });

  await t.test('leaves a scrollable code block its own gesture', () => {
    // A wide code block inside a transcript is `overflow-x: auto`, and
    // swiping it sideways is how you read it. Stealing that would make
    // the block unreadable — worse than having no swipe at all.
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    // Scrolled away from its left edge, so a rightward swipe is its own.
    const pre = el({
      __overflowX: 'auto',
      scrollWidth: 800,
      clientWidth: 300,
      scrollLeft: 120,
      parentElement: root,
    });
    h.onTouchStart({ touches: [{ clientX: 20, clientY: 100 }], target: pre });
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 105 }], currentTarget: root });
    assert.equal(fired, 0);
  });

  await t.test('takes the gesture back once that block is at its left edge', () => {
    // Nothing left to reveal in that direction, so the swipe is free.
    // Checked against live scroll position rather than the mere
    // presence of overflow — otherwise one wide table in a long
    // transcript would disable the gesture for the whole conversation.
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    const pre = el({
      __overflowX: 'auto',
      scrollWidth: 800,
      clientWidth: 300,
      scrollLeft: 0,
      parentElement: root,
    });
    h.onTouchStart({ touches: [{ clientX: 20, clientY: 100 }], target: pre });
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 105 }], currentTarget: root });
    assert.equal(fired, 1);
  });

  await t.test('a swipe that starts on ordinary prose is not blocked', () => {
    // The overwhelmingly common case: the walk up from the target finds
    // nothing scrollable and the gesture belongs to the thread.
    const { el } = stubDom();
    let fired = 0;
    const h = swipeHandlers({ onSwipeRight: () => (fired += 1) });
    const root = el();
    const p = el({ parentElement: root });
    h.onTouchStart({ touches: [{ clientX: 20, clientY: 100 }], target: p });
    h.onTouchEnd({ changedTouches: [{ clientX: 200, clientY: 105 }], currentTarget: root });
    assert.equal(fired, 1);
  });
});
