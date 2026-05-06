/**
 * Quota readout for the calling token.
 *
 * Reads the SAME daily counters that `checkAndIncrement` writes (see
 * api_token_usage), so the value returned by /me/quota is exactly what
 * the next mutation will compare against. No write side-effects: the
 * read does not bump any bucket, which is the whole point of /me/quota
 * (introspection without consuming budget).
 */

import { and, eq } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokenUsage } from "@/db/schema";
import { DEFAULT_DAILY_LIMITS, type LimitCategory } from "./rate-limit";
import type { QuotaBucket, QuotaDto } from "./dto";

function todayUtcDate(): string {
  return new Date().toISOString().slice(0, 10);
}

function nextUtcMidnight(): Date {
  const d = new Date();
  d.setUTCHours(24, 0, 0, 0);
  return d;
}

const ZERO_USAGE: Record<LimitCategory, number> = {
  submissions: 0,
  comments: 0,
  votes: 0,
  saves: 0,
  reads: 0,
};

export async function readQuotaForToken(tokenId: string): Promise<QuotaDto> {
  const bucketDate = todayUtcDate();
  const resetsAtIso = nextUtcMidnight().toISOString();

  const [row] = await db
    .select({
      submissions: apiTokenUsage.submissionsCount,
      comments: apiTokenUsage.commentsCount,
      votes: apiTokenUsage.votesCount,
      saves: apiTokenUsage.savesCount,
      reads: apiTokenUsage.readsCount,
    })
    .from(apiTokenUsage)
    .where(
      and(
        eq(apiTokenUsage.tokenId, tokenId),
        eq(apiTokenUsage.bucketDate, bucketDate),
      ),
    )
    .limit(1);

  const used = row ?? ZERO_USAGE;

  const bucket = (cat: LimitCategory): QuotaBucket => ({
    used: used[cat],
    limit: DEFAULT_DAILY_LIMITS[cat],
    resetsAt: resetsAtIso,
  });

  return {
    buckets: {
      submissions: bucket("submissions"),
      comments: bucket("comments"),
      votes: bucket("votes"),
      saves: bucket("saves"),
      reads: bucket("reads"),
    },
  };
}
