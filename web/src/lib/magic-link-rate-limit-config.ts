/**
 * Pure pieces of the magic-link send throttle.
 *
 * Separated from the DB-backed checker (magic-link-rate-limit.ts) so
 * unit tests can import them without pulling in @/db/client, which
 * requires a connection string at import time — same split as
 * moderation/ladder.ts vs moderation/ladder-config.ts.
 */

export const MAGIC_LINK_LIMITS = {
  /** Sends per normalized email address per UTC hour. */
  perEmailPerHour: 3,
  /** Sends per client IP per UTC hour. */
  perIpPerHour: 10,
} as const;

/**
 * Throttle-key normalization: trim + lowercase. Auth.js normalizes the
 * identifier it stores, but the throttle key is computed from the raw
 * form input, so "  Foo@Bar.COM " must bucket with "foo@bar.com".
 */
export function normalizeEmail(email: string): string {
  return email.trim().toLowerCase();
}

/** UTC hour bucket — `now` truncated to the start of its UTC hour. */
export function hourBucketUtc(now: Date = new Date()): Date {
  const d = new Date(now.getTime());
  d.setUTCMinutes(0, 0, 0);
  return d;
}

/**
 * Decision given post-increment counters (count-then-compare, like
 * lib/api/rate-limit.ts). `ipCount` is 0 when the client IP could not
 * be determined and the IP bucket was skipped — the per-email limit
 * still applies, which is the per-victim-inbox protection.
 */
export function withinLimits(emailCount: number, ipCount: number): boolean {
  return (
    emailCount <= MAGIC_LINK_LIMITS.perEmailPerHour &&
    ipCount <= MAGIC_LINK_LIMITS.perIpPerHour
  );
}

/**
 * Client IP from request headers: first hop of x-forwarded-for (set by
 * Vercel's edge, attacker cannot strip it), else x-real-ip, else null.
 * Accepts any Headers-shaped getter so tests can pass a stub.
 */
export function clientIpFromHeaders(h: {
  get(name: string): string | null;
}): string | null {
  const fwd = h.get("x-forwarded-for");
  if (fwd) {
    const first = fwd.split(",")[0]?.trim();
    if (first) return first;
  }
  const real = h.get("x-real-ip")?.trim();
  return real ? real : null;
}
