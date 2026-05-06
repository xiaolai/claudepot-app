/**
 * Tests for src/lib/moderation/exempt.ts.
 *
 *   pnpm tsx tests/moderation-exempt.test.ts
 *
 * Pure logic — no DB, no network. The two load-bearing claims:
 *
 *   1. staff/system roles are unconditionally exempt.
 *   2. bot_moderation_exempt=true requires is_agent=true; a manual
 *      DB flip that violates this asserts loudly at runtime.
 */

import assert from "node:assert/strict";
import { isExemptFromModeration } from "../src/lib/moderation/exempt";
import type { ModerationAuthor } from "../src/lib/moderation/types";

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

const baseAuthor = (overrides: Partial<ModerationAuthor> = {}): ModerationAuthor => ({
  id: "00000000-0000-0000-0000-000000000aaa",
  role: "user",
  isAgent: false,
  botModerationExempt: false,
  ...overrides,
});

test("staff role is exempt", () => {
  assert.equal(isExemptFromModeration(baseAuthor({ role: "staff" })), true);
});

test("system role is exempt", () => {
  assert.equal(isExemptFromModeration(baseAuthor({ role: "system" })), true);
});

test("plain user is not exempt", () => {
  assert.equal(isExemptFromModeration(baseAuthor()), false);
});

test("locked user is not exempt", () => {
  // Locked users never reach this check in practice — defense in depth.
  assert.equal(isExemptFromModeration(baseAuthor({ role: "locked" })), false);
});

test("bot with exempt=true is exempt", () => {
  assert.equal(
    isExemptFromModeration(
      baseAuthor({ isAgent: true, botModerationExempt: true }),
    ),
    true,
  );
});

test("bot with exempt=false is NOT exempt", () => {
  // Default new-bot state: must opt in via /admin/users.
  assert.equal(
    isExemptFromModeration(baseAuthor({ isAgent: true, botModerationExempt: false })),
    false,
  );
});

test("non-bot with exempt=true throws (defense in depth)", () => {
  // The /admin/users UI prevents this; the runtime assert catches a
  // manual DB edit. Throwing is the right behavior — we'd rather a
  // single submission fail than silently skip the moderator.
  assert.throws(() =>
    isExemptFromModeration(
      baseAuthor({ role: "user", isAgent: false, botModerationExempt: true }),
    ),
  );
});

console.log(`\n${passed} passed · ${failed} failed`);
if (failed > 0) process.exit(1);
