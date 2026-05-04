/**
 * End-to-end smoke test for the public API token surface.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/smoke-api-tokens.ts [BASE_URL]
 *
 * Defaults to http://localhost:3000 — start `pnpm dev` first.
 *
 * Each run creates a fresh ephemeral user (`smoke-<random>`) with role=user
 * (NOT system — we don't want to leave privileged residual identities in
 * shared databases), mints a token for them, exercises GET /api/v1/me,
 * verifies the response shape and that last_used_at gets bumped, then
 * deletes both the token and the user. CASCADE on the foreign keys cleans
 * up the audit-event row alongside.
 */

import { randomBytes } from "node:crypto";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, users } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";

const BASE_URL = process.argv[2] ?? "http://localhost:3000";
const RUN_TAG = randomBytes(4).toString("hex");
const TEST_USERNAME = `smoke-${RUN_TAG}`;

async function createTestUser() {
  const [created] = await db
    .insert(users)
    .values({
      username: TEST_USERNAME,
      email: `${TEST_USERNAME}@smoke.invalid`,
      name: `Smoke ${RUN_TAG}`,
      role: "user",
      isAgent: false,
    })
    .returning();
  return created;
}

async function main() {
  console.log(`> Smoke test against ${BASE_URL}`);

  const user = await createTestUser();
  console.log(`✓ Created test user: @${user.username} (${user.id})`);

  const { plaintext, hashed, displayPrefix } = generateToken();
  const expiresAt = new Date(Date.now() + 15 * 60 * 1000);
  const [tokenRow] = await db
    .insert(apiTokens)
    .values({
      userId: user.id,
      name: `smoke-${Date.now()}`,
      displayPrefix,
      hashedSecret: hashed,
      scopes: ["read:all"],
      expiresAt,
    })
    .returning();
  console.log(`✓ Minted token ${displayPrefix}… (id ${tokenRow.id})`);

  let allPassed = true;
  const fail = (msg: string) => {
    console.error(`✗ ${msg}`);
    allPassed = false;
  };

  try {
    /* ── 1. Happy path: valid token returns user + token shape ── */
    const t0 = Date.now();
    const res = await fetch(`${BASE_URL}/api/v1/me`, {
      headers: { Authorization: `Bearer ${plaintext}` },
    });
    const elapsed = Date.now() - t0;
    if (res.status !== 200) {
      fail(`/me returned ${res.status} (expected 200)`);
    } else {
      console.log(`✓ /api/v1/me 200 in ${elapsed}ms`);
    }
    const json = await res.json();
    if (json?.data?.user?.username !== TEST_USERNAME) {
      fail(`Wrong username: ${JSON.stringify(json?.data?.user)}`);
    }
    if (!Array.isArray(json?.data?.token?.scopes)) {
      fail("token.scopes missing or not an array");
    }
    if (!json?.data?.token?.lastUsedAt) {
      fail("token.lastUsedAt not populated");
    }

    /* ── 2. last_used_at actually persisted in DB ── */
    const [refetched] = await db
      .select({ lastUsedAt: apiTokens.lastUsedAt })
      .from(apiTokens)
      .where(eq(apiTokens.id, tokenRow.id));
    if (!refetched.lastUsedAt) fail("DB last_used_at not bumped");

    /* ── 3. Bad token → 401 with problem+json ── */
    const bad = await fetch(`${BASE_URL}/api/v1/me`, {
      headers: { Authorization: "Bearer shn_pat_invalid" },
    });
    if (bad.status !== 401) fail(`bad token: ${bad.status} (expected 401)`);
    if (bad.headers.get("content-type")?.includes("problem+json") !== true) {
      fail(`bad token: missing problem+json content-type`);
    }
    console.log(`✓ Invalid token → 401 problem+json`);

    /* ── 4. No header → 401 ── */
    const noAuth = await fetch(`${BASE_URL}/api/v1/me`);
    if (noAuth.status !== 401) fail(`no header: ${noAuth.status} (expected 401)`);
    console.log(`✓ Missing Authorization → 401`);

    /* ── 5. Wrong scheme → 401 ── */
    const wrongScheme = await fetch(`${BASE_URL}/api/v1/me`, {
      headers: { Authorization: `Token ${plaintext}` },
    });
    if (wrongScheme.status !== 401) {
      fail(`wrong scheme: ${wrongScheme.status} (expected 401)`);
    }
    console.log(`✓ Non-Bearer scheme → 401`);

    /* ── 6. CORS preflight ── */
    const opt = await fetch(`${BASE_URL}/api/v1/me`, { method: "OPTIONS" });
    if (opt.status !== 204) fail(`OPTIONS: ${opt.status} (expected 204)`);
    if (opt.headers.get("access-control-allow-origin") !== "*") {
      fail(`OPTIONS: missing access-control-allow-origin: *`);
    }
    console.log(`✓ OPTIONS preflight returns CORS headers`);

    /* ── 7. Revoked token → 401 ── */
    await db
      .update(apiTokens)
      .set({ revokedAt: new Date() })
      .where(eq(apiTokens.id, tokenRow.id));
    const afterRevoke = await fetch(`${BASE_URL}/api/v1/me`, {
      headers: { Authorization: `Bearer ${plaintext}` },
    });
    if (afterRevoke.status !== 401) {
      fail(`after revoke: ${afterRevoke.status} (expected 401)`);
    }
    console.log(`✓ Revoked token → 401`);
  } finally {
    // Clean up the token row first (it cascades from the user delete too,
    // but explicit is cheaper than relying on FK semantics if anything
    // changed). Then delete the test user — CASCADE removes any audit
    // events tied to it.
    await db.delete(apiTokens).where(eq(apiTokens.id, tokenRow.id));
    await db.delete(users).where(eq(users.id, user.id));
    console.log(`✓ Test token + user deleted`);
  }

  if (!allPassed) {
    console.error(`\n✗ smoke test failed`);
    process.exit(1);
  }
  console.log(`\n✓ all checks passed`);
}

main().catch((err) => {
  console.error(`✗ smoke test crashed:`, err);
  process.exit(1);
});
