/**
 * DB-backed fixed-window throttle for magic-link email sends.
 *
 * The magic-link path is the one unauthenticated endpoint that sends
 * paid (Resend) email to an arbitrary, attacker-chosen address — a
 * loop can burn quota and get the sending domain blacklisted. This
 * limiter is called from the Auth.js `signIn` callback (src/lib/auth.ts)
 * on `email.verificationRequest`, i.e. BEFORE the email is sent, which
 * covers both the /login server action and the raw
 * /api/auth/signin/resend endpoint.
 *
 * Storage is one row per (key, UTC hour) in magic_link_sends
 * (migration 0040), bumped atomically via INSERT … ON CONFLICT DO
 * UPDATE … RETURNING — the same single-round-trip pattern as
 * lib/api/rate-limit.ts. The counter is bumped first, then compared,
 * so it slightly over-counts when over the limit; irrelevant at this
 * granularity.
 *
 * Failure policy: the limiter protects spend and domain reputation,
 * not authorization — on DB errors it fails OPEN (logs and allows)
 * so a transient blip can't lock everyone out of login.
 */

import { sql } from "drizzle-orm";
import { headers } from "next/headers";

import { db } from "@/db/client";
import {
  clientIpFromHeaders,
  hourBucketUtc,
  normalizeEmail,
  withinLimits,
} from "./magic-link-rate-limit-config";

async function bumpAndCount(key: string, bucket: Date): Promise<number> {
  const result = await db.execute<{ count: number }>(sql`
    INSERT INTO magic_link_sends (key, bucket_hour, count)
    VALUES (${key}, ${bucket.toISOString()}, 1)
    ON CONFLICT (key, bucket_hour)
    DO UPDATE SET count = magic_link_sends.count + 1
    RETURNING count
  `);
  return Number(result.rows[0]?.count ?? 0);
}

/**
 * Charge one send attempt against the email and IP buckets and return
 * whether the send is within limits. The caller decides how to mask a
 * `false` (the signIn callback redirects to the verify-request page so
 * throttling is indistinguishable from success — no account oracle).
 */
export async function allowMagicLinkSend(emailRaw: string): Promise<boolean> {
  let ip: string | null = null;
  try {
    ip = clientIpFromHeaders(await headers());
  } catch {
    // headers() throws outside a request scope (e.g. build-time module
    // probes). Unknown IP skips the IP bucket; the per-email limit
    // still applies.
  }

  try {
    const bucket = hourBucketUtc();
    const emailCount = await bumpAndCount(
      `email:${normalizeEmail(emailRaw)}`,
      bucket,
    );
    const ipCount = ip ? await bumpAndCount(`ip:${ip}`, bucket) : 0;
    const allowed = withinLimits(emailCount, ipCount);

    // Opportunistic prune — sends are rare and the table stays tiny,
    // so one bounded DELETE per send beats a dedicated cron. Best
    // effort: a prune failure must not affect the decision.
    try {
      await db.execute(
        sql`DELETE FROM magic_link_sends WHERE bucket_hour < now() - interval '24 hours'`,
      );
    } catch {
      // best-effort
    }

    return allowed;
  } catch (err) {
    console.error("[magic-link-rate-limit] check failed; allowing send:", err);
    return true;
  }
}
