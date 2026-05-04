/**
 * Tests for the username library — seed derivation, candidate generation,
 * shape validation, and self-rename eligibility decisions.
 *
 *   pnpm tsx tests/username.test.ts
 */

import assert from "node:assert/strict";

import {
  MAX_SELF_RENAMES,
  SELF_RENAME_COOLDOWN_MINUTES,
  SELF_RENAME_GRACE_DAYS,
  canSelfRename,
  generateUsernameCandidates,
  isReservedUsername,
  isValidUsernameShape,
  normalizeUsername,
  usernameFromEmail,
  usernameFromName,
} from "../src/lib/username";

let failed = 0;
function ok(name: string, fn: () => void) {
  try {
    fn();
    console.log(`  PASS  ${name}`);
  } catch (e) {
    failed += 1;
    console.error(`  FAIL  ${name}`);
    console.error(e);
  }
}

console.log("normalizeUsername:");
ok("strips leading @", () => assert.equal(normalizeUsername("@xiaolai"), "xiaolai"));
ok("strips multiple @", () => assert.equal(normalizeUsername("@@@xl"), "xl"));
ok("trims and lowers", () => assert.equal(normalizeUsername("  XL  "), "xl"));

console.log("isValidUsernameShape:");
ok("accepts simple ascii", () => assert.equal(isValidUsernameShape("xiaolai"), true));
ok("accepts internal dash", () => assert.equal(isValidUsernameShape("li-xiaolai"), true));
ok("accepts digits", () => assert.equal(isValidUsernameShape("agent007"), true));
ok("rejects too short", () => assert.equal(isValidUsernameShape("ab"), false));
ok("rejects too long", () =>
  assert.equal(isValidUsernameShape("a".repeat(25)), false));
ok("rejects leading dash", () => assert.equal(isValidUsernameShape("-x"), false));
ok("rejects trailing dash", () =>
  assert.equal(isValidUsernameShape("x-"), false));
ok("rejects double dash", () =>
  assert.equal(isValidUsernameShape("a--b"), false));
ok("rejects underscores", () =>
  assert.equal(isValidUsernameShape("a_b"), false));
ok("rejects spaces", () => assert.equal(isValidUsernameShape("a b"), false));
ok("rejects uppercase", () => assert.equal(isValidUsernameShape("Ada"), false));

console.log("isReservedUsername:");
ok("flags admin", () => assert.equal(isReservedUsername("admin"), true));
ok("flags @-prefixed", () => assert.equal(isReservedUsername("@admin"), true));
ok("does not flag random", () =>
  assert.equal(isReservedUsername("ada-lovelace"), false));
ok("flags brand-owner names", () => {
  assert.equal(isReservedUsername("xiaolai"), true);
  assert.equal(isReservedUsername("lixiaolai"), true);
  assert.equal(isReservedUsername("claudepot"), true);
});

console.log("usernameFromName:");
ok("simple lowercase", () => assert.equal(usernameFromName("xiaolai"), "xiaolai"));
ok("dashes spaces", () =>
  assert.equal(usernameFromName("Li Xiaolai"), "li-xiaolai"));
ok("strips special chars", () =>
  assert.equal(usernameFromName("Ada Lovelace ★"), "ada-lovelace"));
ok("collapses dashes", () =>
  assert.equal(usernameFromName("ada---lovelace"), "ada-lovelace"));
ok("handles trailing slug", () => {
  const out = usernameFromName("@@@");
  assert.match(out, /^user-[0-9a-f]{4,}$/);
});
ok("pads short names", () => {
  const out = usernameFromName("a");
  assert.equal(out.length >= 3, true);
  assert.equal(isValidUsernameShape(out), true);
});
ok("truncates long names", () => {
  const out = usernameFromName("the-quick-brown-fox-jumps-over-the-lazy-dog");
  assert.equal(out.length <= 24, true);
  assert.equal(isValidUsernameShape(out), true);
});

console.log("usernameFromEmail:");
ok("strips +tag", () =>
  assert.equal(usernameFromEmail("ada+tag@example.com"), "ada"));
ok("uses local part", () =>
  assert.equal(usernameFromEmail("li.xiaolai@example.com"), "li-xiaolai"));

console.log("generateUsernameCandidates:");
ok("first candidate is the seed", () => {
  const gen = generateUsernameCandidates("xiaolai");
  assert.equal(gen.next().value, "xiaolai");
});
ok("then -2, -3, -4 …", () => {
  const gen = generateUsernameCandidates("xiaolai");
  gen.next(); // skip seed
  assert.equal(gen.next().value, "xiaolai-2");
  assert.equal(gen.next().value, "xiaolai-3");
  assert.equal(gen.next().value, "xiaolai-4");
});
ok("never exceeds 24 chars", () => {
  const gen = generateUsernameCandidates("a".repeat(24));
  for (let i = 0; i < 50; i += 1) {
    const { value } = gen.next();
    if (!value) break;
    assert.equal(value.length <= 24, true, `oversize: ${value}`);
    assert.equal(isValidUsernameShape(value), true, `invalid: ${value}`);
  }
});
ok("dedupes", () => {
  const gen = generateUsernameCandidates("ab");
  const seen = new Set<string>();
  for (let i = 0; i < 200; i += 1) {
    const { value } = gen.next();
    if (!value) break;
    assert.equal(seen.has(value), false, `duplicate: ${value}`);
    seen.add(value);
  }
});

console.log("canSelfRename:");
const NOW = new Date("2026-04-30T12:00:00Z");
const IN_GRACE = new Date("2026-04-29T12:00:00Z"); // 1 day in
const PAST_GRACE = new Date("2026-04-22T11:00:00Z"); // > 7 days
ok("ok when fresh", () => {
  const d = canSelfRename(
    {
      createdAt: IN_GRACE,
      selfUsernameRenameCount: 0,
      usernameLastChangedAt: null,
    },
    NOW,
  );
  assert.deepEqual(d, { ok: true });
});
ok("grace expired", () => {
  const d = canSelfRename(
    {
      createdAt: PAST_GRACE,
      selfUsernameRenameCount: 0,
      usernameLastChangedAt: null,
    },
    NOW,
  );
  assert.deepEqual(d, { ok: false, reason: "grace_expired" });
});
ok("count exceeded", () => {
  const d = canSelfRename(
    {
      createdAt: IN_GRACE,
      selfUsernameRenameCount: MAX_SELF_RENAMES,
      usernameLastChangedAt: null,
    },
    NOW,
  );
  assert.deepEqual(d, { ok: false, reason: "count_exceeded" });
});
ok("cooldown active", () => {
  const justRenamed = new Date(
    NOW.getTime() - (SELF_RENAME_COOLDOWN_MINUTES - 1) * 60 * 1000,
  );
  const d = canSelfRename(
    {
      createdAt: IN_GRACE,
      selfUsernameRenameCount: 1,
      usernameLastChangedAt: justRenamed,
    },
    NOW,
  );
  assert.deepEqual(d, { ok: false, reason: "cooldown" });
});
ok("cooldown expired", () => {
  const longAgo = new Date(
    NOW.getTime() - (SELF_RENAME_COOLDOWN_MINUTES + 1) * 60 * 1000,
  );
  const d = canSelfRename(
    {
      createdAt: IN_GRACE,
      selfUsernameRenameCount: 1,
      usernameLastChangedAt: longAgo,
    },
    NOW,
  );
  assert.deepEqual(d, { ok: true });
});

console.log(`\n${failed === 0 ? "all green" : `${failed} failure(s)`}`);
process.exit(failed === 0 ? 0 : 1);

// Reference SELF_RENAME_GRACE_DAYS so a future change there triggers
// type errors here if the constant is removed.
void SELF_RENAME_GRACE_DAYS;
