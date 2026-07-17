/**
 * Tests for src/lib/data-export-rate-limit-config.ts — the pure
 * pieces of the data-export send throttle.
 *
 *   pnpm tsx tests/data-export-rate-limit.test.ts
 *
 * The DB-touching half (allowDataExportSend in
 * src/lib/data-export-rate-limit.ts) needs a real Postgres and
 * belongs in tests/integration/. These unit tests pin the decision
 * logic: UTC day bucketing and the limit boundaries — same split as
 * tests/magic-link-rate-limit.test.ts.
 */

import assert from "node:assert/strict";
import {
  DATA_EXPORT_LIMITS,
  dayBucketUtc,
  withinExportLimit,
} from "../src/lib/data-export-rate-limit-config";

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

// The cap the audit prescribed: 2 export emails per user per day.
check("per-user daily cap is 2", () => {
  assert.equal(DATA_EXPORT_LIMITS.perUserPerDay, 2);
});

// dayBucketUtc — truncation to the start of the UTC day.
check("dayBucketUtc truncates hours/minutes/seconds/ms", () => {
  const d = new Date("2026-06-12T23:59:59.999Z");
  assert.equal(dayBucketUtc(d).toISOString(), "2026-06-12T00:00:00.000Z");
});
check("dayBucketUtc same bucket within a day", () => {
  const a = dayBucketUtc(new Date("2026-06-12T00:00:00.000Z"));
  const b = dayBucketUtc(new Date("2026-06-12T23:59:59.999Z"));
  assert.equal(a.getTime(), b.getTime());
});
check("dayBucketUtc different bucket across midnight UTC", () => {
  const a = dayBucketUtc(new Date("2026-06-12T23:59:59.999Z"));
  const b = dayBucketUtc(new Date("2026-06-13T00:00:00.000Z"));
  assert.notEqual(a.getTime(), b.getTime());
});
check("dayBucketUtc does not mutate its input", () => {
  const d = new Date("2026-06-12T10:30:00.000Z");
  dayBucketUtc(d);
  assert.equal(d.toISOString(), "2026-06-12T10:30:00.000Z");
});

// withinExportLimit — count-then-compare boundaries (counts are
// post-increment, so "at the limit" is still allowed).
check("withinExportLimit: first send allowed", () => {
  assert.equal(withinExportLimit(1), true);
});
check("withinExportLimit: at the limit allowed", () => {
  assert.equal(withinExportLimit(DATA_EXPORT_LIMITS.perUserPerDay), true);
});
check("withinExportLimit: over the limit denied", () => {
  assert.equal(withinExportLimit(DATA_EXPORT_LIMITS.perUserPerDay + 1), false);
});

console.log("");
console.log(`${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
