/**
 * Grant each office bot's PAT the full current scope catalog, with audit.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/refresh-bot-scopes.ts \
 *     [--apply] [--backfill-existing]
 *
 * Default is dry-run; --apply commits the UPDATEs and inserts a
 * `scope_change` audit row per affected token. --backfill-existing
 * inserts a single `scope_change` audit row per office bot that
 * already matches the catalog AND has no prior `scope_change` event,
 * so the 2026-05-06 refresh (run before this audit variant existed)
 * leaves a trail.
 *
 * Why: bot tokens are minted by seed-office-bots.ts with `scopes: [...SCOPES]`
 * — i.e. whatever the catalog held at mint time. When the catalog grows
 * (e.g. submission:delete + comment:delete added 2026-05-06), pre-existing
 * bot rows do NOT get the new scopes automatically; the rate-limit/scope
 * column on each api_tokens row is just a text[] snapshot. This script
 * brings each bot's row back in sync without re-minting (so .env.office
 * PATs keep working), and records the change in api_token_events.
 *
 * Idempotent — re-running on already-current rows is a no-op, and the
 * backfill skips tokens that already have a scope_change event.
 *
 * Required env: DATABASE_URL (or NEON_DATABASE_URL).
 */

import { and, eq, isNull } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, apiTokenEvents, users } from "@/db/schema";
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

type BotRow = {
  id: string;
  userId: string;
  displayPrefix: string;
  scopes: string[];
  username: string;
};

async function loadBots(): Promise<BotRow[]> {
  const rows = await db
    .select({
      id: apiTokens.id,
      userId: apiTokens.userId,
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
  return rows.map((r) => ({ ...r, scopes: r.scopes as string[] }));
}

async function applyRefresh(rows: BotRow[]): Promise<number> {
  const desired = [...SCOPES];
  const stalebots = rows.filter((row) => {
    const { toAdd, stale } = diffScopes(row.scopes, desired);
    return toAdd.length > 0 || stale.length > 0;
  });

  if (stalebots.length === 0) {
    console.log("> all bots already on the latest scope catalog");
    return 0;
  }

  for (const row of stalebots) {
    const { toAdd, stale } = diffScopes(row.scopes, desired);
    const previous = row.scopes;
    await db
      .update(apiTokens)
      .set({ scopes: desired })
      .where(eq(apiTokens.id, row.id));
    // Audit row carries the diff so a future reader can reconstruct
    // what changed without re-running git blame on lib/api/scopes.ts.
    await db.insert(apiTokenEvents).values({
      tokenId: row.id,
      userId: row.userId,
      event: "scope_change",
      scopes: desired,
      metadata: {
        previousScopes: previous,
        added: toAdd,
        removed: stale,
        source: "refresh-bot-scopes",
      },
    });
    console.log(`  ✓ refreshed @${row.username}`);
  }
  return stalebots.length;
}

async function backfillAudit(rows: BotRow[]): Promise<number> {
  // For each bot whose scopes already match the catalog AND has no
  // prior scope_change event, insert one backfill row. The 2026-05-06
  // refresh predates the scope_change enum variant, so without this
  // backfill the trail for that production change is missing.
  //
  // The NOT-EXISTS guard is a separate SELECT (rather than a single
  // INSERT … WHERE NOT EXISTS) because Neon's HTTP driver mis-casts
  // the JS scopes array to a record type when the INSERT pulls it
  // through a SELECT. Two queries per token is fine — N=15 bots.
  let inserted = 0;
  const desired = [...SCOPES];
  const desiredKey = [...desired].sort().join(",");
  for (const row of rows) {
    const currentKey = [...row.scopes].sort().join(",");
    if (currentKey !== desiredKey) {
      // Drift — not a candidate for backfill; --apply will pick this
      // up and emit a forward audit row instead.
      continue;
    }
    const [existing] = await db
      .select({ id: apiTokenEvents.id })
      .from(apiTokenEvents)
      .where(
        and(
          eq(apiTokenEvents.tokenId, row.id),
          eq(apiTokenEvents.event, "scope_change"),
        ),
      )
      .limit(1);
    if (existing) {
      console.log(`  · @${row.username}: scope_change row already present`);
      continue;
    }
    await db.insert(apiTokenEvents).values({
      tokenId: row.id,
      userId: row.userId,
      event: "scope_change",
      scopes: desired,
      metadata: {
        source: "refresh-bot-scopes",
        note: "backfill — pre-enum scope_change refresh on or before 2026-05-06",
        scopesAtBackfillTime: desired,
      },
    });
    inserted += 1;
    console.log(`  ✓ backfilled audit for @${row.username}`);
  }
  return inserted;
}

async function main() {
  const apply = process.argv.includes("--apply");
  const backfill = process.argv.includes("--backfill-existing");
  if (!apply && !backfill) {
    console.log(`> refresh-bot-scopes — dry-run`);
  } else {
    const modes = [apply && "APPLY", backfill && "BACKFILL"]
      .filter(Boolean)
      .join("+");
    console.log(`> refresh-bot-scopes — ${modes}`);
  }
  console.log(`> target catalog: [${[...SCOPES].join(", ")}]`);

  const rows = await loadBots();
  if (rows.length === 0) {
    console.log("> no matching bot tokens — nothing to do");
    return;
  }
  console.log(`> ${rows.length} bot token(s) under management`);

  // Always print the per-bot diff so dry-run is informative.
  const desired = [...SCOPES];
  let driftCount = 0;
  for (const row of rows) {
    const { toAdd, stale } = diffScopes(row.scopes, desired);
    if (toAdd.length === 0 && stale.length === 0) {
      console.log(`  · @${row.username} (${row.displayPrefix}…): up-to-date`);
      continue;
    }
    driftCount += 1;
    const adds = toAdd.length > 0 ? `+[${toAdd.join(", ")}]` : "";
    const removes = stale.length > 0 ? ` -[${stale.join(", ")}]` : "";
    console.log(`  · @${row.username} (${row.displayPrefix}…): ${adds}${removes}`);
  }

  if (apply) {
    const updated = await applyRefresh(rows);
    if (updated > 0) console.log(`> applied — ${updated} token(s) refreshed`);
  } else if (driftCount > 0) {
    console.log(
      `> dry-run: ${driftCount} token(s) would update — re-run with --apply`,
    );
  }

  if (backfill) {
    console.log("> backfilling audit rows for already-current tokens…");
    const inserted = await backfillAudit(rows);
    console.log(
      inserted === 0
        ? "> no backfill rows needed"
        : `> backfill complete — ${inserted} audit row(s) inserted`,
    );
  }
}

main().catch((err) => {
  console.error("✗ refresh-bot-scopes failed:", err);
  process.exit(1);
});
