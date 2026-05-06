/**
 * Tests for src/lib/moderation/prompt.ts.
 *
 *   pnpm tsx tests/moderation-prompt.test.ts
 *
 * The prompt is public (lives in this repo, ships to Vercel) and
 * versioned via POLICY_PROMPT_V. These tests lock invariants we
 * never want a casual edit to break:
 *
 *   1. The system prompt names every category in POLICY_CATEGORIES
 *      exactly once each — drift in the taxonomy must be deliberate.
 *   2. The user prompt includes both kind and body, and includes
 *      the title only when present.
 *   3. The JSON schema we send to OpenAI matches the categories
 *      defined in types.ts.
 *
 * The tests do NOT snapshot the full prompt body — that would
 * convert every typo fix into a snapshot update. Instead they
 * pin the load-bearing claims.
 */

import assert from "node:assert/strict";
import {
  POLICY_RESPONSE_JSON_SCHEMA,
  buildSystemPrompt,
  buildUserPrompt,
} from "../src/lib/moderation/prompt";
import { POLICY_CATEGORIES } from "../src/lib/moderation/types";

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

test("system prompt names every category once at the top", () => {
  const prompt = buildSystemPrompt();
  for (const cat of POLICY_CATEGORIES) {
    // Each category name appears in a numbered "1. spam — …" entry,
    // so anchor on "<n>. <category>" to avoid false positives where
    // the category word appears inline elsewhere.
    const re = new RegExp(`\\d\\.\\s+${cat}`);
    assert.match(prompt, re, `category ${cat} missing from numbered list`);
  }
});

test("system prompt includes the verdict-category invariant", () => {
  const prompt = buildSystemPrompt();
  // The model is told category=null on pass.
  assert.match(prompt, /category:\s*null on pass/);
});

test("user prompt includes 'Submission' for kind=submission", () => {
  const prompt = buildUserPrompt({
    kind: "submission",
    title: "Tutorial: prompt patterns for legal review",
    body: "Here's a step-by-step…",
  });
  assert.match(prompt, /Type:\s+Submission/);
  assert.match(prompt, /Tutorial: prompt patterns for legal review/);
  assert.match(prompt, /Here's a step-by-step…/);
});

test("user prompt skips the Title block when title is empty", () => {
  const prompt = buildUserPrompt({
    kind: "comment",
    title: "",
    body: "Nice writeup.",
  });
  assert.match(prompt, /Type:\s+Comment/);
  // Title should not appear as a labeled block.
  assert.doesNotMatch(prompt, /^Title:/m);
  assert.match(prompt, /Body:\nNice writeup\./);
});

test("user prompt trims surrounding whitespace on title and body", () => {
  const prompt = buildUserPrompt({
    kind: "submission",
    title: "  spaced title  ",
    body: "\n\nspaced body\n\n",
  });
  assert.match(prompt, /Title:\nspaced title/);
  assert.match(prompt, /Body:\nspaced body/);
});

test("JSON schema lists exactly POLICY_CATEGORIES plus null", () => {
  const props = POLICY_RESPONSE_JSON_SCHEMA.schema
    .properties as Record<string, unknown>;
  const cat = props.category as { enum: ReadonlyArray<string | null> };
  // null + every category in POLICY_CATEGORIES.
  assert.equal(cat.enum.length, POLICY_CATEGORIES.length + 1);
  for (const c of POLICY_CATEGORIES) {
    assert.ok(cat.enum.includes(c), `JSON schema missing category ${c}`);
  }
  assert.ok(cat.enum.includes(null), "JSON schema missing null category");
  // Negative: the legacy off_topic value must not be in the v=2 enum.
  assert.ok(
    !cat.enum.includes("off_topic"),
    "off_topic should be retired from v=2 schema",
  );
});

test("JSON schema flags strict + additionalProperties=false", () => {
  assert.equal(POLICY_RESPONSE_JSON_SCHEMA.strict, true);
  assert.equal(
    POLICY_RESPONSE_JSON_SCHEMA.schema.additionalProperties,
    false,
  );
});

console.log(`\n${passed} passed · ${failed} failed`);
if (failed > 0) process.exit(1);
