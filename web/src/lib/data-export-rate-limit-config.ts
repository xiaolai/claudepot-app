/**
 * Pure pieces of the data-export send throttle.
 *
 * Separated from the DB-backed checker (data-export-rate-limit.ts) so
 * unit tests can import them without pulling in @/db/client, which
 * requires a connection string at import time — same split as
 * magic-link-rate-limit.ts vs magic-link-rate-limit-config.ts.
 */

export const DATA_EXPORT_LIMITS = {
  /** Export emails per user per UTC day. */
  perUserPerDay: 2,
} as const;

/** UTC day bucket — `now` truncated to the start of its UTC day. */
export function dayBucketUtc(now: Date = new Date()): Date {
  const d = new Date(now.getTime());
  d.setUTCHours(0, 0, 0, 0);
  return d;
}

/**
 * Decision given the post-increment counter (count-then-compare, like
 * magic-link-rate-limit-config.ts — "at the limit" is still allowed
 * because the count includes the current attempt).
 */
export function withinExportLimit(count: number): boolean {
  return count <= DATA_EXPORT_LIMITS.perUserPerDay;
}
