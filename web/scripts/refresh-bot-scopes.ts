/**
 * Grant each office bot's PAT the full current scope catalog.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/refresh-bot-scopes.ts [--apply]
 *
 * Default is dry-run; --apply commits the UPDATEs.
 *
 * Why: bot tokens are minted by seed-office-bots.ts with `scopes: [...SCOPES]`
 * — i.e. whatever the catalog held at mint time. When the catalog grows
 * (e.g. submission:delete + comment:delete added 2026-05-06), pre-existing
 * bot rows do NOT get the new scopes automatically; the rate-limit/scope
 * column on each api_tokens row is just a text[] snapshot. This script
 * brings each bot's row back in sync without re-minting (so .env.office
 * PATs keep working).
 *
 * Audit trail: api_token_events.event is a closed enum {mint, revoke}.
 * Until/unless a `scope_change` variant lands, this script logs to
 * stdout instead of inserting an audit row. The git history of this
 * script + lib/api/scopes.ts is the recoverable trail.
 *
 * Idempotent — re-running on already-current rows is a no-op.
 *
 * Required env: DATABASE_URL (or NEON_DATABASE_URL).
 */

import { and, eq, isNull } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, users } from "@/db/schema";
import { SCOPES, type Scope } from "@/lib/api/scopes";

// Mint name used by scripts/seed-office-bots.ts. Filtering on it scopes
// this script tightly to the office-bot fleet — user-minted PATs (which
// carry user-chosen scopes) are deliberately untouched.
const TARGET_TOKEN_NAME = "office (full access, no-expiry)";

function diffScopes(current: readonly string[], desired: readonly Scope[]) {
  const currentSet: Set<string> = new Set(current);
  const desiredSet: Set<string> = new Set(desired);
  const toAdd = desired.filter((s) => !currentSet.has(s));
  const stale = current.filter((s) => !desiredSet.has(s));
  return { toAdd, stale };
}

async function main() {
  const apply = process.argv.includes("--apply");
  console.log(`> refresh-bot-scopes — ${apply ? "APPLY" : "dry-run"}`);
  console.log(`> target catalog: [${[...SCOPES].join(", ")}]`);

  const rows = await db
    .select({
      id: apiTokens.id,
      displayPrefix: apiTokens.displayPrefix,
      scopes: apiTokens.scopes,
      username: users.username,
    })
    .from(apiTokens)
    .innerJoin(users, eq(users.id, apiTokens.userId))
    .where(
      and(
        eq(apiTokens.name, TARGET_TOKEN_NAME),
        eq(users.isAgent, true),
        isNull(apiTokens.revokedAt),
      ),
    );

  if (rows.length === 0) {
    console.log("> no matching bot tokens — nothing to do");
    return;
  }
  console.log(`> ${rows.length} bot token(s) under management`);

  const desired = [...SCOPES];
  const stalebots: typeof rows = [];
  for (const row of rows) {
    const { toAdd, stale } = diffScopes(row.scopes as string[], desired);
    if (toAdd.length === 0 && stale.length === 0) {
      console.log(`  · @${row.username} (${row.displayPrefix}…): up-to-date`);
      continue;
    }
    stalebots.push(row);
    const adds = toAdd.length > 0 ? `+[${toAdd.join(", ")}]` : "";
    const removes = stale.length > 0 ? ` -[${stale.join(", ")}]` : "";
    console.log(`  · @${row.username} (${row.displayPrefix}…): ${adds}${removes}`);
  }

  if (stalebots.length === 0) {
    console.log("> all bots already on the latest scope catalog");
    return;
  }
  if (!apply) {
    console.log(
      `> dry-run: ${stalebots.length} token(s) would update — re-run with --apply`,
    );
    return;
  }

  for (const row of stalebots) {
    await db
      .update(apiTokens)
      .set({ scopes: desired })
      .where(eq(apiTokens.id, row.id));
    console.log(`  ✓ refreshed @${row.username}`);
  }
  console.log(`> applied — ${stalebots.length} token(s) refreshed`);
}

main().catch((err) => {
  console.error("✗ refresh-bot-scopes failed:", err);
  process.exit(1);
});
