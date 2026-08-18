// Adaptive elapsed-duration formatting for live agent runs.
//
// Granularity by magnitude, per dev-docs/agents-run-visibility-plan.md
// §2.5. Seconds ticking for a fifteen-minute run is visual noise, and a
// surface that flickers gets ignored — which defeats the point of an
// ambient indicator.
//
// Pure and locale-independent by design: the unit suffixes are the same
// short forms in both catalogs today, and threading i18n through a
// function called on a 1s timer would re-resolve the catalog every tick
// for no reader benefit. If a locale ever needs different units, this
// becomes a `t()` call — not a second formatter.

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;

/**
 * Format a running duration.
 *
 * | Elapsed  | Rendered  |
 * |----------|-----------|
 * | < 2 min  | `1m 42s`  |
 * | < 1 hour | `14m`     |
 * | >= 1 hour| `1h 12m`  |
 *
 * Negative input clamps to zero: a clock skew between the backend's
 * `started_ms` and the renderer's `Date.now()` must render `0s`, never
 * a negative duration, which would read as a bug in the app rather than
 * in the clock.
 */
export function formatElapsed(ms: number): string {
  const t = Math.max(0, ms);
  if (t < 2 * MINUTE) {
    const m = Math.floor(t / MINUTE);
    const s = Math.floor((t % MINUTE) / SECOND);
    return m > 0 ? `${m}m ${s}s` : `${s}s`;
  }
  if (t < HOUR) {
    return `${Math.floor(t / MINUTE)}m`;
  }
  const h = Math.floor(t / HOUR);
  const m = Math.floor((t % HOUR) / MINUTE);
  return `${h}h ${m}m`;
}
