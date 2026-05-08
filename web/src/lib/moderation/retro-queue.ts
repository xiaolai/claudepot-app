/**
 * Retroactive moderation queue — comment fail-open backstop.
 *
 * When the moderator errors on a comment, the optimistic-publish
 * design says publish anyway and queue the comment for a retro
 * pass. This module owns enqueue + drain.
 *
 *   enqueueRetroComment() — called from createComment when the
 *     verdict is synthetic-due-to-error. Idempotent per (target,
 *     attempted_at): repeated retries are explicit history.
 *
 *   drainRetroQueue() — called by /api/cron/moderation-retro.
 *     Picks up to BATCH_SIZE 'pending' rows under SELECT … FOR
 *     UPDATE SKIP LOCKED so concurrent invocations don't double-
 *     process. Re-runs moderate() against the comment row; on
 *     reject, retracts via the same persist+notify+log path the
 *     pass-2 confirmation uses.
 *
 * Per-target retry policy: at most MAX_ATTEMPTS attempts before
 * the entry transitions to 'failed' and stops being picked up. A
 * staff-side surface to inspect failed entries is a follow-up; for
 * v0, /admin/log will show the resulting moderation_log row when
 * the retro pass succeeds + retracts.
 */

import { revalidatePath } from "next/cache";
import { and, desc, eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { comments, moderationRetroQueue, users } from "@/db/schema";
import { checkBanCandidate } from "./ladder";
import { moderate } from "./index";
import { writeModerationLogForReject } from "./persist";
import { writeModerationNotification } from "./notify";
import { writePolicyDecision } from "./persist";
import type { ModerationAuthor } from "./types";

const MAX_ATTEMPTS = 3;
const BATCH_SIZE = 25;

export interface EnqueueRetroParams {
  targetType: "submission" | "comment";
  targetId: string;
  authorId: string;
  triggerReason: string;
}

export async function enqueueRetroComment(
  params: EnqueueRetroParams,
): Promise<void> {
  await db.insert(moderationRetroQueue).values({
    targetType: params.targetType,
    targetId: params.targetId,
    authorId: params.authorId,
    triggerReason: params.triggerReason.slice(0, 500),
  });
}

export interface DrainResult {
  picked: number;
  succeeded: number;
  retracted: number;
  failed: number;
}

/**
 * Drain a batch from the retro queue. Idempotent across concurrent
 * cron invocations via SELECT … FOR UPDATE SKIP LOCKED.
 *
 * Each pending row:
 *   1. Lock + transition to 'in_progress'.
 *   2. Re-fetch the comment by target_id; skip if it's been
 *      deleted or already rejected (no work to do).
 *   3. Re-run moderate() with the comment's body. The author
 *      record is required for the moderate() call's exempt check.
 *   4. On real reject: flip state to 'rejected', write pass=3
 *      policy_decisions row, moderation_log + notification.
 *   5. Mark the queue entry 'done' (regardless of pass/reject).
 *   6. On error: increment attempts; mark 'failed' if MAX_ATTEMPTS
 *      reached, else leave 'pending' for the next cron tick.
 */
export async function drainRetroQueue(): Promise<DrainResult> {
  const result: DrainResult = {
    picked: 0,
    succeeded: 0,
    retracted: 0,
    failed: 0,
  };

  // Step 1: pick + lock a batch.
  // FOR UPDATE SKIP LOCKED on a single SELECT is the canonical
  // queue-leasing pattern in Postgres. Drizzle exposes it via raw
  // sql; the inner SELECT is wrapped in a CTE-update so we can
  // both lock and transition state in one round-trip.
  const picked = await db.execute<{
    id: string;
    target_type: "submission" | "comment" | "user";
    target_id: string;
    author_id: string;
    trigger_reason: string;
    attempts: number;
  }>(sql`
    WITH leased AS (
      SELECT id
        FROM moderation_retro_queue
       WHERE state = 'pending'
       ORDER BY enqueued_at ASC
       LIMIT ${BATCH_SIZE}
       FOR UPDATE SKIP LOCKED
    )
    UPDATE moderation_retro_queue q
       SET state = 'in_progress', started_at = now()
      FROM leased
     WHERE q.id = leased.id
     RETURNING q.id, q.target_type, q.target_id, q.author_id,
               q.trigger_reason, q.attempts
  `);
  const rows = picked.rows;
  result.picked = rows.length;

  for (const row of rows) {
    if (row.target_type !== "comment") {
      // Submissions are fail-closed (state='pending') — they don't
      // enter the retro queue. Defensive only: mark done.
      await db
        .update(moderationRetroQueue)
        .set({ state: "done", completedAt: new Date() })
        .where(eq(moderationRetroQueue.id, row.id));
      continue;
    }

    try {
      const [c] = await db
        .select({
          id: comments.id,
          body: comments.body,
          state: comments.state,
          deletedAt: comments.deletedAt,
          submissionId: comments.submissionId,
        })
        .from(comments)
        .where(eq(comments.id, row.target_id))
        .limit(1);

      // No work if the comment is gone or already rejected.
      if (!c || c.deletedAt || c.state === "rejected") {
        await db
          .update(moderationRetroQueue)
          .set({ state: "done", completedAt: new Date() })
          .where(eq(moderationRetroQueue.id, row.id));
        result.succeeded += 1;
        continue;
      }

      const [u] = await db
        .select({
          role: users.role,
          isAgent: users.isAgent,
          botModerationExempt: users.botModerationExempt,
        })
        .from(users)
        .where(eq(users.id, row.author_id))
        .limit(1);
      if (!u) {
        await db
          .update(moderationRetroQueue)
          .set({
            state: "failed",
            completedAt: new Date(),
            lastError: "author not found",
          })
          .where(eq(moderationRetroQueue.id, row.id));
        result.failed += 1;
        continue;
      }

      const author: ModerationAuthor = {
        id: row.author_id,
        role: u.role,
        isAgent: u.isAgent,
        botModerationExempt: u.botModerationExempt,
      };
      const verdict = await moderate(
        { kind: "comment", title: "", body: c.body },
        author,
      );

      // Synthetic verdicts: branch on syntheticReason.
      //   - 'disabled' / 'exempt' / 'capped': retrying produces the
      //     same outcome (moderator configured off, author allow-
      //     listed, or cap hit). Mark 'done' so the queue doesn't
      //     poison the row by cycling until MAX_ATTEMPTS. The
      //     optimistic publish stands.
      //   - 'error': transient model failure. Retry up to
      //     MAX_ATTEMPTS, then transition to 'failed' so a persistent
      //     outage doesn't accumulate pending work indefinitely.
      if (verdict.synthetic) {
        const isTransientError = verdict.syntheticReason === "error";
        if (!isTransientError) {
          await db
            .update(moderationRetroQueue)
            .set({
              state: "done",
              completedAt: new Date(),
              lastError: verdict.oneLineWhy,
            })
            .where(eq(moderationRetroQueue.id, row.id));
          result.succeeded += 1;
          continue;
        }
        const nextAttempts = row.attempts + 1;
        const terminal = nextAttempts >= MAX_ATTEMPTS;
        await db
          .update(moderationRetroQueue)
          .set({
            state: terminal ? "failed" : "pending",
            attempts: nextAttempts,
            startedAt: null,
            lastError: terminal ? verdict.oneLineWhy : null,
            completedAt: terminal ? new Date() : null,
          })
          .where(eq(moderationRetroQueue.id, row.id));
        if (terminal) result.failed += 1;
        continue;
      }

      // Real verdict — record it and, on reject, retract.
      const decisionId = await writePolicyDecision({
        authorId: row.author_id,
        targetType: "comment",
        targetId: c.id,
        verdict,
        // pass=3 distinguishes retro-queue retries from pass-2
        // confirmation passes for calibration analysis.
        passNumber: 3,
      });

      if (verdict.verdict === "reject" && verdict.category) {
        await db
          .update(comments)
          .set({ state: "rejected" })
          .where(eq(comments.id, c.id));
        await writeModerationLogForReject({
          targetType: "comment",
          targetId: c.id,
          category: verdict.category,
          oneLineWhy: verdict.oneLineWhy,
          passNumber: 3,
        });
        await writeModerationNotification({
          recipientId: row.author_id,
          targetType: "comment",
          targetId: c.id,
          targetTitle: null,
          category: verdict.category,
          oneLineWhy: verdict.oneLineWhy,
          decisionId,
        });
        await checkBanCandidate(row.author_id, verdict, "comment", c.id);
        revalidatePath(`/post/${c.submissionId}`);
        result.retracted += 1;
      } else {
        result.succeeded += 1;
      }

      await db
        .update(moderationRetroQueue)
        .set({ state: "done", completedAt: new Date() })
        .where(eq(moderationRetroQueue.id, row.id));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      const nextAttempts = row.attempts + 1;
      const terminal = nextAttempts >= MAX_ATTEMPTS;
      await db
        .update(moderationRetroQueue)
        .set({
          state: terminal ? "failed" : "pending",
          attempts: nextAttempts,
          startedAt: null,
          lastError: msg.slice(0, 500),
          completedAt: terminal ? new Date() : null,
        })
        .where(eq(moderationRetroQueue.id, row.id));
      if (terminal) result.failed += 1;
      console.warn(
        `[moderation/retro-queue] entry ${row.id} attempt ${nextAttempts} failed: ${msg}`,
      );
    }
  }

  return result;
}

/** Test-only / debug-only helper. Production code should not call this. */
export async function listRetroQueueForTarget(
  targetType: "submission" | "comment",
  targetId: string,
): Promise<unknown[]> {
  return db
    .select()
    .from(moderationRetroQueue)
    .where(
      and(
        eq(moderationRetroQueue.targetType, targetType),
        eq(moderationRetroQueue.targetId, targetId),
      ),
    )
    .orderBy(desc(moderationRetroQueue.enqueuedAt));
}
