/**
 * Tests for src/lib/editorial-writes/schemas.ts.
 *
 *   pnpm tsx tests/editorial-writes-schemas.test.ts
 *
 * Schema-level coverage: zod input validators must accept the
 * shapes documented in dev-docs/2026-05-08-polity-api-asks.md and
 * reject malformed payloads. Persistence (persistDecision /
 * persistOverride / persistScoutRun) is integration-level and
 * needs a Neon DB to exercise — not covered here.
 */

import assert from "node:assert/strict";
// Import directly from the schemas module so the test doesn't pull
// persist.ts → @/db/client (which throws at module load if
// DATABASE_URL is unset). Schema tests are pure and shouldn't need
// a DB; integration coverage for the persistence path lives
// elsewhere.
import {
  decisionInputSchema,
  overrideInputSchema,
  scoutRunInputSchema,
} from "../src/lib/editorial-writes/schemas";

let passed = 0;
let failed = 0;
function test(name: string, fn: () => void) {
  try {
    fn();
    console.log(`PASS  ${name}`);
    passed += 1;
  } catch (err) {
    console.error(`FAIL  ${name}`);
    console.error(`      ${err instanceof Error ? err.message : String(err)}`);
    failed += 1;
  }
}

// v4 UUID — third group starts with 4 (version), fourth with 8/9/a/b
// (variant). The looser /[0-9a-f]{8}-…/ shape that older zod
// accepted is now rejected by zod v4's uuid() RFC-bit check.
const FAKE_UUID = "11111111-2222-4333-8444-555555555555";

/* ── Decision input ─────────────────────────────────────────── */

test("decisionInputSchema accepts the office's documented shape", () => {
  const result = decisionInputSchema.safeParse({
    submissionId: FAKE_UUID,
    rubricVersion: "0.2.3",
    audienceDocVersion: "0.1.2",
    appliedPersona: "ada",
    perCriterionScores: { mechanism_specificity: 5, evidence_quality: 4 },
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
    promptHash: "sha256:deadbeef",
    costUsd: 0.0123,
  });
  assert.equal(result.success, true);
});

test("decisionInputSchema accepts an open persona/criterion vocabulary", () => {
  const result = decisionInputSchema.safeParse({
    submissionId: FAKE_UUID,
    rubricVersion: "0.3.0",
    audienceDocVersion: "0.1.2",
    appliedPersona: "future_persona_not_yet_in_polity",
    perCriterionScores: {
      brand_new_criterion: 4,
      another_one: -2,
    },
    weightedTotal: 12,
    hardRejectsHit: [],
    inclusionGates: {},
    typeInferred: "release",
    subSegmentInferred: "platform-engineers",
    confidence: "low",
    oneLineWhy: "Open vocabulary verified.",
    finalDecision: "borderline_to_human_queue",
    routing: "human_queue",
    modelId: "anthropic.claude-opus-4-7",
  });
  assert.equal(result.success, true);
});

test("decisionInputSchema rejects unknown finalDecision", () => {
  const result = decisionInputSchema.safeParse({
    submissionId: FAKE_UUID,
    rubricVersion: "0.2.3",
    audienceDocVersion: "0.1.2",
    appliedPersona: "ada",
    perCriterionScores: {},
    weightedTotal: 0,
    inclusionGates: {},
    typeInferred: "tutorial",
    subSegmentInferred: "engineers",
    confidence: "high",
    oneLineWhy: "n/a",
    finalDecision: "publish_immediately", // not in the closed enum
    routing: "feed",
    modelId: "claude-opus-4-7",
  });
  assert.equal(result.success, false);
});

test("decisionInputSchema rejects non-UUID submissionId", () => {
  const result = decisionInputSchema.safeParse({
    submissionId: "not-a-uuid",
    rubricVersion: "0.2.3",
    audienceDocVersion: "0.1.2",
    appliedPersona: "ada",
    perCriterionScores: {},
    weightedTotal: 0,
    inclusionGates: {},
    typeInferred: "tutorial",
    subSegmentInferred: "engineers",
    confidence: "high",
    oneLineWhy: "n/a",
    finalDecision: "accept",
    routing: "feed",
    modelId: "claude-opus-4-7",
  });
  assert.equal(result.success, false);
});

/* ── Override input ─────────────────────────────────────────── */

test("overrideInputSchema accepts the documented shape", () => {
  const result = overrideInputSchema.safeParse({
    overrideDecision: "borderline_to_human_queue",
    overrideRouting: "human_queue",
    reason: "Source quality looks weaker on second read.",
  });
  assert.equal(result.success, true);
});

test("overrideInputSchema requires reason", () => {
  const result = overrideInputSchema.safeParse({
    overrideDecision: "accept",
    overrideRouting: "feed",
  });
  assert.equal(result.success, false);
});

/* ── Scout-run input ────────────────────────────────────────── */

test("scoutRunInputSchema accepts the documented shape", () => {
  const result = scoutRunInputSchema.safeParse({
    sourceId: "hn-frontpage",
    startedAt: "2026-05-08T01:00:00Z",
    finishedAt: "2026-05-08T01:00:42Z",
    itemsPulled: 30,
    itemsKept: 4,
    itemsDropped: 26,
  });
  assert.equal(result.success, true);
});

test("scoutRunInputSchema rejects finishedAt before startedAt", () => {
  const result = scoutRunInputSchema.safeParse({
    sourceId: "hn-frontpage",
    startedAt: "2026-05-08T01:00:42Z",
    finishedAt: "2026-05-08T01:00:00Z",
    itemsPulled: 30,
    itemsKept: 4,
    itemsDropped: 26,
  });
  assert.equal(result.success, false);
});

test("scoutRunInputSchema accepts finishedAt === startedAt (zero-duration scout)", () => {
  // Boundary: the refine uses >=, so equal timestamps are valid.
  // A scout that pulled nothing in zero duration is unusual but
  // legal — guard against an off-by-one regression that would
  // make the >= a >.
  const result = scoutRunInputSchema.safeParse({
    sourceId: "hn-frontpage",
    startedAt: "2026-05-08T01:00:00Z",
    finishedAt: "2026-05-08T01:00:00Z",
    itemsPulled: 0,
    itemsKept: 0,
    itemsDropped: 0,
  });
  assert.equal(result.success, true);
});

test("scoutRunInputSchema rejects itemsKept + itemsDropped > itemsPulled", () => {
  const result = scoutRunInputSchema.safeParse({
    sourceId: "hn-frontpage",
    startedAt: "2026-05-08T01:00:00Z",
    finishedAt: "2026-05-08T01:00:42Z",
    itemsPulled: 5,
    itemsKept: 4,
    itemsDropped: 26,
  });
  assert.equal(result.success, false);
});

test("scoutRunInputSchema accepts itemsKept + itemsDropped === itemsPulled (full classification)", () => {
  // Boundary: exact equality is the common case (every pulled item
  // is either kept or dropped). The refine uses <=, so this must
  // pass; off-by-one to < would lock out 100% of normal scout runs.
  const result = scoutRunInputSchema.safeParse({
    sourceId: "hn-frontpage",
    startedAt: "2026-05-08T01:00:00Z",
    finishedAt: "2026-05-08T01:00:42Z",
    itemsPulled: 30,
    itemsKept: 12,
    itemsDropped: 18,
  });
  assert.equal(result.success, true);
});

/* ── Decision input — promptHash optionality ─────────────────── */

test("decisionInputSchema accepts a missing promptHash (NULL idempotency key)", () => {
  // The idempotency unique on decision_records uses
  // COALESCE(prompt_hash, '') so two NULL retries collide. The
  // schema must accept the missing field for that path to fire.
  // Integration coverage of the actual NULL-vs-NULL conflict needs
  // a DB; this test guards the schema contract upstream of it.
  const result = decisionInputSchema.safeParse({
    submissionId: FAKE_UUID,
    rubricVersion: "0.2.3",
    audienceDocVersion: "0.1.2",
    appliedPersona: "ada",
    perCriterionScores: {},
    weightedTotal: 0,
    inclusionGates: {},
    typeInferred: "tutorial",
    subSegmentInferred: "engineers",
    confidence: "high",
    oneLineWhy: "no prompt hash provided",
    finalDecision: "accept",
    routing: "feed",
    modelId: "claude-opus-4-7",
    // promptHash deliberately omitted
  });
  assert.equal(result.success, true);
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
