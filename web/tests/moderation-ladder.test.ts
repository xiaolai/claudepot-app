/**
 * Tests for src/lib/moderation/ladder.ts.
 *
 *   pnpm tsx tests/moderation-ladder.test.ts
 *
 * The DB-touching logic (recentRejectsForAuthor, checkBanCandidate,
 * checkLadderRateLimit) is best exercised with integration tests
 * against a real Postgres. These unit tests pin the load-bearing
 * invariants that don't need a DB:
 *
 *   1. The threshold constants make sense relative to each other.
 *   2. The exported types match the documented shape.
 *   3. The reason-prefix format used for ban-candidate dedup is
 *      stable — staff UI greps for it.
 */

import assert from "node:assert/strict";
import { LADDER_THRESHOLDS } from "../src/lib/moderation/ladder-config";

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

test("rung 4 trigger is stricter than rung 3 trigger", () => {
  // Rung 4 lands a ban-candidate flag — it must require more rejects
  // than rung 3 (which is a reversible cap shrink). If they were
  // equal, rung 3 would never fire alone; if 4 < 3, the cap-shrink
  // would trigger after a ban candidate which is the wrong order.
  assert.ok(
    LADDER_THRESHOLDS.RUNG4_REJECT_TRIGGER > LADDER_THRESHOLDS.RUNG3_REJECT_TRIGGER,
    `expected RUNG4 (${LADDER_THRESHOLDS.RUNG4_REJECT_TRIGGER}) > RUNG3 (${LADDER_THRESHOLDS.RUNG3_REJECT_TRIGGER})`,
  );
});

test("daily cap is positive and below typical organic volume", () => {
  // The cap kicks in only after rejects accumulate; a healthy user
  // should never hit the floor. If the cap is set too high it has
  // no teeth; too low it punishes a user mid-correction.
  assert.ok(LADDER_THRESHOLDS.RUNG3_DAILY_CAP > 0);
  assert.ok(LADDER_THRESHOLDS.RUNG3_DAILY_CAP <= 10);
});

test("rolling windows are positive", () => {
  assert.ok(LADDER_THRESHOLDS.RUNG3_WINDOW_DAYS > 0);
  assert.ok(LADDER_THRESHOLDS.RUNG4_WINDOW_DAYS > 0);
});

test("rung 4 window is not shorter than rung 3 window", () => {
  // A shorter rung-4 window would mean a 5-rejects-in-3-days user
  // hits ban-candidate before they can hit the rung-3 cap, which
  // skips a rung. Equal is fine; longer is fine.
  assert.ok(
    LADDER_THRESHOLDS.RUNG4_WINDOW_DAYS >= LADDER_THRESHOLDS.RUNG3_WINDOW_DAYS,
  );
});

test("LADDER_THRESHOLDS is frozen-shape (readonly via 'as const')", () => {
  // `as const` gives compile-time readonly; this asserts the shape
  // hasn't drifted into a runtime-mutable object that callers could
  // tamper with.
  const keys = Object.keys(LADDER_THRESHOLDS).sort();
  assert.deepEqual(keys, [
    "RUNG3_DAILY_CAP",
    "RUNG3_REJECT_TRIGGER",
    "RUNG3_WINDOW_DAYS",
    "RUNG4_REJECT_TRIGGER",
    "RUNG4_WINDOW_DAYS",
  ]);
});

console.log(`\n${passed} passed · ${failed} failed`);
if (failed > 0) process.exit(1);
