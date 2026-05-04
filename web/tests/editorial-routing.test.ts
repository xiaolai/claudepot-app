/**
 * Tests for src/lib/editorial/routing.ts.
 * Pure functions only — no API calls, no file I/O.
 *
 *   pnpm tsx tests/editorial-routing.test.ts
 */

import { strict as assert } from "node:assert";
import { computeWeightedTotal, decideRouting } from "../src/lib/editorial/routing";
import type { Rubric, ScoreResponse } from "../src/lib/editorial/types";

let passed = 0;
let failed = 0;

function test(name: string, fn: () => void) {
  try {
    fn();
    console.log(`✓ ${name}`);
    passed++;
  } catch (err) {
    console.error(`✗ ${name}`);
    console.error(`    ${err instanceof Error ? err.message : String(err)}`);
    failed++;
  }
}

// Minimal rubric stub — just enough to exercise routing math.
const rubric: Rubric = {
  version: "0.2.3",
  audience: { doc: "editorial/audience.md", doc_version_pinned: "0.1.2", sub_segment_ids: [] },
  routing: {
    feed_threshold: 37,
    borderline_threshold: 5,
    destinations: { feed: "", firehose: "", human_queue: "" },
  },
  hard_rejects: [],
  inclusion_gates: [],
  quality_score: {
    mechanism_specificity: { weight: 5, scale: [0, 5], rubric: "" },
    evidence_quality: { weight: 5, scale: [0, 5], rubric: "" },
    practitioner_fit: { weight: 4, scale: [0, 5], rubric: "" },
    domain_legibility: { weight: 2, scale: [0, 3], rubric: "" },
    counter_current: { weight: 3, scale: [0, 3], rubric: "" },
    author_credibility: { weight: 2, scale: [0, 3], rubric: "" },
    recency_bonus: { weight: 2, scale: [0, 3], rubric: "" },
    diversity_bonus: { weight: 2, scale: [0, 3], rubric: "" },
  },
  persona_overlays: {
    ada: {
      description: "",
      multipliers: { evidence_quality: 1.5, mechanism_specificity: 1.2, counter_current: 0.8 },
    },
    historian: { description: "", multipliers: {} },
    scout: { description: "", multipliers: {} },
  },
};

function passingGates(): ScoreResponse["inclusion_gates"] {
  return {
    primary_source_identifiable: true,
    testable_or_demonstrable: true,
    actionable_within_one_week: true,
    within_recency_window: true,
  };
}

function makeResponse(scores: Partial<Record<string, number>>, overrides: Partial<ScoreResponse> = {}): ScoreResponse {
  return {
    hard_rejects_hit: [],
    inclusion_gates: passingGates(),
    scores: {
      mechanism_specificity: 0,
      evidence_quality: 0,
      practitioner_fit: 0,
      domain_legibility: 0,
      counter_current: 0,
      author_credibility: 0,
      recency_bonus: 0,
      diversity_bonus: 0,
      ...scores,
    } as ScoreResponse["scores"],
    type_inferred: "discussion",
    sub_segment_inferred: "engineers",
    confidence: "high",
    one_line_why: "test",
    ...overrides,
  };
}

// ---- computeWeightedTotal --------------------------------------------------

test("base persona: total = sum of score * weight", () => {
  const r = makeResponse({ mechanism_specificity: 5, evidence_quality: 5, practitioner_fit: 5 });
  const total = computeWeightedTotal(r.scores, rubric, "base");
  assert.equal(total, 5 * 5 + 5 * 5 + 5 * 4);
});

test("base persona: zeros yield zero", () => {
  const r = makeResponse({});
  assert.equal(computeWeightedTotal(r.scores, rubric, "base"), 0);
});

test("ada persona: evidence_quality is multiplied by 1.5", () => {
  const r = makeResponse({ evidence_quality: 5 });
  const total = computeWeightedTotal(r.scores, rubric, "ada");
  assert.equal(total, 5 * 5 * 1.5);
});

test("ada persona: counter_current is dampened by 0.8", () => {
  const r = makeResponse({ counter_current: 3 });
  const total = computeWeightedTotal(r.scores, rubric, "ada");
  assert.equal(total, 3 * 3 * 0.8);
});

test("personas: criterion without an overlay multiplier defaults to 1.0", () => {
  const r = makeResponse({ recency_bonus: 3 });
  assert.equal(computeWeightedTotal(r.scores, rubric, "ada"), 3 * 2);
});

// ---- decideRouting --------------------------------------------------------

test("hard reject → reject + firehose, total 0", () => {
  const r = makeResponse({ mechanism_specificity: 5 }, { hard_rejects_hit: ["listicle_pattern"] });
  const result = decideRouting(r, rubric, "base");
  assert.equal(result.final_decision, "reject");
  assert.equal(result.routing, "firehose");
  assert.equal(result.weighted_total, 0);
});

test("any failed gate → reject + firehose, total 0", () => {
  const r = makeResponse(
    { mechanism_specificity: 5 },
    { inclusion_gates: { ...passingGates(), actionable_within_one_week: false } }
  );
  const result = decideRouting(r, rubric, "base");
  assert.equal(result.final_decision, "reject");
  assert.equal(result.routing, "firehose");
});

test("score below borderline-low → reject + firehose", () => {
  // feed_threshold 37, borderline 5 → low cutoff = 32. Total = 25 (5 mechanism * 5 weight) → below.
  const r = makeResponse({ mechanism_specificity: 5 });
  const result = decideRouting(r, rubric, "base");
  assert.equal(result.final_decision, "reject");
  assert.equal(result.routing, "firehose");
  assert.equal(result.weighted_total, 25);
});

test("score within borderline → human_queue", () => {
  // total 35 → within ±5 of 37 → human_queue.
  // 5 mechanism * 5 + 2 evidence * 5 = 25 + 10 = 35
  const r = makeResponse({ mechanism_specificity: 5, evidence_quality: 2 });
  const result = decideRouting(r, rubric, "base");
  assert.equal(result.final_decision, "borderline_to_human_queue");
  assert.equal(result.routing, "human_queue");
  assert.equal(result.weighted_total, 35);
});

test("score at borderline_high (42) → still human_queue (inclusive)", () => {
  // 5 mechanism * 5 + 3 evidence * 5 + 1 practitioner * 4 - wait recompute:
  // need exactly 42. 4 mechanism * 5 + 4 evidence * 5 + 0.5 practitioner... can't with int.
  // Try 5 mechanism * 5 + 3 evidence * 5 + 0 + 1 author * 2 = 25+15+2 = 42.
  const r = makeResponse({ mechanism_specificity: 5, evidence_quality: 3, author_credibility: 1 });
  const result = decideRouting(r, rubric, "base");
  assert.equal(result.weighted_total, 42);
  assert.equal(result.final_decision, "borderline_to_human_queue");
  assert.equal(result.routing, "human_queue");
});

test("score above borderline_high → accept + feed", () => {
  // total 50 → above 42 → accept.
  // 5 mechanism * 5 + 5 evidence * 5 = 50
  const r = makeResponse({ mechanism_specificity: 5, evidence_quality: 5 });
  const result = decideRouting(r, rubric, "base");
  assert.equal(result.final_decision, "accept");
  assert.equal(result.routing, "feed");
  assert.equal(result.weighted_total, 50);
});

test("ada persona pushes evidence-heavy submission above feed_threshold", () => {
  // 5 evidence * 5 weight * 1.5 multiplier = 37.5 → just barely above 37, but within borderline of ±5 → human_queue.
  // Need to cross above 42. 5 evidence * 5 * 1.5 + 1 author * 2 = 37.5 + 2 = 39.5 → still borderline.
  // Try 5 evidence * 5 * 1.5 + 1 mechanism * 5 * 1.2 = 37.5 + 6 = 43.5 → above borderline → accept.
  const r = makeResponse({ evidence_quality: 5, mechanism_specificity: 1 });
  const result = decideRouting(r, rubric, "ada");
  assert.equal(result.weighted_total, 43.5);
  assert.equal(result.final_decision, "accept");
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
