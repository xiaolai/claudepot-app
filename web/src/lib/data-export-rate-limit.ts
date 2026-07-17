/**
 * DB-backed fixed-window throttle for data-export emails.
 *
 * requestDataExport (src/lib/actions/settings.ts) dumps every row the
 * user owns and sends a paid (Resend) email per call — a held-down
 * submit button burns quota and hammers the DB. This limiter caps
 * sends at DATA_EXPORT_LIMITS.perUserPerDay per UTC day and is called
 * BEFORE the dump queries run, mirroring the magic-link throttle
 * (src/lib/magic-link-rate-limit.ts, migration 0040) which charges
 * before the email is sent.
 *
 * Storage is one row per (user, UTC day) in data_export_sends
 * (migration 0044), bumped atomically via INSERT … ON CONFLICT DO
 * UPDATE … RETURNING — the same single-round-trip pattern. The
 * counter is bumped first, then compared, so it slightly over-counts
 * when over the limit; irrelevant at this granularity.
 *
 * Failure policy: the limiter protects spend, not authorization — on
 * DB errors it fails OPEN (logs and allows) so a transient blip can't
 * lock a user out of their own data export.
 */

import { sql } from "drizzle-orm";

import { db } from "@/db/client";
import { dayBucketUtc, withinExportLimit } from "./data-export-rate-limit-config";

async function bumpAndCount(userId: string, bucket: Date): Promise<number> {
  const result = await db.execute<{ count: number }>(sql`
    INSERT INTO data_export_sends (user_id, bucket_day, count)
    VALUES (${userId}, ${bucket.toISOString()}, 1)
    ON CONFLICT (user_id, bucket_day)
    DO UPDATE SET count = data_export_sends.count + 1
    RETURNING count
  `);
  return Number(result.rows[0]?.count ?? 0);
}

/**
 * Charge one export attempt against the user's daily bucket and
 * return whether the send is within limits. The caller masks a
 * `false` as success (no distinguishable throttle response), matching
 * the magic-link path's oracle-free style.
 */
export async function allowDataExportSend(userId: string): Promise<boolean> {
  try {
    const count = await bumpAndCount(userId, dayBucketUtc());
    const allowed = withinExportLimit(count);

    // Opportunistic prune — exports are rare and the table stays
    // tiny, so one bounded DELETE per send beats a dedicated cron.
    // Best effort: a prune failure must not affect the decision.
    try {
      await db.execute(
        sql`DELETE FROM data_export_sends WHERE bucket_day < now() - interval '48 hours'`,
      );
    } catch {
      // best-effort
    }

    return allowed;
  } catch (err) {
    console.error("[data-export-rate-limit] check failed; allowing send:", err);
    return true;
  }
}
