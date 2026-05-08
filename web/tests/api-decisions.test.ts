/**
 * Tests for src/lib/api/decisions.ts (pure builder).
 *
 *   pnpm tsx tests/api-decisions.test.ts
 *
 * The privacy contract is the load-bearing piece — the public DTO must
 * NEVER carry per-criterion scores, weighted totals, prompt hashes, or
 * cost fields, even by accident. Tested by introspecting the keys of
 * the produced object, not just by happy-path round-tripping.
 */

import assert from "node:assert/strict";
import { buildDecisionDto } from "../src/lib/api/decision-dto";
import type { decisionRecords, overrideRecords } from "../src/db/schema";

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

const FAKE_UUID = "11111111-2222-3333-4444-555555555555";
const FAKE_DECISION_ID = "22222222-3333-4444-5555-666666666666";

function fixtureRow(): typeof decisionRecords.$inferSelect {
  return {
    id: FAKE_DECISION_ID,
    submissionId: FAKE_UUID,
    rubricVersion: "0.2.3",
    audienceDocVersion: "0.1.2",
    appliedPersona: "ada",
    perCriterionScores: { mechanism_specificity: 5, evidence_quality: 4 },
    weightedTotal: "47.5",
    hardRejectsHit: ["listicle_pattern"],
    inclusionGates: {
      primary_source_identifiable: true,
      testable_or_demonstrable: false,
    },
    typeInferred: "tutorial",
    subSegmentInferred: "engineers",
    confidence: "high",
    oneLineWhy: "Mechanism is specific and reproducible.",
    finalDecision: "accept",
    routing: "feed",
    modelId: "claude-opus-4-7",
    promptHash: "sha256:deadbeef",
    costUsd: "0.0123",
    scoredAt: new Date("2026-05-06T01:23:45Z"),
  };
}

function fixtureOverride(): typeof overrideRecords.$inferSelect {
  return {
    id: "33333333-4444-5555-6666-777777777777",
    decisionRecordId: FAKE_DECISION_ID,
    reviewerId: "44444444-5555-6666-7777-888888888888",
    originalDecision: "accept",
    overrideDecision: "borderline_to_human_queue",
    overrideRouting: "human_queue",
    reviewerScores: null,
    reason: "Source quality looks weaker on second read.",
    reviewerKind: "human" as const,
    createdAt: new Date("2026-05-06T02:00:00Z"),
  };
}

/* ── Privacy whitelist ───────────────────────────────────────── */

test("DTO never carries perCriterionScores", () => {
  const dto = buildDecisionDto(fixtureRow(), null);
  assert.equal("perCriterionScores" in dto, false);
});

test("DTO never carries weightedTotal", () => {
  const dto = buildDecisionDto(fixtureRow(), null);
  assert.equal("weightedTotal" in dto, false);
});

test("DTO never carries promptHash", () => {
  const dto = buildDecisionDto(fixtureRow(), null);
  assert.equal("promptHash" in dto, false);
});

test("DTO never carries costUsd", () => {
  const dto = buildDecisionDto(fixtureRow(), null);
  assert.equal("costUsd" in dto, false);
});

/* ── Happy path ──────────────────────────────────────────────── */

test("DTO carries the public-safe fields", () => {
  const dto = buildDecisionDto(fixtureRow(), null);
  assert.equal(dto.submissionId, FAKE_UUID);
  assert.equal(dto.finalDecision, "accept");
  assert.equal(dto.routing, "feed");
  assert.equal(dto.oneLineWhy, "Mechanism is specific and reproducible.");
  assert.deepEqual(dto.hardRejectsHit, ["listicle_pattern"]);
  assert.deepEqual(dto.inclusionGates, {
    primary_source_identifiable: true,
    testable_or_demonstrable: false,
  });
  assert.equal(dto.typeInferred, "tutorial");
  assert.equal(dto.subSegmentInferred, "engineers");
  assert.equal(dto.confidence, "high");
  assert.equal(dto.appliedPersona, "ada");
  assert.equal(dto.rubricVersion, "0.2.3");
  assert.equal(dto.audienceDocVersion, "0.1.2");
  assert.equal(dto.modelId, "claude-opus-4-7");
  assert.equal(dto.scoredAt, "2026-05-06T01:23:45.000Z");
});

test("override is null when none exists", () => {
  const dto = buildDecisionDto(fixtureRow(), null);
  assert.equal(dto.override, null);
});

test("override surfaces the public-safe fields when present", () => {
  const dto = buildDecisionDto(fixtureRow(), fixtureOverride());
  assert.ok(dto.override);
  assert.equal(dto.override.overrideDecision, "borderline_to_human_queue");
  assert.equal(dto.override.overrideRouting, "human_queue");
  assert.equal(dto.override.reason, "Source quality looks weaker on second read.");
  assert.equal(dto.override.createdAt, "2026-05-06T02:00:00.000Z");
  // override block must NOT leak the per-criterion reviewer scores —
  // same privacy reasoning as the base record.
  assert.equal("reviewerScores" in dto.override, false);
  assert.equal("reviewerId" in dto.override, false);
});

/* ── Coercion of malformed jsonb ─────────────────────────────── */

test("hardRejectsHit: non-array → []", () => {
  const row = fixtureRow();
  (row as unknown as { hardRejectsHit: unknown }).hardRejectsHit = "not-an-array";
  const dto = buildDecisionDto(row, null);
  assert.deepEqual(dto.hardRejectsHit, []);
});

test("hardRejectsHit: array with non-strings → filtered", () => {
  const row = fixtureRow();
  (row as unknown as { hardRejectsHit: unknown }).hardRejectsHit = [
    "ok",
    42,
    null,
    "good",
  ];
  const dto = buildDecisionDto(row, null);
  assert.deepEqual(dto.hardRejectsHit, ["ok", "good"]);
});

test("inclusionGates: null → {}", () => {
  const row = fixtureRow();
  (row as unknown as { inclusionGates: unknown }).inclusionGates = null;
  const dto = buildDecisionDto(row, null);
  assert.deepEqual(dto.inclusionGates, {});
});

test("inclusionGates: non-boolean values are filtered", () => {
  const row = fixtureRow();
  (row as unknown as { inclusionGates: unknown }).inclusionGates = {
    pass: true,
    weird: "yes",
    nope: false,
  };
  const dto = buildDecisionDto(row, null);
  assert.deepEqual(dto.inclusionGates, { pass: true, nope: false });
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
