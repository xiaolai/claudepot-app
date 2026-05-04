/**
 * Per-token, per-day rate limiting backed by api_token_usage.
 *
 * One INSERT … ON CONFLICT DO UPDATE per request; the new count comes back
 * via RETURNING so we can compare against the limit in a single round trip.
 * Buckets keyed by UTC date so the reset boundary is unambiguous.
 *
 * Per-token override (api_tokens.rate_limits jsonb) is intentionally NOT
 * implemented in this slice — defaults below cover the bot use cases. Add
 * the column + override path when a real reason appears (staff-issued
 * high-volume token, abuse mitigation, paid tier).
 */

import { sql } from "drizzle-orm";
import { db } from "@/db/client";

export const DEFAULT_DAILY_LIMITS = {
  submissions: 30,
  comments: 200,
  votes: 1000,
  saves: 1000,
  reads: 10_000,
} as const;

export type LimitCategory = keyof typeof DEFAULT_DAILY_LIMITS;

const COLUMN_BY_CATEGORY: Record<LimitCategory, string> = {
  submissions: "submissions_count",
  comments: "comments_count",
  votes: "votes_count",
  saves: "saves_count",
  reads: "reads_count",
};

export type RateLimitResult =
  | { ok: true; remaining: number; limit: number; resetAt: Date }
  | { ok: false; limit: number; resetAt: Date };

function nextUtcMidnight(): Date {
  const d = new Date();
  d.setUTCHours(24, 0, 0, 0);
  return d;
}

function todayUtcDate(): string {
  return new Date().toISOString().slice(0, 10);
}

/**
 * Atomically increments the bucket counter for `category` and returns
 * whether the request is within the limit.
 *
 * The counter is bumped first, then compared — slightly over-counts when
 * over the limit, but daily granularity makes that drift irrelevant.
 */
export async function checkAndIncrement(
  tokenId: string,
  category: LimitCategory,
  limit: number = DEFAULT_DAILY_LIMITS[category],
): Promise<RateLimitResult> {
  const col = COLUMN_BY_CATEGORY[category]; // closed-enum, safe for sql.raw
  const bucketDate = todayUtcDate();
  const resetAt = nextUtcMidnight();

  const result = await db.execute<{ count: number }>(sql`
    INSERT INTO api_token_usage (token_id, bucket_date, ${sql.raw(col)})
    VALUES (${tokenId}, ${bucketDate}, 1)
    ON CONFLICT (token_id, bucket_date)
    DO UPDATE SET ${sql.raw(col)} = api_token_usage.${sql.raw(col)} + 1
    RETURNING ${sql.raw(col)} AS count
  `);

  const count = Number(result.rows[0]?.count ?? 0);
  if (count > limit) return { ok: false, limit, resetAt };
  return { ok: true, remaining: Math.max(0, limit - count), limit, resetAt };
}
