/**
 * Tests for src/lib/api/cursor.ts (pure encode/decode).
 *
 *   pnpm tsx tests/api-cursor.test.ts
 *
 * Exercises:
 *   - round-trip on time-shaped and score-shaped cursors
 *   - rejection of malformed input (bad base64, bad JSON, bad shape,
 *     non-uuid id, non-finite numeric)
 *   - clampPageLimit defaulting + capping
 */

import assert from "node:assert/strict";
import {
  clampPageLimit,
  decodeCursor,
  DEFAULT_PAGE_LIMIT,
  encodeCursor,
  isCursorScore,
  isCursorTime,
  MAX_PAGE_LIMIT,
} from "../src/lib/api/cursor";

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

/* ── Round-trip ──────────────────────────────────────────────── */

test("encode/decode a time cursor round-trips", () => {
  const enc = encodeCursor({ t: 1730000000000, id: FAKE_UUID });
  const dec = decodeCursor(enc);
  assert.ok(dec);
  assert.equal(isCursorTime(dec), true);
  if (!isCursorTime(dec)) throw new Error("expected time cursor");
  assert.equal(dec.t, 1730000000000);
  assert.equal(dec.id, FAKE_UUID);
});

test("encode/decode a score cursor round-trips", () => {
  const enc = encodeCursor({ s: 42, id: FAKE_UUID });
  const dec = decodeCursor(enc);
  assert.ok(dec);
  assert.equal(isCursorScore(dec), true);
  if (!isCursorScore(dec)) throw new Error("expected score cursor");
  assert.equal(dec.s, 42);
  assert.equal(dec.id, FAKE_UUID);
});

/* ── Decode failures ─────────────────────────────────────────── */

test("decode null → null", () => {
  assert.equal(decodeCursor(null), null);
});

test("decode undefined → null", () => {
  assert.equal(decodeCursor(undefined), null);
});

test("decode empty string → null", () => {
  assert.equal(decodeCursor(""), null);
});

test("decode garbage base64 → null", () => {
  // Decodes to garbage bytes, JSON.parse throws.
  assert.equal(decodeCursor("!@#$"), null);
});

test("decode valid base64 of non-JSON → null", () => {
  const bad = Buffer.from("not json at all", "utf-8").toString("base64url");
  assert.equal(decodeCursor(bad), null);
});

test("decode valid JSON missing id → null", () => {
  const bad = Buffer.from(JSON.stringify({ t: 1 }), "utf-8").toString("base64url");
  assert.equal(decodeCursor(bad), null);
});

test("decode valid JSON with non-uuid id → null", () => {
  const bad = Buffer.from(
    JSON.stringify({ t: 1, id: "not-a-uuid" }),
    "utf-8",
  ).toString("base64url");
  assert.equal(decodeCursor(bad), null);
});

test("decode valid JSON with neither t nor s → null", () => {
  const bad = Buffer.from(
    JSON.stringify({ id: FAKE_UUID }),
    "utf-8",
  ).toString("base64url");
  assert.equal(decodeCursor(bad), null);
});

test("decode valid JSON with non-finite t → null", () => {
  const bad = Buffer.from(
    JSON.stringify({ t: "abc", id: FAKE_UUID }),
    "utf-8",
  ).toString("base64url");
  assert.equal(decodeCursor(bad), null);
});

test("decode valid JSON with non-finite s → null", () => {
  // JSON cannot represent NaN/Infinity directly; sending a string
  // for `s` is the realistic bad-shape case.
  const bad = Buffer.from(
    JSON.stringify({ s: "high", id: FAKE_UUID }),
    "utf-8",
  ).toString("base64url");
  assert.equal(decodeCursor(bad), null);
});

/* ── clampPageLimit ──────────────────────────────────────────── */

test("clampPageLimit: undefined → default", () => {
  assert.equal(clampPageLimit(undefined), DEFAULT_PAGE_LIMIT);
});

test("clampPageLimit: zero → default", () => {
  assert.equal(clampPageLimit(0), DEFAULT_PAGE_LIMIT);
});

test("clampPageLimit: negative → default", () => {
  assert.equal(clampPageLimit(-5), DEFAULT_PAGE_LIMIT);
});

test("clampPageLimit: NaN → default", () => {
  assert.equal(clampPageLimit(Number.NaN), DEFAULT_PAGE_LIMIT);
});

test("clampPageLimit: 12 → 12", () => {
  assert.equal(clampPageLimit(12), 12);
});

test("clampPageLimit: 12.7 → 12 (floor)", () => {
  assert.equal(clampPageLimit(12.7), 12);
});

test("clampPageLimit: 5000 → MAX_PAGE_LIMIT", () => {
  assert.equal(clampPageLimit(5000), MAX_PAGE_LIMIT);
});

test("clampPageLimit: string → default (non-number)", () => {
  assert.equal(clampPageLimit("12" as unknown), DEFAULT_PAGE_LIMIT);
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
