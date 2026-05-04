/**
 * Tests for src/lib/social/format.ts.
 * Run: pnpm tsx tests/social-format.test.ts
 */

import { strict as assert } from "node:assert";
import { formatForX, formatForBluesky } from "../src/lib/social/format";

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

// ---- X ----------------------------------------------------------------------

test("formatForX: short text passes through unchanged", () => {
  const r = formatForX({ text: "hello" });
  assert.equal(r.text, "hello");
  assert.equal(r.truncated, false);
});

test("formatForX: appends URL with separator", () => {
  const r = formatForX({ text: "hello", url: "https://example.com" });
  assert.equal(r.text, "hello https://example.com");
  assert.equal(r.truncated, false);
});

test("formatForX: truncates text past 280 char limit", () => {
  const r = formatForX({ text: "a".repeat(300) });
  assert.equal(r.text.length, 280);
  assert.ok(r.text.endsWith("…"));
  assert.equal(r.truncated, true);
});

test("formatForX: URL counts as 23 chars regardless of actual length", () => {
  const longUrl = "https://example.com/" + "x".repeat(200);
  const textBudget = 280 - 24; // 23 (URL) + 1 (space) = 24
  const r = formatForX({ text: "a".repeat(textBudget), url: longUrl });
  assert.equal(r.truncated, false);
  assert.ok(r.text.endsWith(longUrl));
});

test("formatForX: text truncated when URL eats budget", () => {
  const r = formatForX({ text: "a".repeat(300), url: "https://example.com" });
  assert.equal(r.truncated, true);
  assert.ok(r.text.includes("…"));
  assert.ok(r.text.endsWith("https://example.com"));
});

// ---- Bluesky ---------------------------------------------------------------

test("formatForBluesky: short text passes through", () => {
  const r = formatForBluesky({ text: "hello" });
  assert.equal(r.text, "hello");
  assert.equal(r.truncated, false);
});

test("formatForBluesky: appends URL with separator", () => {
  const r = formatForBluesky({ text: "hi", url: "https://example.com" });
  assert.equal(r.text, "hi https://example.com");
});

test("formatForBluesky: truncates past 300 char limit", () => {
  const r = formatForBluesky({ text: "a".repeat(400) });
  assert.equal(r.text.length, 300);
  assert.ok(r.text.endsWith("…"));
  assert.equal(r.truncated, true);
});

test("formatForBluesky: URL counts at actual length (unlike X)", () => {
  const url20 = "https://example.com/"; // 20 chars
  const textBudget = 300 - 21; // url + 1 (space)
  const r = formatForBluesky({ text: "a".repeat(textBudget + 50), url: url20 });
  assert.equal(r.truncated, true);
  assert.ok(r.text.endsWith(url20));
});

// ---- Summary ---------------------------------------------------------------

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
