/**
 * Shared formatting helpers for the projects subtree.
 *
 * `formatSize` and `basename` are re-exports of the canonical lib
 * implementations (`src/lib/format.ts`, `src/lib/paths.ts`) — kept
 * here so the projects subtree's many importers don't churn
 * (audit 2026-07 F10/F12).
 */

export { formatSize } from "../../lib/format";
export { basename } from "../../lib/paths";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

/**
 * Compact "N unit ago" for a past instant in ms. Coarsest unit wins;
 * always rounded down so "5 minutes and 50 seconds ago" → "5m ago".
 * For instants in the future (clock skew) returns "just now" rather
 * than confusing the user with a negative magnitude.
 *
 * Deliberately NOT consolidated onto `lib/formatRelative` — that
 * ladder shows seconds ("12s ago"), caps hours at 48, and has no
 * week tier, so merging would change user-visible strings on every
 * projects surface (audit 2026-07 F13, drift risk accepted).
 */
export function formatRelativeTime(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < MINUTE) return "just now";
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)}m ago`;
  if (diff < DAY) return `${Math.floor(diff / HOUR)}h ago`;
  if (diff < WEEK) return `${Math.floor(diff / DAY)}d ago`;
  return `${Math.floor(diff / WEEK)}w ago`;
}
