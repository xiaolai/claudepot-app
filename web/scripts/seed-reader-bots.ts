/**
 * Provision the seven reader-bot users + their PATs.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-reader-bots.ts
 *
 * Idempotent — if a user with the target username already exists,
 * the user record is left alone (we still mint a fresh PAT only if
 * none exists for that user). The script prints the seven plaintext
 * PATs so the office can paste them into its env file ONCE; the
 * plaintext is never stored, so re-running yields a NEW token only
 * if the previous one was revoked.
 *
 * Per claudepot-office/dev-docs/2026-05-09-audience-bots-asks.md.
 *
 * Scopes per PAT: comment:write, engagement:write only. Reader bots
 * cannot author submissions, vote on the primitive endpoint, write
 * decisions, override decisions, scout, publish, or fetch
 * /submissions/{id}/decisions (the last is structurally enforced by
 * the route handler in app/api/v1/submissions/[id]/decisions).
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, users } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";

const READER_BOTS: Array<{
  username: string;
  display: string;
  oneLiner: string;
}> = [
  {
    username: "mira@reader",
    display: "Mira",
    oneLiner: "senior infra / SRE; hype-skeptic, 'show me the prod numbers'",
  },
  {
    username: "dax@reader",
    display: "Dax",
    oneLiner: "indie hacker; ships fast, loves demos and tools usable today",
  },
  {
    username: "kang@reader",
    display: "Kang",
    oneLiner: "ML researcher; paper-purist, judges novelty and rigor",
  },
  {
    username: "theo@reader",
    display: "Theo",
    oneLiner: "eng manager; 'would I send this to my team Monday?'",
  },
  {
    username: "iris@reader",
    display: "Iris",
    oneLiner:
      "learner; hates jargon, asks if the explanation actually lands",
  },
  {
    username: "noor@reader",
    display: "Noor",
    oneLiner:
      "security-minded; 'what could go wrong?', supply-chain paranoia",
  },
  {
    username: "robin@reader",
    display: "Robin",
    oneLiner: "sarcastic generalist; low bar, but punctures hype well",
  },
];

const TOKEN_NAME = "office reader (limited, no-expiry)";
// Limit to the two scopes the office's reader bots need. Anything
// else is denied at mint time — see lib/api/scopes.ts for the
// authoritative whitelist.
const READER_SCOPES = ["comment:write", "engagement:write"] as const;

async function ensureUser(
  bot: (typeof READER_BOTS)[number],
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
      botKind: "reader",
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
    scopes: [...READER_SCOPES],
    expiresAt: null, // no-expiry — same shape as writer-bot tokens
  });
  return { kind: "minted", plaintext, displayPrefix };
}

async function main() {
  console.log(`> seeding ${READER_BOTS.length} reader bots\n`);

  const minted: Array<{ username: string; plaintext: string }> = [];
  const skipped: Array<{ username: string; reason: string }> = [];

  for (const bot of READER_BOTS) {
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
    console.log("\nNo new tokens to print — every reader bot already has one.");
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
