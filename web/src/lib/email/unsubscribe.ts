/**
 * Stateless one-click unsubscribe tokens for the weekly digest.
 *
 * Why HMAC over a stored unsubscribe_tokens table: the link goes into
 * outgoing email and is essentially append-only. We never need to
 * revoke a single link (revoking a session-token table is more
 * complex), and round-tripping a DB row on every Gmail-bot click adds
 * latency for no security gain. HMAC under AUTH_SECRET ties the link
 * lifetime to the same secret rotation that already protects every
 * Auth.js session — a secret rotation invalidates all unsubscribe
 * links automatically, which matches the threat model.
 *
 * Mechanism: token = base64url(HMAC-SHA256(secret, "digest:" + userId)).
 * The "digest:" namespace prevents a token from one channel (e.g., a
 * future "comment notifications" mailer) being replayed to unsubscribe
 * from a different channel.
 *
 * Verification uses timingSafeEqual so two valid candidates can't be
 * distinguished by response time.
 */

import { createHmac, timingSafeEqual } from "node:crypto";

const NAMESPACE = "digest:";

function getSecret(): string | null {
  return process.env.AUTH_SECRET ?? null;
}

export function signUnsubscribeToken(userId: string): string | null {
  const secret = getSecret();
  if (!secret) return null;
  return createHmac("sha256", secret)
    .update(NAMESPACE + userId)
    .digest("base64url");
}

export function verifyUnsubscribeToken(
  userId: string,
  candidate: string,
): boolean {
  const expected = signUnsubscribeToken(userId);
  if (!expected) return false;
  // Reject obvious shape mismatches before invoking timingSafeEqual,
  // which throws on length differences.
  if (typeof candidate !== "string" || candidate.length !== expected.length) {
    return false;
  }
  try {
    return timingSafeEqual(
      Buffer.from(expected, "utf8"),
      Buffer.from(candidate, "utf8"),
    );
  } catch {
    return false;
  }
}

export function buildUnsubscribeUrl(
  siteUrl: string,
  userId: string,
): string | null {
  const sig = signUnsubscribeToken(userId);
  if (!sig) return null;
  const url = new URL("/api/unsubscribe/digest", siteUrl);
  url.searchParams.set("u", userId);
  url.searchParams.set("t", sig);
  return url.toString();
}
