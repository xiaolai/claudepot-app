// Touch gestures for the panel.
//
// There were none. The panel is a phone app whose only way out of a
// conversation was a chevron in the top-left corner — the hardest
// target on a large screen, reached with the hand that is not holding
// the device.
//
// Two things are deliberately separate here, and only the second lives
// in this file:
//
//   1. The OS back gesture. iOS Safari's edge swipe and Android's back
//      gesture both drive session history, and the panel kept no
//      history at all — one entry for its whole life — so that gesture
//      left the app instead of closing the thread. `Panel` fixes that
//      with `pushState`/`popstate`; it needs no touch code, and it is
//      the higher-value half.
//   2. This: a swipe from ANYWHERE in the transcript, not just the
//      screen edge. It complements the OS gesture rather than
//      duplicating it — the edge is already spoken for in a browser
//      tab, and in a standalone home-screen app the OS gesture may not
//      be there at all.

/** Past this, a horizontal drag counts as a swipe. */
const DISTANCE_PX = 64;

/**
 * How much more horizontal than vertical the drag has to be.
 *
 * The transcript is a tall scrolling column, so almost every touch in
 * it is meant as a scroll. A generous ratio is what keeps a diagonal
 * flick from closing the conversation someone was reading.
 */
const RATIO = 1.6;

/** Slower than this and it reads as a drag, not a flick. */
const DURATION_MS = 800;

/**
 * Can this element scroll horizontally, right now, in this direction?
 *
 * Markdown tables and code blocks inside a transcript are
 * `overflow-x: auto` (see `markdown.css`), and a wide code block is
 * exactly the thing a reader swipes sideways. Stealing that gesture
 * would make the block unreadable — strictly worse than having no
 * swipe at all — so a drag that starts inside one is not a swipe.
 *
 * Checked against the live scroll position rather than the mere
 * presence of overflow: a table already scrolled to its left edge has
 * nothing more to give in that direction, and the gesture is free.
 */
function scrollableAt(target, root, dx) {
  let el = target;
  while (el && el !== root && el.nodeType === 1) {
    const canOverflow = el.scrollWidth > el.clientWidth + 1;
    if (canOverflow) {
      const style = getComputedStyle(el);
      if (style.overflowX === 'auto' || style.overflowX === 'scroll') {
        // Swiping right wants to reveal content on the left, so the
        // element only claims the gesture when it has some.
        if (dx > 0 && el.scrollLeft > 0) return true;
        if (dx < 0 && el.scrollLeft + el.clientWidth < el.scrollWidth - 1) return true;
      }
    }
    el = el.parentElement;
  }
  return false;
}

/**
 * Recognise a horizontal swipe over an element.
 *
 * Returns handlers to spread onto a container. `onSwipeRight` fires on
 * the platform's universal go-back direction. `onSwipeLeft` is accepted
 * for symmetry and nothing passes it, deliberately: the obvious
 * candidate is switching tabs, and a tab bar is lateral rather than
 * deeper — the same reason `Panel` gives no tab a history entry. A
 * swipe that moved sideways through tabs would collide with the one
 * direction this file exists to recognise.
 *
 * Pure of React so it can be tested without a renderer; jsdom has no
 * touch input, so the decision function below is the part that gets
 * asserted, and the wiring is exercised by the panel's render check.
 */
export function swipeHandlers({ onSwipeRight, onSwipeLeft, enabled = true }) {
  if (!enabled) return {};
  let start = null;

  return {
    onTouchStart(e) {
      // Multi-touch is a pinch or a two-finger scroll; neither is this.
      if (e.touches.length !== 1) {
        start = null;
        return;
      }
      const t = e.touches[0];
      start = { x: t.clientX, y: t.clientY, at: Date.now(), target: e.target };
    },
    onTouchMove(e) {
      // A second finger arriving mid-drag cancels it.
      if (start && e.touches.length !== 1) start = null;
    },
    onTouchEnd(e) {
      const from = start;
      start = null;
      if (!from) return;
      const t = e.changedTouches && e.changedTouches[0];
      if (!t) return;

      const decision = decideSwipe({
        dx: t.clientX - from.x,
        dy: t.clientY - from.y,
        ms: Date.now() - from.at,
      });
      if (!decision) return;
      if (scrollableAt(from.target, e.currentTarget, t.clientX - from.x)) return;

      if (decision === 'right' && onSwipeRight) onSwipeRight();
      if (decision === 'left' && onSwipeLeft) onSwipeLeft();
    },
    onTouchCancel() {
      start = null;
    },
  };
}

/**
 * The pure decision: `'left'`, `'right'`, or null for "that was a
 * scroll".
 *
 * Split out from the handlers because it is the whole judgement, and
 * because jsdom cannot deliver a real touch — this is the part a test
 * can hold.
 */
export function decideSwipe({ dx, dy, ms }) {
  if (ms > DURATION_MS) return null;
  if (Math.abs(dx) < DISTANCE_PX) return null;
  if (Math.abs(dx) < Math.abs(dy) * RATIO) return null;
  return dx > 0 ? 'right' : 'left';
}

export const SWIPE = { DISTANCE_PX, RATIO, DURATION_MS };
