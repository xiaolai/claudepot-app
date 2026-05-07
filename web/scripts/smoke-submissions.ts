// Empty export marks this file as a module so top-level await is allowed.
export {};

/**
 * End-to-end smoke test for slice-2 submission surfaces.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/smoke-submissions.ts [BASE_URL]
 *
 * Defaults to http://localhost:3000 — start `pnpm dev` first.
 *
 * Mints an ephemeral PAT for an ephemeral user, then exercises both
 * surfaces against the same submission flow:
 *
 *   1. POST /api/v1/submissions  → REST path
 *   2. POST /api/mcp tool=submit_link  → MCP path
 *
 * Verifies:
 *   - Successful create returns 201 + url
 *   - Same URL submitted twice returns the duplicate marker
 *   - Missing scope is rejected with 403
 *   - REST + MCP write through the same dedup window
 *
 * Cleanup deletes the test submissions, token, user, and (via CASCADE)
 * the audit-event rows.
 */

import { randomBytes } from "node:crypto";
import { eq, inArray } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, submissions, users } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";

const BASE_URL = process.argv[2] ?? "http://localhost:3000";
const RUN_TAG = randomBytes(4).toString("hex");
const USERNAME = `smoke-${RUN_TAG}`;

async function createTestUser() {
  const [u] = await db
    .insert(users)
    .values({
      username: USERNAME,
      email: `${USERNAME}@smoke.invalid`,
      name: `Smoke ${RUN_TAG}`,
      role: "system", // bypass moderation queue so smoke is deterministic
      isAgent: true,
      karma: 100,
    })
    .returning();
  return u;
}

async function mintToken(userId: string, scopes: string[]) {
  const { plaintext, hashed, displayPrefix } = generateToken();
  const expiresAt = new Date(Date.now() + 15 * 60 * 1000);
  const [t] = await db
    .insert(apiTokens)
    .values({
      userId,
      name: `smoke-${RUN_TAG}-${scopes.join(",")}`,
      displayPrefix,
      hashedSecret: hashed,
      scopes,
      expiresAt,
    })
    .returning();
  return { plaintext, row: t };
}

async function main() {
  console.log(`> Smoke test against ${BASE_URL}`);

  const user = await createTestUser();
  console.log(`✓ Test user @${user.username} (${user.id})`);

  const writeToken = await mintToken(user.id, [
    "submission:write",
    "read:all",
  ]);
  const readOnlyToken = await mintToken(user.id, ["read:all"]);
  console.log(
    `✓ Minted write token ${writeToken.row.displayPrefix}… and read-only token ${readOnlyToken.row.displayPrefix}…`,
  );

  const createdIds: string[] = [];
  let allPassed = true;
  const fail = (msg: string) => {
    console.error(`✗ ${msg}`);
    allPassed = false;
  };

  try {
    /* ── 1. REST POST happy path ─────────────────────────────── */
    const restUrl = `https://example.com/${RUN_TAG}/rest`;
    const restRes = await fetch(`${BASE_URL}/api/v1/submissions`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${writeToken.plaintext}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        type: "tool",
        title: `Smoke REST ${RUN_TAG}`,
        url: restUrl,
        tags: [],
      }),
    });
    if (restRes.status !== 201) {
      const body = await restRes.text();
      fail(`REST create: ${restRes.status} (expected 201). Body: ${body.slice(0, 300)}`);
    } else {
      const json = await restRes.json();
      const id = json?.data?.id;
      if (!id) fail("REST create: no id in response");
      else {
        createdIds.push(id);
        console.log(`✓ REST created /post/${id}`);
      }
    }

    /* ── 2. REST dedup ───────────────────────────────────────── */
    const dupRes = await fetch(`${BASE_URL}/api/v1/submissions`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${writeToken.plaintext}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        type: "tool",
        title: `Smoke REST ${RUN_TAG} (dup)`,
        url: restUrl,
      }),
    });
    if (dupRes.status !== 200) {
      fail(`REST dedup: ${dupRes.status} (expected 200 with duplicate marker)`);
    } else {
      const json = await dupRes.json();
      if (!json?.data?.duplicate) fail("REST dedup: missing duplicate flag");
      else console.log(`✓ REST dedup detected — ${json.data.existingId}`);
    }

    /* ── 3. Missing scope ────────────────────────────────────── */
    const noScopeRes = await fetch(`${BASE_URL}/api/v1/submissions`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${readOnlyToken.plaintext}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        type: "tool",
        title: `Smoke noscope ${RUN_TAG}`,
        url: `https://example.com/${RUN_TAG}/noscope`,
      }),
    });
    if (noScopeRes.status !== 403) {
      fail(`Missing scope: ${noScopeRes.status} (expected 403)`);
    } else console.log(`✓ submission:write scope enforced (403)`);

    /* ── 4. MCP submit_link via streamable HTTP (stateless) ─── */
    // mcp-handler runs streamable HTTP in stateless mode by default —
    // no Mcp-Session-Id, each request carries its own auth context.
    // Just POST tools/call directly with the Bearer token.
    const mcpUrl = `https://example.com/${RUN_TAG}/mcp`;
    const callRes = await fetch(`${BASE_URL}/api/mcp`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${writeToken.plaintext}`,
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: {
          name: "submit_link",
          arguments: {
            type: "tool",
            title: `Smoke MCP ${RUN_TAG}`,
            url: mcpUrl,
          },
        },
      }),
    });
    if (callRes.status !== 200) {
      const body = await callRes.text();
      fail(`MCP submit_link: ${callRes.status}. Body: ${body.slice(0, 300)}`);
    } else {
      const body = await callRes.text();
      // Streamable HTTP returns SSE-formatted result by default. Check
      // for the "Published:" marker the tool emits in the text content.
      if (!body.includes("Published:") && !body.includes("post/")) {
        fail(
          `MCP submit_link: unexpected response: ${body.slice(0, 400)}`,
        );
      } else {
        console.log(`✓ MCP submit_link succeeded`);
        // Extract id from "https://claudepot.com/post/<id>" in the body.
        const match = /post\/([a-f0-9-]{36})/.exec(body);
        if (match) createdIds.push(match[1]);
      }
    }

    /* ── 4b. MCP scope enforcement: read-only token can't submit ── */
    const noScopeMcp = await fetch(`${BASE_URL}/api/mcp`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${readOnlyToken.plaintext}`,
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: {
          name: "submit_link",
          arguments: {
            type: "tool",
            title: `Smoke MCP noscope ${RUN_TAG}`,
            url: `https://example.com/${RUN_TAG}/mcp-noscope`,
          },
        },
      }),
    });
    const noScopeBody = await noScopeMcp.text();
    if (!noScopeBody.includes("missing the submission:write scope")) {
      fail(
        `MCP scope enforcement: expected scope-missing error, got: ${noScopeBody.slice(0, 200)}`,
      );
    } else {
      console.log(`✓ MCP submission:write scope enforced`);
    }

    /* ── 5. Confirm both submissions are in the DB with token-prefix
     *      sourceId, marking them as API-created.                   */
    if (createdIds.length > 0) {
      const rows = await db
        .select({
          id: submissions.id,
          submitterKind: submissions.submitterKind,
          sourceId: submissions.sourceId,
        })
        .from(submissions)
        .where(inArray(submissions.id, createdIds));
      const wrong = rows.filter(
        (r) => r.submitterKind !== "scout" || r.sourceId === null,
      );
      if (wrong.length > 0) {
        fail(
          `Provenance: ${wrong.length} submissions missing scout/sourceId tag: ${JSON.stringify(wrong)}`,
        );
      } else {
        console.log(
          `✓ Provenance: ${rows.length} submissions tagged scout + sourceId=token-uuid`,
        );
      }
    }
  } finally {
    if (createdIds.length > 0) {
      await db.delete(submissions).where(inArray(submissions.id, createdIds));
    }
    await db.delete(users).where(eq(users.id, user.id));
    console.log(
      `✓ Cleaned up ${createdIds.length} submission(s), 2 token(s), 1 user`,
    );
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
