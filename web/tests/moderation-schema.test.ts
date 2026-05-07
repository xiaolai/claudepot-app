/**
 * Tests for src/lib/moderation/schema.ts (pure validator).
 *
 *   pnpm tsx tests/moderation-schema.test.ts
 *
 * The contract is the model-output shape: the wrapper validates
 * once with Zod and reconciles the verdict↔category invariant
 * server-side. These tests lock the parse + reconcile behavior so
 * a future schema bump is a deliberate change, not a silent one.
 */

import assert from "node:assert/strict";
import {
  PolicyResponseSchema,
  reconcileCategory,
} from "../src/lib/moderation/schema";

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

test("parses a happy-path pass response", () => {
  const raw = {
    verdict: "pass",
    category: null,
    confidence: "high",
    one_line_why: "Looks like a normal tutorial submission.",
    tags: [],
  };
  const parsed = PolicyResponseSchema.parse(raw);
  assert.equal(parsed.verdict, "pass");
  assert.equal(parsed.category, null);
  assert.deepEqual(parsed.tags, []);
});

test("parses a happy-path reject response", () => {
  const raw = {
    verdict: "reject",
    category: "spam",
    confidence: "high",
    one_line_why: "Promotional link with no surrounding discussion.",
    tags: [],
  };
  const parsed = PolicyResponseSchema.parse(raw);
  assert.equal(parsed.verdict, "reject");
  assert.equal(parsed.category, "spam");
});

test("parses Ada-proposed tags on a pass response", () => {
  const raw = {
    verdict: "pass",
    category: null,
    confidence: "high",
    one_line_why: "Tutorial on retrieval-augmented agents.",
    tags: [
      { slug: "rag", is_new: false },
      { slug: "ai-agents", is_new: true },
    ],
  };
  const parsed = PolicyResponseSchema.parse(raw);
  assert.equal(parsed.tags.length, 2);
  assert.equal(parsed.tags[0].slug, "rag");
  assert.equal(parsed.tags[0].is_new, false);
  assert.equal(parsed.tags[1].is_new, true);
});

test("rejects more than 2 tags", () => {
  const raw = {
    verdict: "pass",
    category: null,
    confidence: "high",
    one_line_why: "x",
    tags: [
      { slug: "a", is_new: false },
      { slug: "b", is_new: false },
      { slug: "c", is_new: false },
    ],
  };
  assert.throws(() => PolicyResponseSchema.parse(raw));
});

test("rejects malformed tag slug", () => {
  const raw = {
    verdict: "pass",
    category: null,
    confidence: "high",
    one_line_why: "x",
    tags: [{ slug: "Invalid Slug!", is_new: true }],
  };
  assert.throws(() => PolicyResponseSchema.parse(raw));
});

test("rejects an unknown category", () => {
  const raw = {
    verdict: "reject",
    category: "self_harm",
    confidence: "high",
    one_line_why: "x",
    tags: [],
  };
  assert.throws(() => PolicyResponseSchema.parse(raw));
});

test("rejects an unknown verdict", () => {
  const raw = {
    verdict: "maybe",
    category: null,
    confidence: "high",
    one_line_why: "x",
    tags: [],
  };
  assert.throws(() => PolicyResponseSchema.parse(raw));
});

test("rejects a one_line_why over the length limit", () => {
  const raw = {
    verdict: "pass",
    category: null,
    confidence: "high",
    one_line_why: "x".repeat(500),
    tags: [],
  };
  assert.throws(() => PolicyResponseSchema.parse(raw));
});

test("rejects an empty one_line_why", () => {
  const raw = {
    verdict: "pass",
    category: null,
    confidence: "high",
    one_line_why: "",
    tags: [],
  };
  assert.throws(() => PolicyResponseSchema.parse(raw));
});

test("reconcileCategory clears category when verdict is pass", () => {
  const parsed = PolicyResponseSchema.parse({
    verdict: "pass",
    category: "spam",
    confidence: "low",
    one_line_why: "borderline pass",
    tags: [],
  });
  const out = reconcileCategory(parsed);
  assert.equal(out.category, null);
  assert.equal(out.verdict, "pass");
});

test("reconcileCategory throws when reject ships category=null", () => {
  const parsed = PolicyResponseSchema.parse({
    verdict: "reject",
    category: null,
    confidence: "high",
    one_line_why: "x",
    tags: [],
  });
  assert.throws(() => reconcileCategory(parsed));
});

console.log(`\n${passed} passed · ${failed} failed`);
if (failed > 0) process.exit(1);
