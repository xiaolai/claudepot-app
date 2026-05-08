/**
 * Integration test for the editorial-writes persistence layer +
 * the publish primitive.
 *
 *   pnpm tsx --env-file=.env.local tests/integration/editorial-writes.test.ts
 *
 * Requires both DATABASE_URL (runtime db client) and
 * TEST_DATABASE_URL (harness reset/seed) pointing at the SAME
 * test database with all migrations through 0036 applied.
 *
 * Covers what the schema-level tests can't:
 *   - Idempotency on (submissionId, appliedPersona, modelId, NULL prompt_hash):
 *     two retries with NULL prompt_hash collide; second returns the
 *     existing decisionId with created=false.
 *   - persistDecision / persistOverride NEVER touch submissions.state.
 *     The polity stops conflating "the office decided" with "the
 *     office decided to publish."
 *   - publishSubmission(true) flips draft→approved.
 *   - publishSubmission(false) flips approved→draft.
 *   - Publish is idempotent (re-call → outcome='unchanged').
 *   - Publish is refused on citizen-authored submissions
 *     (not_office_owned).
 *
 * Cleanup truncates editorial-runtime tables explicitly because the
 * existing resetTables() helper doesn't know about them.
 */

import assert from "node:assert/strict";
import { eq } from "drizzle-orm";

import {
  decisionRecords,
  overrideRecords,
  submissions,
} from "@/db/schema";
import {
  persistDecision,
  persistOverride,
  type DecisionInput,
} from "@/lib/editorial-writes";
import { publishSubmission } from "@/lib/submissions";

import {
  ensurePolicyModeratorUser,
  requireHarness,
  resetTables,
  seedUser,
  type TestDb,
} from "./db";

const db = requireHarness();
if (!db) {
  console.log("SKIP editorial-writes (no TEST_DATABASE_URL)");
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

async function resetEditorial(d: TestDb): Promise<void> {
  // FK order: override_records → decision_records → submissions.
  await d.execute(/* sql */ `DELETE FROM "override_records"`);
  await d.execute(/* sql */ `DELETE FROM "decision_records"`);
  await resetTables(d);
}

async function seedDraftSubmission(authorId: string): Promise<string> {
  const [row] = await db!
    .insert(submissions)
    .values({
      authorId,
      type: "tutorial",
      title: "Draft fixture for editorial-writes",
      url: "https://example.test/draft-fixture",
      text: null,
      state: "draft",
      submitterKind: "scout",
      sourceId: null,
    })
    .returning({ id: submissions.id });
  return row.id;
}

function decisionInputFor(submissionId: string): DecisionInput {
  return {
    submissionId,
    rubricVersion: "0.2.3",
    audienceDocVersion: "0.1.2",
    appliedPersona: "ada",
    perCriterionScores: { mechanism_specificity: 5 },
    weightedTotal: 47.5,
    hardRejectsHit: [],
    inclusionGates: { primary_source_identifiable: true },
    typeInferred: "tutorial",
    subSegmentInferred: "engineers",
    confidence: "high",
    oneLineWhy: "Mechanism is specific and reproducible.",
    finalDecision: "accept",
    routing: "feed",
    modelId: "claude-opus-4-7",
    // promptHash deliberately omitted — exercises NULL-collide path.
  };
}

async function main() {
  await test("two NULL-prompt-hash retries collide via idempotency unique", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const author = await seedUser(db!, { isAgent: true, role: "system" });
    const submissionId = await seedDraftSubmission(author.id);

    const first = await persistDecision(decisionInputFor(submissionId));
    assert.equal(first.ok, true);
    if (!first.ok) return;
    assert.equal(first.created, true);
    const firstId = first.decisionId;

    const second = await persistDecision(decisionInputFor(submissionId));
    assert.equal(second.ok, true);
    if (!second.ok) return;
    assert.equal(second.created, false);
    assert.equal(second.decisionId, firstId);

    const rows = await db!
      .select({ id: decisionRecords.id })
      .from(decisionRecords)
      .where(eq(decisionRecords.submissionId, submissionId));
    assert.equal(rows.length, 1);
  });

  await test("persistDecision never touches submissions.state", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const author = await seedUser(db!, { isAgent: true, role: "system" });
    const submissionId = await seedDraftSubmission(author.id);

    // Even with routing='feed' AND finalDecision='accept', the
    // submission stays draft. Publishing is the office's job.
    const result = await persistDecision(decisionInputFor(submissionId));
    assert.equal(result.ok, true);

    const [sub] = await db!
      .select({
        state: submissions.state,
        publishedAt: submissions.publishedAt,
      })
      .from(submissions)
      .where(eq(submissions.id, submissionId))
      .limit(1);
    assert.equal(sub.state, "draft");
    assert.equal(sub.publishedAt, null);
  });

  await test("persistOverride never touches submissions.state", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const author = await seedUser(db!, { isAgent: true, role: "system" });
    const reviewer = await seedUser(db!, { isAgent: true, role: "system" });
    const submissionId = await seedDraftSubmission(author.id);

    const initial = await persistDecision({
      ...decisionInputFor(submissionId),
      routing: "firehose",
      finalDecision: "reject",
    });
    assert.equal(initial.ok, true);
    if (!initial.ok) return;

    const override = await persistOverride(initial.decisionId, reviewer.id, {
      overrideDecision: "accept",
      overrideRouting: "feed",
      reason: "Re-read; mechanism is solid.",
    });
    assert.equal(override.ok, true);

    const [sub] = await db!
      .select({ state: submissions.state })
      .from(submissions)
      .where(eq(submissions.id, submissionId))
      .limit(1);
    assert.equal(sub.state, "draft");

    const overrides = await db!
      .select({ reviewerKind: overrideRecords.reviewerKind })
      .from(overrideRecords)
      .where(eq(overrideRecords.decisionRecordId, initial.decisionId));
    assert.equal(overrides.length, 1);
    assert.equal(overrides[0].reviewerKind, "bot");
  });

  await test("publishSubmission(true) flips draft→approved and sets publishedAt", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const author = await seedUser(db!, { isAgent: true, role: "system" });
    const submissionId = await seedDraftSubmission(author.id);

    const result = await publishSubmission(submissionId, true);
    assert.equal(result.ok, true);
    if (!result.ok) return;
    assert.equal(result.outcome, "published");
    assert.equal(result.state, "approved");

    const [sub] = await db!
      .select({
        state: submissions.state,
        publishedAt: submissions.publishedAt,
      })
      .from(submissions)
      .where(eq(submissions.id, submissionId))
      .limit(1);
    assert.equal(sub.state, "approved");
    assert.notEqual(sub.publishedAt, null);
  });

  await test("publishSubmission(false) flips approved→draft and clears publishedAt", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const author = await seedUser(db!, { isAgent: true, role: "system" });
    const submissionId = await seedDraftSubmission(author.id);

    await publishSubmission(submissionId, true);
    const result = await publishSubmission(submissionId, false);
    assert.equal(result.ok, true);
    if (!result.ok) return;
    assert.equal(result.outcome, "unpublished");
    assert.equal(result.state, "draft");

    const [sub] = await db!
      .select({
        state: submissions.state,
        publishedAt: submissions.publishedAt,
      })
      .from(submissions)
      .where(eq(submissions.id, submissionId))
      .limit(1);
    assert.equal(sub.state, "draft");
    assert.equal(sub.publishedAt, null);
  });

  await test("publishSubmission is idempotent (re-publish = unchanged)", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const author = await seedUser(db!, { isAgent: true, role: "system" });
    const submissionId = await seedDraftSubmission(author.id);

    await publishSubmission(submissionId, true);
    const second = await publishSubmission(submissionId, true);
    assert.equal(second.ok, true);
    if (!second.ok) return;
    assert.equal(second.outcome, "unchanged");
    assert.equal(second.state, "approved");
  });

  await test("publishSubmission refuses citizen-authored submissions", async () => {
    await resetEditorial(db!);
    await ensurePolicyModeratorUser(db!);
    const citizen = await seedUser(db!, { isAgent: false, role: "user" });

    // Insert a citizen submission directly (bypasses createSubmission's
    // moderator path — we want a row with author.is_agent=false to
    // attack the publish primitive's gate).
    const [row] = await db!
      .insert(submissions)
      .values({
        authorId: citizen.id,
        type: "discussion",
        title: "Citizen post",
        url: null,
        text: "Hello",
        state: "approved",
      })
      .returning({ id: submissions.id });

    const result = await publishSubmission(row.id, false);
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.equal(result.reason, "not_office_owned");
  });

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

void main();
