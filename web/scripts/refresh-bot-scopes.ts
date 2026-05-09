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

// Reader-bot tokens (minted by scripts/seed-reader-bots.ts) have a
// SUBSET of the catalog, not the whole thing. Per the office memos
// 2026-05-09-audience-bots-asks.md and -reader-bots-scope-followup.md.
const READER_TARGET_TOKEN_NAME = "office reader (limited, no-expiry)";

/**
 * Canonical reader-bot scope set. Five scopes:
 *   - read:all          — fetch submissions / comments / threads to react to
 *   - comment:write     — post 1-3 sentence reactions
 *   - comment:update    — refine a past reaction (after seeing replies);
 *                         updatedAt bumps so revisions are auditable
 *   - engagement:write  — semantic vote-substitute kinds
 *   - notification:read — see replies for the discuss cadence + reactions
 *                         to a reader's own comments
 *
 * Permanently denied (do NOT add to this list without an office memo):
 *   - comment:delete    — readers can't erase miscalibration evidence;
 *                         that's the load-bearing feedback signal
 *   - vote:write        — primitive vote endpoint must not pollute
 *                         public voteCount with bot reactions
 *   - save:write        — irrelevant to a reader's role
 *   - submission:*      — readers don't author
 *   - decision:*        — readers don't gatekeep editorial decisions
 *   - scout:write       — not a writer/scout role
 *   - bots:report       — meta-monitoring is the office's job, not the bots'
 *
 * The /api/v1/submissions/{id}/decisions route additionally refuses
 * any PAT whose user has bot_kind='reader' (structural backstop for
 * the writer-reasoning contamination prevention).
 */
const READER_SCOPES: readonly Scope[] = [
  "read:all",
  "comment:write",
  "comment:update",
  "engagement:write",
  "notification:read",
];

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

async function loadBots(tokenName: string): Promise<BotRow[]> {
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
        eq(apiTokens.name, tokenName),
        eq(users.isAgent, true),
        isNull(apiTokens.revokedAt),
      ),
    );
  return rows.map((r) => ({ ...r, scopes: r.scopes as string[] }));
}

async function applyRefresh(
  rows: BotRow[],
  desiredScopes: readonly Scope[],
): Promise<number> {
  const desired = [...desiredScopes];
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

async function backfillAudit(
  rows: BotRow[],
  desiredScopes: readonly Scope[],
): Promise<number> {
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
  const desired = [...desiredScopes];
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

type Catalog = {
  label: string;
  tokenName: string;
  scopes: readonly Scope[];
};

const CATALOGS: Catalog[] = [
  {
    label: "writer",
    tokenName: TARGET_TOKEN_NAME,
    scopes: [...SCOPES],
  },
  {
    label: "reader",
    tokenName: READER_TARGET_TOKEN_NAME,
    scopes: READER_SCOPES,
  },
];

async function processCatalog(
  cat: Catalog,
  apply: boolean,
  backfill: boolean,
): Promise<{ drift: number; updated: number; backfilled: number }> {
  console.log(`\n> [${cat.label}] target catalog: [${cat.scopes.join(", ")}]`);
  const rows = await loadBots(cat.tokenName);
  if (rows.length === 0) {
    console.log(`> [${cat.label}] no matching bot tokens — skipping`);
    return { drift: 0, updated: 0, backfilled: 0 };
  }
  console.log(`> [${cat.label}] ${rows.length} bot token(s) under management`);

  let driftCount = 0;
  for (const row of rows) {
    const { toAdd, stale } = diffScopes(row.scopes, cat.scopes);
    if (toAdd.length === 0 && stale.length === 0) {
      console.log(`  · @${row.username} (${row.displayPrefix}…): up-to-date`);
      continue;
    }
    driftCount += 1;
    const adds = toAdd.length > 0 ? `+[${toAdd.join(", ")}]` : "";
    const removes = stale.length > 0 ? ` -[${stale.join(", ")}]` : "";
    console.log(`  · @${row.username} (${row.displayPrefix}…): ${adds}${removes}`);
  }

  let updated = 0;
  let backfilled = 0;
  if (apply) {
    updated = await applyRefresh(rows, cat.scopes);
    if (updated > 0)
      console.log(`> [${cat.label}] applied — ${updated} token(s) refreshed`);
  }
  if (backfill) {
    console.log(`> [${cat.label}] backfilling audit rows for already-current tokens…`);
    backfilled = await backfillAudit(rows, cat.scopes);
    console.log(
      backfilled === 0
        ? `> [${cat.label}] no backfill rows needed`
        : `> [${cat.label}] backfill complete — ${backfilled} audit row(s) inserted`,
    );
  }
  return { drift: driftCount, updated, backfilled };
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

  let totalDrift = 0;
  let totalUpdated = 0;
  for (const cat of CATALOGS) {
    const result = await processCatalog(cat, apply, backfill);
    totalDrift += result.drift;
    totalUpdated += result.updated;
  }

  if (!apply && totalDrift > 0) {
    console.log(
      `\n> dry-run: ${totalDrift} token(s) would update across catalogs — re-run with --apply`,
    );
  } else if (apply && totalUpdated === 0) {
    console.log("\n> all catalogs up-to-date — nothing applied");
  }
}

main().catch((err) => {
  console.error("✗ refresh-bot-scopes failed:", err);
  process.exit(1);
});
