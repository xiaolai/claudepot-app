/**
 * Tests for src/lib/magic-link-rate-limit-config.ts — the pure pieces
 * of the magic-link send throttle.
 *
 *   pnpm tsx tests/magic-link-rate-limit.test.ts
 *
 * The DB-touching half (allowMagicLinkSend in
 * src/lib/magic-link-rate-limit.ts) needs a real Postgres and belongs
 * in tests/integration/. These unit tests pin the decision logic:
 * key normalization, UTC hour bucketing, the limit boundaries, and
 * client-IP extraction from forwarded headers.
 */

import assert from "node:assert/strict";
import {
  MAGIC_LINK_LIMITS,
  clientIpFromHeaders,
  hourBucketUtc,
  normalizeEmail,
  withinLimits,
} from "../src/lib/magic-link-rate-limit-config";

let passed = 0;
let failed = 0;

function check(label: string, fn: () => void) {
  try {
    fn();
    console.log(`PASS  ${label}`);
    passed += 1;
  } catch (err) {
    console.error(`FAIL  ${label}`);
    console.error(`      ${err}`);
    failed += 1;
  }
}

function stubHeaders(entries: Record<string, string>) {
  const map = new Map(
    Object.entries(entries).map(([k, v]) => [k.toLowerCase(), v]),
  );
  return { get: (name: string) => map.get(name.toLowerCase()) ?? null };
}

// Limits — sanity relative to each other: the per-IP budget must allow
// at least one full per-email window (otherwise a single legitimate
// user retrying could exhaust their own IP before their email bucket).
check("per-IP limit exceeds per-email limit", () => {
  assert.ok(MAGIC_LINK_LIMITS.perIpPerHour > MAGIC_LINK_LIMITS.perEmailPerHour);
});

// normalizeEmail — case + whitespace must collapse onto one key.
check("normalizeEmail lowercases", () => {
  assert.equal(normalizeEmail("Foo@Bar.COM"), "foo@bar.com");
});
check("normalizeEmail trims", () => {
  assert.equal(normalizeEmail("  a@b.c \n"), "a@b.c");
});

// hourBucketUtc — truncation to the start of the UTC hour.
check("hourBucketUtc truncates minutes/seconds/ms", () => {
  const d = new Date("2026-06-12T10:59:59.999Z");
  assert.equal(hourBucketUtc(d).toISOString(), "2026-06-12T10:00:00.000Z");
});
check("hourBucketUtc same bucket within an hour", () => {
  const a = hourBucketUtc(new Date("2026-06-12T10:00:00.000Z"));
  const b = hourBucketUtc(new Date("2026-06-12T10:59:59.999Z"));
  assert.equal(a.getTime(), b.getTime());
});
check("hourBucketUtc different bucket across the boundary", () => {
  const a = hourBucketUtc(new Date("2026-06-12T10:59:59.999Z"));
  const b = hourBucketUtc(new Date("2026-06-12T11:00:00.000Z"));
  assert.notEqual(a.getTime(), b.getTime());
});
check("hourBucketUtc does not mutate its input", () => {
  const d = new Date("2026-06-12T10:30:00.000Z");
  hourBucketUtc(d);
  assert.equal(d.toISOString(), "2026-06-12T10:30:00.000Z");
});

// withinLimits — count-then-compare boundaries (counts are
// post-increment, so "at the limit" is still allowed).
check("withinLimits: at email limit allowed", () => {
  assert.equal(withinLimits(MAGIC_LINK_LIMITS.perEmailPerHour, 1), true);
});
check("withinLimits: over email limit denied", () => {
  assert.equal(withinLimits(MAGIC_LINK_LIMITS.perEmailPerHour + 1, 1), false);
});
check("withinLimits: at IP limit allowed", () => {
  assert.equal(withinLimits(1, MAGIC_LINK_LIMITS.perIpPerHour), true);
});
check("withinLimits: over IP limit denied", () => {
  assert.equal(withinLimits(1, MAGIC_LINK_LIMITS.perIpPerHour + 1), false);
});
check("withinLimits: both over denied", () => {
  assert.equal(
    withinLimits(
      MAGIC_LINK_LIMITS.perEmailPerHour + 1,
      MAGIC_LINK_LIMITS.perIpPerHour + 1,
    ),
    false,
  );
});
check("withinLimits: ipCount 0 (unknown IP) leaves email limit in force", () => {
  assert.equal(withinLimits(1, 0), true);
  assert.equal(withinLimits(MAGIC_LINK_LIMITS.perEmailPerHour + 1, 0), false);
});

// clientIpFromHeaders — first hop of x-forwarded-for wins.
check("clientIp: single x-forwarded-for", () => {
  assert.equal(
    clientIpFromHeaders(stubHeaders({ "x-forwarded-for": "203.0.113.7" })),
    "203.0.113.7",
  );
});
check("clientIp: multi-hop takes first", () => {
  assert.equal(
    clientIpFromHeaders(
      stubHeaders({ "x-forwarded-for": "203.0.113.7, 10.0.0.1, 10.0.0.2" }),
    ),
    "203.0.113.7",
  );
});
check("clientIp: falls back to x-real-ip", () => {
  assert.equal(
    clientIpFromHeaders(stubHeaders({ "x-real-ip": "198.51.100.4" })),
    "198.51.100.4",
  );
});
check("clientIp: no headers → null", () => {
  assert.equal(clientIpFromHeaders(stubHeaders({})), null);
});
check("clientIp: whitespace-only values → null", () => {
  assert.equal(
    clientIpFromHeaders(
      stubHeaders({ "x-forwarded-for": " ", "x-real-ip": "  " }),
    ),
    null,
  );
});

console.log("");
console.log(`${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
