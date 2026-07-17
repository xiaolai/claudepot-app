/**
 * Tests for the privileged-scope entitlement policy in
 * src/lib/api/scopes.ts — the constants and predicate that BOTH
 * enforcement surfaces (lib/api/policy.ts:checkAuthForSpec,
 * lib/mcp/policy.ts:checkAuthForTool) and the mint path
 * (lib/actions/api-tokens.ts:createApiToken) apply.
 *
 *   pnpm tsx tests/api-scope-entitlement.test.ts
 *
 * The policy modules themselves pull in @/db/client (throws at module
 * load without a connection string), so this pins the shared pure
 * pieces: an ordinary user holding a forged editorial scope must be
 * refused by the predicate every layer consults; office identities
 * and staff must pass; citizen bots keep bots:report but stay inside
 * the CITIZEN_BOT_DENIED_SCOPES fence for every editorial scope.
 */

import assert from "node:assert/strict";
import {
  canHoldPrivilegedScopes,
  CITIZEN_BOT_DENIED_SCOPES,
  PRIVILEGED_SCOPES,
  SCOPE_GROUPS,
  type Scope,
} from "../src/lib/api/scopes";
import { CITIZEN_SCOPES } from "../src/lib/citizen-bots/scopes";

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

/* ── Identity fixtures (users.role / users.is_agent shapes) ──── */

const ordinaryUser = { role: "user", isAgent: false };
const staffHuman = { role: "staff", isAgent: false };
const officeBot = { role: "system", isAgent: true }; // alan, blair, …
const opBot = { role: "system", isAgent: true }; // otto@daemon
const citizenBot = { role: "user", isAgent: true }; // bot_kind='citizen'
const lockedUser = { role: "locked", isAgent: false };

/* ── The privileged set covers the Editorial + Bots groups ───── */

test("PRIVILEGED_SCOPES equals the Editorial + Bots scope groups", () => {
  const grouped = SCOPE_GROUPS.filter(
    (g) => g.label === "Editorial" || g.label === "Bots",
  ).flatMap((g) => g.scopes);
  assert.deepEqual(
    [...PRIVILEGED_SCOPES].sort(),
    [...grouped].sort(),
  );
});

/* ── Ordinary users: forged privileged scopes are refused ────── */

test("ordinary user is refused every privileged scope", () => {
  assert.equal(canHoldPrivilegedScopes(ordinaryUser), false);
  // Belt-and-braces: spell out the per-scope decision every
  // enforcement layer computes (PRIVILEGED_SCOPES.has(scope) &&
  // !canHoldPrivilegedScopes(owner) → refuse).
  for (const scope of PRIVILEGED_SCOPES) {
    const refused =
      PRIVILEGED_SCOPES.has(scope) && !canHoldPrivilegedScopes(ordinaryUser);
    assert.equal(refused, true, `expected refusal for ${scope}`);
  }
});

test("ordinary user with a forged decision:write token is refused", () => {
  // The A1 scenario: a token row carrying decision:write but owned
  // by role='user', is_agent=false. Scope possession alone must not
  // authorize.
  const forgedScope: Scope = "decision:write";
  assert.equal(PRIVILEGED_SCOPES.has(forgedScope), true);
  assert.equal(canHoldPrivilegedScopes(ordinaryUser), false);
});

test("locked user is refused privileged scopes", () => {
  assert.equal(canHoldPrivilegedScopes(lockedUser), false);
});

/* ── Entitled identities ─────────────────────────────────────── */

test("staff humans are entitled", () => {
  assert.equal(canHoldPrivilegedScopes(staffHuman), true);
});

test("office bots (role=system, is_agent) are entitled", () => {
  assert.equal(canHoldPrivilegedScopes(officeBot), true);
  assert.equal(canHoldPrivilegedScopes(opBot), true);
});

/* ── Citizen bots: bots:report stays, editorial stays denied ── */

test("citizen bots pass the owner predicate (bots:report must keep working)", () => {
  assert.equal(canHoldPrivilegedScopes(citizenBot), true);
  assert.equal(CITIZEN_SCOPES.includes("bots:report"), true);
  assert.equal(CITIZEN_BOT_DENIED_SCOPES.has("bots:report"), false);
});

test("every privileged scope except bots:report is citizen-denied", () => {
  for (const scope of PRIVILEGED_SCOPES) {
    if (scope === "bots:report") continue;
    assert.equal(
      CITIZEN_BOT_DENIED_SCOPES.has(scope),
      true,
      `expected citizen deny for ${scope}`,
    );
  }
});

test("no privileged scope is mintable through the citizen allowlist except bots:report", () => {
  for (const scope of CITIZEN_SCOPES) {
    if (scope === "bots:report") continue;
    assert.equal(
      PRIVILEGED_SCOPES.has(scope),
      false,
      `citizen allowlist unexpectedly carries privileged scope ${scope}`,
    );
  }
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
