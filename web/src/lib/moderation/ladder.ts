/**
 * Ban ladder — rungs 3 + 4 per dev-docs/policy-moderator-plan.md §9.
 *
 * Two reversible/escalation steps the moderator triggers; bans
 * themselves stay staff-only (lib/actions/moderation.ts:lock_user).
 *
 *   - Rung 3: rate-limit shrink. After RUNG3_REJECT_TRIGGER rejects
 *     in the last RUNG3_WINDOW_DAYS days, the per-author daily
 *     moderation-eligible-action cap drops to RUNG3_DAILY_CAP. Auto-
 *     restores once the rolling reject count falls back under the
 *     trigger.
 *
 *   - Rung 4: ban-candidate flag. After RUNG4_REJECT_TRIGGER rejects
 *     in the last RUNG4_WINDOW_DAYS days OR any 'illegal' verdict,
 *     a flag row appears in /admin/queue tagged 'ban_candidate:user=<id>:…'
 *     so staff can review and (optionally) lock_user. Dedup'd: at
 *     most one open ban-candidate flag per user.
 *
 * Numbers are strawman per the plan; tune in production. Constants
 * are the only place to change them — no env vars yet, since
 * silent threshold drift between environments is worse than a
 * conscious code change.
 */

import { and, count, eq, gt, gte, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { comments, flags, policyDecisions, submissions } from "@/db/schema";

import { DAY_MS, LADDER_THRESHOLDS } from "./ladder-config";
import { getSystemUserId } from "./system-user";
import type { ModerationKind, ModerationVerdict } from "./types";

const {
  RUNG3_REJECT_TRIGGER,
  RUNG3_WINDOW_DAYS,
  RUNG3_DAILY_CAP,
  RUNG4_REJECT_TRIGGER,
  RUNG4_WINDOW_DAYS,
} = LADDER_THRESHOLDS;

export interface LadderRateLimitDecision {
  rateLimited: boolean;
  /** Human-readable explanation when rate-limited; null otherwise. */
  reason: string | null;
}

/**
 * Counts an author's `verdict='reject'` rows in policy_decisions
 * within the trailing `windowDays` window. Powers both rungs.
 *
 * Strike-counting rules:
 *   - Pass-1 reject is the strike. Pass-2 confirms or overturns it
 *     but is not its own strike — counting both would double-count
 *     a single moderation event.
 *   - Pass-1 rejects whose pass-2 cleared (verdict='pass') don't
 *     count: the plan's optimistic-publish design says an overturned
 *     reject is not a strike against the author.
 *   - Submission rejects always count (no pass-2 path exists).
 *   - Illegal comment blocks count (target_id=NULL means no pass-2
 *     row can match the NOT EXISTS predicate, so it holds).
 */
export async function recentRejectsForAuthor(
  authorId: string,
  windowDays: number,
): Promise<number> {
  const cutoff = new Date(Date.now() - windowDays * DAY_MS);
  const [row] = await db
    .select({ n: count() })
    .from(policyDecisions)
    .where(
      and(
        eq(policyDecisions.authorId, authorId),
        eq(policyDecisions.verdict, "reject"),
        eq(policyDecisions.passNumber, 1),
        gt(policyDecisions.decidedAt, cutoff),
        sql`NOT EXISTS (
          SELECT 1 FROM ${policyDecisions} p2
           WHERE p2.target_type = ${policyDecisions.targetType}
             AND p2.target_id   = ${policyDecisions.targetId}
             AND p2.pass_number = 2
             AND p2.verdict     = 'pass'
        )`,
      ),
    );
  return row?.n ?? 0;
}

/** Counts the author's submissions + comments created since UTC midnight today. */
async function todayContentCountForAuthor(authorId: string): Promise<number> {
  const now = new Date();
  const startOfDayUtc = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const [s] = await db
    .select({ n: count() })
    .from(submissions)
    .where(
      and(
        eq(submissions.authorId, authorId),
        gte(submissions.createdAt, startOfDayUtc),
      ),
    );
  const [c] = await db
    .select({ n: count() })
    .from(comments)
    .where(
      and(
        eq(comments.authorId, authorId),
        gte(comments.createdAt, startOfDayUtc),
      ),
    );
  return (s?.n ?? 0) + (c?.n ?? 0);
}

/**
 * Rung 3 check. Call BEFORE moderate() at the top of createSubmission /
 * createComment. Returns `{ rateLimited: true }` when the author has
 * already burned through the reduced daily cap; the caller short-
 * circuits with a 'rate' result so the moderator never even runs.
 *
 * Two queries on the hot path. If they show up in EXPLAIN, denormalize
 * onto users — but at v0 the cost is negligible.
 */
export async function checkLadderRateLimit(
  authorId: string,
): Promise<LadderRateLimitDecision> {
  const rejects = await recentRejectsForAuthor(authorId, RUNG3_WINDOW_DAYS);
  if (rejects < RUNG3_REJECT_TRIGGER) {
    return { rateLimited: false, reason: null };
  }
  const todayCount = await todayContentCountForAuthor(authorId);
  if (todayCount >= RUNG3_DAILY_CAP) {
    return {
      rateLimited: true,
      reason: `Daily limit ${RUNG3_DAILY_CAP}/day after ${rejects} moderation rejects in the last ${RUNG3_WINDOW_DAYS} days. Resets at UTC midnight.`,
    };
  }
  return { rateLimited: false, reason: null };
}

/**
 * Rung 4 check. Call AFTER moderate() returns reject AND the
 * policy_decisions row has been written, so `recentRejects` reflects
 * the latest reject inclusive.
 *
 * Inserts a `flags` row with target_type='user', target_id=authorId,
 * reason='ban_candidate:trigger=<...>: <one_line>' if:
 *   1. The latest verdict's category is 'illegal' (any single
 *      illegal verdict triggers immediate review), OR
 *   2. The author has accumulated ≥ RUNG4_REJECT_TRIGGER rejects in
 *      the last RUNG4_WINDOW_DAYS days.
 *
 * Dedup'd by an open-flag check on (target_type='user', target_id);
 * at most one open ban-candidate flag per user. The flag's
 * target_type='user' (added in migration 0019) means /admin/queue's
 * destructive actions point at the user under review, not at a
 * representative submission. The system user (policy-moderator) is
 * the reporter, distinguishing AI-flagged from user-reported content.
 *
 * The `targetType` / `targetId` arguments are the just-rejected
 * content's coordinates — kept for API symmetry with checkBanCandidate's
 * pre-0019 signature, but no longer used in the flag insert itself.
 */
export async function checkBanCandidate(
  authorId: string,
  verdict: ModerationVerdict,
  // Kept for backward-compat with callers in submissions/create.ts
  // and comments/create.ts; not read.
  _targetType: ModerationKind,
  _targetId: string,
): Promise<void> {
  if (verdict.verdict !== "reject") return;

  const isIllegal = verdict.category === "illegal";
  const rejects = await recentRejectsForAuthor(authorId, RUNG4_WINDOW_DAYS);
  if (!isIllegal && rejects < RUNG4_REJECT_TRIGGER) return;

  // Dedup: one open ban-candidate flag per user. Open flags are
  // resolved by staff via the existing /admin/queue dismiss path;
  // until they are, additional rejects don't multiply the flag.
  const [existing] = await db
    .select({ id: flags.id })
    .from(flags)
    .where(
      and(
        eq(flags.targetType, "user"),
        eq(flags.targetId, authorId),
        eq(flags.status, "open"),
        sql`${flags.reason} LIKE 'ban_candidate:%'`,
      ),
    )
    .limit(1);
  if (existing) return;

  const trigger = isIllegal ? "illegal" : `rejects_${rejects}`;
  const reason =
    `ban_candidate:trigger=${trigger}: ${verdict.oneLineWhy}`.slice(0, 500);

  const systemUserId = await getSystemUserId();
  await db.insert(flags).values({
    reporterId: systemUserId,
    targetType: "user",
    targetId: authorId,
    reason,
  });
}

// Re-export constants from ladder-config so callers reaching for the
// barrel (lib/moderation) get them at the same path the rest of the
// helpers come from.
export { LADDER_THRESHOLDS } from "./ladder-config";
