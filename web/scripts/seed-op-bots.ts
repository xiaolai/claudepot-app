/**
 * Provision op-bot users + their PATs.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-op-bots.ts
 *
 * Op bots are infra canaries / health probes / cost loggers — they
 * hold a single scope (bots:report) and never author content or
 * read other users' data. The first one is otto@daemon (per
 * claudepot-office/dev-docs/2026-05-08-bot-team-frameworks.md).
 *
 * Idempotent — if a user with the target username already exists,
 * the user record is left alone (we still mint a fresh PAT only if
 * none exists for that user). Plaintext PATs are printed once; the
 * polity stores only the SHA-256 digest.
 *
 * Companion file: scripts/refresh-bot-scopes.ts (catalog "op") —
 * keep OP_SCOPES there in lockstep with this file's scope set.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, users } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";

const OP_BOTS: Array<{
  username: string;
  display: string;
  oneLiner: string;
}> = [
  {
    username: "otto@daemon",
    display: "Otto",
    oneLiner:
      "polity canary — emits a heartbeat every ~60s; zero LLM calls, no state",
  },
];

const TOKEN_NAME = "office op (limited, no-expiry)";
// Canonical op-bot scope set. One scope. See
// scripts/refresh-bot-scopes.ts (OP_SCOPES) for the authoritative
// docs on what's granted vs permanently denied.
const OP_SCOPES = ["bots:report"] as const;

async function ensureUser(
  bot: (typeof OP_BOTS)[number],
): Promise<{ id: string; created: boolean }> {
  const existing = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, bot.username))
    .limit(1);
  if (existing.length > 0) {
    return { id: existing[0].id, created: false };
  }
  const [row] = await db
    .insert(users)
    .values({
      username: bot.username,
      name: bot.display,
      email: `${bot.username.replace("@", "+at+")}@bots.claudepot.local`,
      role: "system",
      isAgent: true,
      botKind: "op",
      bio: bot.oneLiner,
    })
    .returning({ id: users.id });
  return { id: row.id, created: true };
}

async function ensureToken(
  userId: string,
): Promise<
  | { kind: "minted"; plaintext: string; displayPrefix: string }
  | { kind: "exists"; displayPrefix: string }
> {
  const existing = await db
    .select({ id: apiTokens.id, displayPrefix: apiTokens.displayPrefix })
    .from(apiTokens)
    .where(eq(apiTokens.userId, userId))
    .limit(1);
  if (existing.length > 0) {
    return { kind: "exists", displayPrefix: existing[0].displayPrefix };
  }
  const { plaintext, hashed, displayPrefix } = generateToken();
  await db.insert(apiTokens).values({
    userId,
    name: TOKEN_NAME,
    displayPrefix,
    hashedSecret: hashed,
    scopes: [...OP_SCOPES],
    expiresAt: null,
  });
  return { kind: "minted", plaintext, displayPrefix };
}

async function main() {
  console.log(`> seeding ${OP_BOTS.length} op bot(s)\n`);

  const minted: Array<{ username: string; plaintext: string }> = [];
  const skipped: Array<{ username: string; reason: string }> = [];

  for (const bot of OP_BOTS) {
    const userResult = await ensureUser(bot);
    const tokenResult = await ensureToken(userResult.id);

    const userTag = userResult.created ? "NEW USER" : "EXISTING USER";
    if (tokenResult.kind === "minted") {
      console.log(
        `  [${userTag.padEnd(13)}] @${bot.username.padEnd(15)} → token ${tokenResult.displayPrefix}…`,
      );
      minted.push({ username: bot.username, plaintext: tokenResult.plaintext });
    } else {
      console.log(
        `  [${userTag.padEnd(13)}] @${bot.username.padEnd(15)} → existing token ${tokenResult.displayPrefix}… (skipped)`,
      );
      skipped.push({
        username: bot.username,
        reason: `existing PAT ${tokenResult.displayPrefix}…; revoke it via /admin/users to re-mint`,
      });
    }
  }

  if (minted.length === 0) {
    console.log("\nNo new tokens to print — every op bot already has one.");
    if (skipped.length > 0) {
      console.log("To rotate: revoke at /admin/users → re-run this script.");
    }
    return;
  }

  console.log("\n────────────────────────────────────────────────");
  console.log("PLAINTEXT TOKENS (shown once — save NOW)");
  console.log("────────────────────────────────────────────────");
  for (const m of minted) {
    console.log(`${m.username.padEnd(15)}  ${m.plaintext}`);
  }
  console.log("────────────────────────────────────────────────");
  console.log("Paste these into the office bot env files;");
  console.log("the plaintext is not stored on the polity side.");
}

void main().then(() => process.exit(0));
