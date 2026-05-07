/**
 * Integration test for createComment's moderator branches.
 *
 *   pnpm tsx --env-file=.env.local tests/integration/createComment-moderation.test.ts
 *
 * Requires TEST_DATABASE_URL with all migrations applied. Skips
 * (exits 0) when unset.
 *
 * Covers four branches:
 *   1. Pass verdict — comment inserts as approved, policy_decisions
 *      pass-1 row written.
 *   2. Non-illegal reject — comment inserts as 'approved' (optimistic
 *      publish), policy_decisions pass-1 row written. The pass-2
 *      retract path is exercised in runCommentConfirmation tests.
 *   3. Illegal verdict — NO comment row inserted, ok:false
 *      reason='illegal', policy_decisions written with target_id=NULL,
 *      ban_candidate flag inserted (target_type='user'), moderation_log
 *      row written against parent submission, notification written.
 *   4. Synthetic-error verdict — comment inserts as 'approved'
 *      (fail-OPEN per plan §11), retro_queue entry inserted.
 */

import assert from "node:assert/strict";
import { eq } from "drizzle-orm";

import {
  comments,
  flags,
  moderationLog,
  moderationRetroQueue,
  notifications,
  policyDecisions,
  users,
} from "@/db/schema";
import { createComment } from "@/lib/comments";
import { createSubmission } from "@/lib/submissions";
import {
  __setTestVerdictOverride,
  POLICY_MODEL,
  POLICY_PROMPT_V,
  type ModerationVerdict,
} from "@/lib/moderation";
import {
  ensurePolicyModeratorUser,
  requireHarness,
  resetTables,
  seedUser,
  type TestDb,
} from "./db";

const db = requireHarness();
if (!db) {
  console.log("SKIP createComment-moderation (no TEST_DATABASE_URL)");
  process.exit(0);
}

let passed = 0;
let failed = 0;

function test(name: string, fn: () => Promise<void>) {
  return (async () => {
    try {
      await fn();
      console.log(`PASS  ${name}`);
      passed += 1;
    } catch (err) {
      console.error(`FAIL  ${name}`);
      console.error(`      ${err instanceof Error ? err.stack : String(err)}`);
      failed += 1;
    }
  })();
}

function passVerdict(): ModerationVerdict {
  return {
    verdict: "pass",
    category: null,
    confidence: "high",
    oneLineWhy: "looks fine",
    synthetic: false,
    syntheticReason: null,
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: 0.0001,
    tags: [],
  };
}

function rejectSpamVerdict(): ModerationVerdict {
  return {
    verdict: "reject",
    category: "spam",
    confidence: "high",
    oneLineWhy: "Promotional copy with no surrounding context.",
    synthetic: false,
    syntheticReason: null,
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: 0.0001,
    tags: [],
  };
}

function illegalVerdict(): ModerationVerdict {
  return {
    verdict: "reject",
    category: "illegal",
    confidence: "high",
    oneLineWhy: "Distribution of stolen credentials.",
    synthetic: false,
    syntheticReason: null,
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: 0.0001,
    tags: [],
  };
}

function syntheticErrorVerdict(): ModerationVerdict {
  return {
    verdict: "pass",
    category: null,
    confidence: "high",
    oneLineWhy: "moderator unavailable: fake test error",
    synthetic: true,
    syntheticReason: "error",
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: null,
    tags: [],
  };
}

async function freshAuthor(testDb: TestDb): Promise<{ id: string; username: string }> {
  return seedUser(testDb, { karma: 100 });
}

/**
 * Seeds an approved submission so comments have a parent to attach
 * to. Bypasses the moderator (override returns pass) so the
 * submission insert is a clean fixture.
 */
async function seedSubmission(
  testDb: TestDb,
  authorId: string,
): Promise<string> {
  __setTestVerdictOverride(passVerdict());
  try {
    const r = await createSubmission(authorId, {
      type: "discussion",
      title: "Parent thread",
      text: "A submission for the comment tests.",
    });
    if (!r.ok) {
      throw new Error(
        `failed to seed submission: ${"reason" in r ? r.reason : "unknown"}`,
      );
    }
    // Truncate the seed's audit side-effects so per-test assertions
    // start clean (the test wants to count rows produced by THIS
    // call, not the fixture).
    await testDb
      .delete(policyDecisions)
      .where(eq(policyDecisions.targetId, r.submissionId));
    return r.submissionId;
  } finally {
    __setTestVerdictOverride(null);
  }
}

(async () => {
  await ensurePolicyModeratorUser(db);

  // Test 1 — pass verdict
  await test("pass verdict → state='approved', pass-1 policy_decisions row", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    const submissionId = await seedSubmission(db, author.id);

    __setTestVerdictOverride(passVerdict());
    try {
      const result = await createComment(author.id, {
        submissionId,
        body: "Nice writeup, thanks for sharing.",
      });
      assert.equal(result.ok, true);
      if (!result.ok) throw new Error("unreachable");
      assert.equal(result.pending, false);

      const [row] = await db
        .select({ state: comments.state })
        .from(comments)
        .where(eq(comments.id, result.commentId));
      assert.equal(row?.state, "approved");

      const [pd] = await db
        .select({
          verdict: policyDecisions.verdict,
          passNumber: policyDecisions.passNumber,
        })
        .from(policyDecisions)
        .where(eq(policyDecisions.targetId, result.commentId));
      assert.equal(pd?.verdict, "pass");
      assert.equal(pd?.passNumber, 1);
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Test 2 — non-illegal reject (optimistic publish)
  await test("non-illegal reject → state='approved' (optimistic publish), pass-1 row", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    const submissionId = await seedSubmission(db, author.id);

    __setTestVerdictOverride(rejectSpamVerdict());
    try {
      const result = await createComment(author.id, {
        submissionId,
        body: "Buy followers cheap! www.spammy.example/promo",
      });
      assert.equal(result.ok, true);
      if (!result.ok) throw new Error("unreachable");

      const [row] = await db
        .select({ state: comments.state })
        .from(comments)
        .where(eq(comments.id, result.commentId));
      assert.equal(
        row?.state,
        "approved",
        "non-illegal reject must publish optimistically — pass-2 confirm retracts later",
      );

      const [pd] = await db
        .select({
          verdict: policyDecisions.verdict,
          category: policyDecisions.category,
        })
        .from(policyDecisions)
        .where(eq(policyDecisions.targetId, result.commentId));
      assert.equal(pd?.verdict, "reject");
      assert.equal(pd?.category, "spam");

      // No moderation_log / notification YET — those fire only when
      // the confirmation pass also rejects (runCommentConfirmation).
      const logRows = await db
        .select({ id: moderationLog.id })
        .from(moderationLog)
        .where(eq(moderationLog.targetId, result.commentId));
      assert.equal(logRows.length, 0);
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Test 3 — illegal verdict (hard block)
  await test("illegal verdict → no comment row, target_id=null on policy_decisions, ban_candidate user flag, moderation_log on parent, notification", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    const submissionId = await seedSubmission(db, author.id);

    __setTestVerdictOverride(illegalVerdict());
    try {
      const result = await createComment(author.id, {
        submissionId,
        body: "Here are some stolen credentials: …",
      });
      assert.equal(result.ok, false);
      if (result.ok) throw new Error("unreachable");
      assert.equal(result.reason, "illegal");

      // No comment row was inserted on illegal block.
      const commentRows = await db
        .select({ id: comments.id })
        .from(comments)
        .where(eq(comments.authorId, author.id));
      assert.equal(commentRows.length, 0);

      // policy_decisions has the row with target_id=NULL.
      const pdRows = await db
        .select({
          targetType: policyDecisions.targetType,
          targetId: policyDecisions.targetId,
          category: policyDecisions.category,
        })
        .from(policyDecisions)
        .where(eq(policyDecisions.authorId, author.id));
      assert.equal(pdRows.length, 1);
      assert.equal(pdRows[0]?.targetType, "comment");
      assert.equal(pdRows[0]?.targetId, null);
      assert.equal(pdRows[0]?.category, "illegal");

      // Ban-candidate flag with target_type='user'.
      const flagRows = await db
        .select({
          targetType: flags.targetType,
          targetId: flags.targetId,
          reason: flags.reason,
        })
        .from(flags)
        .where(eq(flags.targetId, author.id));
      assert.equal(flagRows.length, 1);
      assert.equal(flagRows[0]?.targetType, "user");
      assert.ok(flagRows[0]?.reason.startsWith("ban_candidate:"));

      // moderation_log against parent submission, note='illegal'
      // (no PII spillage).
      const [systemUser] = await db
        .select({ id: users.id })
        .from(users)
        .where(eq(users.username, "policy-moderator"));
      assert.ok(systemUser);
      const logRows = await db
        .select({
          targetType: moderationLog.targetType,
          targetId: moderationLog.targetId,
          note: moderationLog.note,
        })
        .from(moderationLog)
        .where(eq(moderationLog.staffId, systemUser.id));
      assert.equal(logRows.length, 1);
      assert.equal(logRows[0]?.targetType, "submission");
      assert.equal(logRows[0]?.targetId, submissionId);
      assert.equal(logRows[0]?.note, "illegal");

      // Notification with appeal_url=null (no row to appeal against).
      const notifRows = await db
        .select({ kind: notifications.kind, payload: notifications.payload })
        .from(notifications)
        .where(eq(notifications.userId, author.id));
      assert.equal(notifRows.length, 1);
      const payload = notifRows[0]?.payload as Record<string, unknown>;
      assert.equal(payload?.appeal_url, null);
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Test 4 — synthetic-error verdict (fail-open + retro enqueue)
  await test("synthetic-error verdict → state='approved' (fail-open), retro_queue entry inserted", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    const submissionId = await seedSubmission(db, author.id);

    __setTestVerdictOverride(syntheticErrorVerdict());
    try {
      const result = await createComment(author.id, {
        submissionId,
        body: "Comment that publishes optimistically on model failure.",
      });
      assert.equal(result.ok, true);
      if (!result.ok) throw new Error("unreachable");

      const [row] = await db
        .select({ state: comments.state })
        .from(comments)
        .where(eq(comments.id, result.commentId));
      assert.equal(
        row?.state,
        "approved",
        "comment fail-OPEN per plan §11 — publish, queue retroactive",
      );

      // No policy_decisions row — synthetic verdicts don't persist.
      const pdRows = await db
        .select({ id: policyDecisions.id })
        .from(policyDecisions)
        .where(eq(policyDecisions.targetId, result.commentId));
      assert.equal(pdRows.length, 0);

      // Retro-queue entry waiting for the cron to pick up.
      const queueRows = await db
        .select({
          targetType: moderationRetroQueue.targetType,
          targetId: moderationRetroQueue.targetId,
          state: moderationRetroQueue.state,
        })
        .from(moderationRetroQueue)
        .where(eq(moderationRetroQueue.targetId, result.commentId));
      assert.equal(queueRows.length, 1);
      assert.equal(queueRows[0]?.targetType, "comment");
      assert.equal(queueRows[0]?.state, "pending");
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  console.log(`\n${passed} passed · ${failed} failed`);
  if (failed > 0) process.exit(1);
})();
