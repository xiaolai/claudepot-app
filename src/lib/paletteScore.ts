/**
 * Scored fuzzy matching for the ⌘K palette.
 *
 * The palette previously used a bare subsequence test, which is a
 * boolean: "set" matched both "Open **Set**tings" and
 * "**S**ign D**e**sktop ou**t**", and results kept their production
 * order, so the scattered match could outrank the exact one. Scoring
 * makes the ordering answer "how well does this match" instead of
 * "which producer ran first".
 *
 * Tiers are spaced 100 apart and every within-tier penalty sums to
 * less than 100, so a weaker match can never outrank a stronger tier
 * no matter how long the text is. That invariant is what the tests
 * pin; the exact constants are free to move.
 */

const TIER_EXACT = 1000;
const TIER_PREFIX = 900;
const TIER_WORD_START = 800;
const TIER_SUBSTRING = 700;
const TIER_SUBSEQUENCE = 400;

/** Characters that start a new "word" for word-boundary matching. */
const BOUNDARY = /[\s\-_/\\.:·(),[\]]/;

/** Max 24. Prefers the shorter of two otherwise-equal matches. */
function lengthPenalty(len: number): number {
  return Math.min(len, 120) * 0.2;
}

/** Max 30. Prefers a match nearer the start of the text. */
function positionPenalty(index: number): number {
  return Math.min(index, 60) * 0.5;
}

/** Max 30. Prefers a subsequence whose characters sit close together. */
function spanPenalty(span: number, queryLen: number): number {
  return Math.min(Math.max(span - queryLen, 0), 200) * 0.15;
}

/**
 * Greedy forward subsequence match. Returns the span (distance from
 * first to last matched character) or null when `q` is not a
 * subsequence of `t`. Greedy-forward is not guaranteed minimal, but it
 * is O(n) and the ranking only needs a monotone "how scattered" proxy.
 */
function subsequenceSpan(q: string, t: string): number | null {
  let qi = 0;
  let first = -1;
  let last = -1;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      if (first === -1) first = ti;
      last = ti;
      qi++;
    }
  }
  if (qi !== q.length) return null;
  return last - first + 1;
}

/**
 * Score how well `query` matches `text`. Higher is better; `null`
 * means no match at all. An empty query matches everything at 0 so
 * callers can keep their natural order.
 */
export function scoreMatch(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  if (!t) return null;

  const idx = t.indexOf(q);
  if (idx === 0) {
    return (
      (t.length === q.length ? TIER_EXACT : TIER_PREFIX) -
      lengthPenalty(t.length)
    );
  }
  if (idx > 0) {
    const prev = t[idx - 1] ?? "";
    const tier = BOUNDARY.test(prev) ? TIER_WORD_START : TIER_SUBSTRING;
    return tier - lengthPenalty(t.length) - positionPenalty(idx);
  }

  const span = subsequenceSpan(q, t);
  if (span === null) return null;
  return TIER_SUBSEQUENCE - lengthPenalty(t.length) - spanPenalty(span, q.length);
}

/**
 * Best score across a primary label and any number of secondary
 * fields (detail text, keyword synonyms). Secondary fields are
 * discounted so a label hit always beats an equally-good keyword hit
 * — the user typed what they see.
 */
export function scoreFields(
  query: string,
  label: string,
  secondary: readonly (string | undefined)[] = [],
): number | null {
  let best = scoreMatch(query, label);
  for (const field of secondary) {
    if (!field) continue;
    const s = scoreMatch(query, field);
    if (s === null) continue;
    const discounted = s - 50;
    if (best === null || discounted > best) best = discounted;
  }
  return best;
}
