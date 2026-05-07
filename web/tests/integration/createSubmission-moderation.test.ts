/**
 * Integration test for createSubmission's moderator branches.
 *
 *   pnpm tsx --env-file=.env.local tests/integration/createSubmission-moderation.test.ts
 *
 * Requires TEST_DATABASE_URL pointing at a database with all
 * migrations applied (use a Neon preview branch or a local
 * Postgres). If unset, exits 0 with a warning — CI gates on the
 * harness running, but contributors can skip locally.
 *
 * Covers four branches the unit tests can't reach:
 *   1. Pass verdict — submission inserts as approved (or karma
 *      gate's pending), policy_decisions written, no moderation_log
 *      entry, no notification.
 *   2. Reject verdict — submission inserts as state='rejected', the
 *      ok:false {reason:'rejected', decisionId}-shaped result fires,
 *      moderation_log + notification written.
 *   3. Synthetic-error verdict — moderator failed; submission
 *      forced to state='pending' regardless of karma; no audit row
 *      (synthetic verdicts don't persist).
 *   4. Capped verdict — same failure-mode behavior as error,
 *      different syntheticReason.
 */

import assert from "node:assert/strict";
import { eq } from "drizzle-orm";

import {
  comments,
  moderationLog,
  notifications,
  policyDecisions,
  submissions,
  users,
} from "@/db/schema";
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
  console.log("SKIP createSubmission-moderation (no TEST_DATABASE_URL)");
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
    oneLineWhy: "Promotional link with no surrounding discussion.",
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

function syntheticCappedVerdict(): ModerationVerdict {
  return {
    verdict: "pass",
    category: null,
    confidence: "high",
    oneLineWhy: "daily moderation cap (50) reached for this author",
    synthetic: true,
    syntheticReason: "capped",
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: null,
    tags: [],
  };
}

async function freshAuthor(testDb: TestDb): Promise<{ id: string; username: string }> {
  // Karma 100 → state='approved' on pass. New users with karma 0 hit
  // the "pending" branch, which makes the pass-verdict assertions
  // depend on karma; using a high-karma user isolates the moderator
  // path.
  return seedUser(testDb, { karma: 100 });
}

(async () => {
  await ensurePolicyModeratorUser(db);

  // Test 1 — pass verdict
  await test("pass verdict → state='approved', policy_decisions row, no moderation_log, no notification", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    __setTestVerdictOverride(passVerdict());
    try {
      const result = await createSubmission(author.id, {
        type: "discussion",
        title: "Test pass-path",
        text: "A submission body that the moderator approves.",
      });
      assert.equal(result.ok, true);
      if (!result.ok) throw new Error("unreachable");
      assert.equal(result.pending, false);

      const [row] = await db
        .select({ state: submissions.state })
        .from(submissions)
        .where(eq(submissions.id, result.submissionId));
      assert.equal(row?.state, "approved");

      const [pd] = await db
        .select({ verdict: policyDecisions.verdict })
        .from(policyDecisions)
        .where(eq(policyDecisions.targetId, result.submissionId));
      assert.equal(pd?.verdict, "pass");

      const logRows = await db
        .select({ id: moderationLog.id })
        .from(moderationLog)
        .where(eq(moderationLog.targetId, result.submissionId));
      assert.equal(logRows.length, 0);

      const notifRows = await db
        .select({ id: notifications.id })
        .from(notifications)
        .where(eq(notifications.userId, author.id));
      assert.equal(notifRows.length, 0);
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Test 2 — reject verdict
  await test("reject verdict → ok:false reason='rejected', state='rejected', audit + notification", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    __setTestVerdictOverride(rejectSpamVerdict());
    try {
      const result = await createSubmission(author.id, {
        type: "discussion",
        title: "Test reject-path",
        text: "Buy followers cheap! www.spammy.example/promo",
      });
      assert.equal(result.ok, false);
      if (result.ok) throw new Error("unreachable");
      assert.equal(result.reason, "rejected");
      if (result.reason !== "rejected") throw new Error("unreachable");
      assert.equal(result.category, "spam");
      assert.ok(result.decisionId, "decisionId must be set on a real reject");

      const [row] = await db
        .select({ state: submissions.state })
        .from(submissions)
        .where(eq(submissions.id, result.submissionId));
      assert.equal(row?.state, "rejected");

      const [pd] = await db
        .select({ verdict: policyDecisions.verdict, category: policyDecisions.category })
        .from(policyDecisions)
        .where(eq(policyDecisions.id, result.decisionId!));
      assert.equal(pd?.verdict, "reject");
      assert.equal(pd?.category, "spam");

      // moderation_log: one row with action='reject' and the system user
      // as actor. Note carries category only (no PII spillage).
      const [systemUser] = await db
        .select({ id: users.id })
        .from(users)
        .where(eq(users.username, "policy-moderator"));
      assert.ok(systemUser, "policy-moderator system user must exist");
      const logRows = await db
        .select({
          id: moderationLog.id,
          action: moderationLog.action,
          staffId: moderationLog.staffId,
          note: moderationLog.note,
        })
        .from(moderationLog)
        .where(eq(moderationLog.targetId, result.submissionId));
      assert.equal(logRows.length, 1);
      assert.equal(logRows[0]?.action, "reject");
      assert.equal(logRows[0]?.staffId, systemUser.id);
      assert.equal(
        logRows[0]?.note,
        "spam",
        "log note should be category only — no verbatim oneLineWhy",
      );

      // Notification: one row with kind='moderation' for the author.
      const notifRows = await db
        .select({ kind: notifications.kind, payload: notifications.payload })
        .from(notifications)
        .where(eq(notifications.userId, author.id));
      assert.equal(notifRows.length, 1);
      assert.equal(notifRows[0]?.kind, "moderation");
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Test 3 — synthetic-error verdict
  await test("synthetic-error verdict → state='pending', no audit row, no notification", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    __setTestVerdictOverride(syntheticErrorVerdict());
    try {
      const result = await createSubmission(author.id, {
        type: "discussion",
        title: "Test fail-closed-path",
        text: "Body that would normally be approved.",
      });
      assert.equal(result.ok, true);
      if (!result.ok) throw new Error("unreachable");
      assert.equal(
        result.pending,
        true,
        "model error must force submission to state='pending' regardless of karma",
      );

      const [row] = await db
        .select({ state: submissions.state })
        .from(submissions)
        .where(eq(submissions.id, result.submissionId));
      assert.equal(row?.state, "pending");

      // Synthetic verdicts do not persist policy_decisions.
      const pdRows = await db
        .select({ id: policyDecisions.id })
        .from(policyDecisions)
        .where(eq(policyDecisions.targetId, result.submissionId));
      assert.equal(pdRows.length, 0);

      const logRows = await db
        .select({ id: moderationLog.id })
        .from(moderationLog)
        .where(eq(moderationLog.targetId, result.submissionId));
      assert.equal(logRows.length, 0);

      const notifRows = await db
        .select({ id: notifications.id })
        .from(notifications)
        .where(eq(notifications.userId, author.id));
      assert.equal(notifRows.length, 0);
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Test 4 — synthetic-capped verdict (per-author daily cap exceeded)
  await test("synthetic-capped verdict → state='pending' (same matrix as 'error')", async () => {
    await resetTables(db);
    const author = await freshAuthor(db);
    __setTestVerdictOverride(syntheticCappedVerdict());
    try {
      const result = await createSubmission(author.id, {
        type: "discussion",
        title: "Test cap-path",
        text: "Body submitted after the per-author daily cap.",
      });
      assert.equal(result.ok, true);
      if (!result.ok) throw new Error("unreachable");
      assert.equal(
        result.pending,
        true,
        "capped synthetic should fail-closed for submissions, same as error",
      );
      const [row] = await db
        .select({ state: submissions.state })
        .from(submissions)
        .where(eq(submissions.id, result.submissionId));
      assert.equal(row?.state, "pending");
      // Synthetic verdicts don't persist policy_decisions.
      const pdRows = await db
        .select({ id: policyDecisions.id })
        .from(policyDecisions)
        .where(eq(policyDecisions.targetId, result.submissionId));
      assert.equal(pdRows.length, 0);
    } finally {
      __setTestVerdictOverride(null);
    }
  });

  // Suppress unused-comment-import warning if the file is later edited
  // to reference comments. Keeping the import here documents the
  // tables the harness owns. resetTables() truncates them.
  void comments;

  console.log(`\n${passed} passed · ${failed} failed`);
  if (failed > 0) process.exit(1);
})();
