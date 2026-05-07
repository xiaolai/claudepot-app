/**
 * Tests for src/lib/api/constitution.ts (pure logic).
 *
 *   pnpm tsx tests/api-constitution.test.ts
 *
 * Exercises:
 *   - ifNoneMatchMatches handles single tag, list, *, missing header,
 *     and (deliberately) misses on weak (W/) prefixes.
 *   - etagFor wraps the version in double quotes.
 *   - getConstitution memoizes (same object across calls).
 *   - getConstitution falls back to a content hash when
 *     VERCEL_GIT_COMMIT_SHA is unset, and uses the env value when set.
 *
 * The helper does I/O (reads editorial/*); we run from repo root so
 * the relative paths in editorial-spec.ts resolve correctly.
 */

import assert from "node:assert/strict";
import {
  _resetConstitutionCacheForTests,
  etagFor,
  getConstitution,
  ifNoneMatchMatches,
} from "../src/lib/api/constitution";

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

/* ── etagFor ─────────────────────────────────────────────────── */

test("etagFor wraps the version in double quotes", () => {
  assert.equal(etagFor("abc123"), '"abc123"');
});

/* ── ifNoneMatchMatches ──────────────────────────────────────── */

test("ifNoneMatchMatches: missing header → false", () => {
  assert.equal(ifNoneMatchMatches(null, '"abc"'), false);
});

test("ifNoneMatchMatches: empty header → false", () => {
  assert.equal(ifNoneMatchMatches("", '"abc"'), false);
});

test("ifNoneMatchMatches: exact match → true", () => {
  assert.equal(ifNoneMatchMatches('"abc"', '"abc"'), true);
});

test("ifNoneMatchMatches: mismatch → false", () => {
  assert.equal(ifNoneMatchMatches('"def"', '"abc"'), false);
});

test("ifNoneMatchMatches: list with match → true", () => {
  assert.equal(ifNoneMatchMatches('"old", "abc", "newer"', '"abc"'), true);
});

test("ifNoneMatchMatches: list without match → false", () => {
  assert.equal(ifNoneMatchMatches('"old", "newer"', '"abc"'), false);
});

test("ifNoneMatchMatches: wildcard * → true", () => {
  assert.equal(ifNoneMatchMatches("*", '"abc"'), true);
});

test("ifNoneMatchMatches: weak ETag prefix W/ does NOT match", () => {
  // We issue strong ETags only; a weak revalidation should fall
  // through to the full body so the client can refresh its cache.
  assert.equal(ifNoneMatchMatches('W/"abc"', '"abc"'), false);
});

/* ── getConstitution ─────────────────────────────────────────── */

test("getConstitution: shape contains audience, rubric, transparency", () => {
  delete process.env.VERCEL_GIT_COMMIT_SHA;
  _resetConstitutionCacheForTests();
  const c = getConstitution();
  assert.equal(c.audience.path, "editorial/audience.md");
  assert.equal(c.rubric.path, "editorial/rubric.yml");
  assert.equal(c.transparency.path, "editorial/transparency.md");
  assert.ok(c.audience.markdown.length > 0);
  assert.ok(c.rubric.yaml.length > 0);
  assert.ok(c.transparency.markdown.length > 0);
  // public view of rubric — weights MUST be absent.
  assert.ok(c.rubric.public.version);
  assert.ok(Array.isArray(c.rubric.public.quality_criteria));
  for (const crit of c.rubric.public.quality_criteria) {
    assert.equal(
      "weight" in crit,
      false,
      `rubric.public leaked a weight on ${crit.id}`,
    );
  }
});

test("getConstitution: memoized — second call returns same object", () => {
  delete process.env.VERCEL_GIT_COMMIT_SHA;
  _resetConstitutionCacheForTests();
  const first = getConstitution();
  const second = getConstitution();
  assert.equal(first, second, "expected referential equality from cache");
});

test("getConstitution: version is content hash when env unset", () => {
  delete process.env.VERCEL_GIT_COMMIT_SHA;
  _resetConstitutionCacheForTests();
  const c = getConstitution();
  // 12-char sha256 prefix.
  assert.match(c.version, /^[0-9a-f]{12}$/);
});

test("getConstitution: version is the Vercel SHA when env is set", () => {
  process.env.VERCEL_GIT_COMMIT_SHA = "deadbeefcafef00d";
  _resetConstitutionCacheForTests();
  const c = getConstitution();
  assert.equal(c.version, "deadbeefcafef00d");
  delete process.env.VERCEL_GIT_COMMIT_SHA;
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
