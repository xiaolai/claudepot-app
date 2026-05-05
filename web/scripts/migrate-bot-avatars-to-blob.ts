/**
 * One-shot: upload the 15 office-bot avatars to Vercel Blob and
 * rewrite users.image / users.avatar_url to point at the Blob URLs.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/migrate-bot-avatars-to-blob.ts [--dry-run]
 *
 * Source PNGs: ~/shannon-family/design-system/outputs/icons/<icon>_256.png
 * Destination: <BLOB_STORE>/avatars/<username>.png  (stable, no random suffix)
 *
 * Idempotent: re-uploading the same pathname overwrites in place
 * (allowOverwrite=true). Re-running this script re-uploads bytes and
 * re-writes the same URL into the row — a no-op net of HTTP cost.
 *
 * After this lands, `web/public/avatars/` is dead weight and should be
 * deleted in the same change-set.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { homedir } from "node:os";

import { put } from "@vercel/blob";
import { eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { users } from "@/db/schema";

// (botUsername, sourceIconBasename) — basename matches files in
// ~/shannon-family/design-system/outputs/icons/<basename>_256.png
const BOTS: Array<{ username: string; iconBase: string }> = [
  { username: "bonnie",  iconBase: "mom" },
  { username: "joe",     iconBase: "daddy" },
  { username: "ada",     iconBase: "ada" },
  { username: "alan",    iconBase: "alan" },
  { username: "blair",   iconBase: "blair" },
  { username: "byte",    iconBase: "byte" },
  { username: "delon",   iconBase: "delon" },
  { username: "laura",   iconBase: "laura" },
  { username: "loki",    iconBase: "loki" },
  { username: "nancy",   iconBase: "nancy" },
  { username: "selina",  iconBase: "selina" },
  { username: "shirley", iconBase: "shirley" },
  { username: "stephen", iconBase: "stephen" },
  { username: "warren",  iconBase: "warren" },
  { username: "wayne",   iconBase: "wayne" },
];

const ICON_DIR = resolve(homedir(), "shannon-family/design-system/outputs/icons");

function parseArgs(argv: string[]): { dryRun: boolean } {
  return { dryRun: argv.includes("--dry-run") };
}

async function main() {
  const { dryRun } = parseArgs(process.argv.slice(2));
  console.log(`> migrate-bot-avatars-to-blob — ${dryRun ? "DRY RUN" : "APPLY"}`);

  if (!process.env.BLOB_READ_WRITE_TOKEN) {
    throw new Error(
      "BLOB_READ_WRITE_TOKEN missing. Run `vercel env pull` in web/ first.",
    );
  }

  const probe = await db.execute(sql`SELECT current_database() AS db`);
  console.log(`> db = ${(probe.rows ?? probe)[0]?.db}`);

  type Result = {
    username: string;
    sourcePath: string;
    blobUrl: string | null;
    rowsUpdated: number;
  };
  const results: Result[] = [];

  for (const { username, iconBase } of BOTS) {
    const sourcePath = resolve(ICON_DIR, `${iconBase}_256.png`);
    const bytes = readFileSync(sourcePath);

    if (dryRun) {
      console.log(`  PLAN ${username.padEnd(8)} ${sourcePath} → avatars/${username}.png (${bytes.length}B)`);
      results.push({ username, sourcePath, blobUrl: null, rowsUpdated: 0 });
      continue;
    }

    const { url } = await put(`avatars/${username}.png`, bytes, {
      access: "public",
      contentType: "image/png",
      addRandomSuffix: false,
      allowOverwrite: true,
      cacheControlMaxAge: 60 * 60 * 24 * 365, // 1y; treat avatars as immutable per username
    });

    const updated = await db
      .update(users)
      .set({ image: url, avatarUrl: url, updatedAt: new Date() })
      .where(eq(users.username, username))
      .returning({ id: users.id });

    results.push({
      username,
      sourcePath,
      blobUrl: url,
      rowsUpdated: updated.length,
    });
    console.log(`  OK   ${username.padEnd(8)} → ${url}  (${updated.length} row${updated.length === 1 ? "" : "s"})`);
  }

  if (dryRun) {
    console.log(`> dry run complete; re-run without --dry-run to apply`);
    return;
  }

  const total = results.reduce((acc, r) => acc + r.rowsUpdated, 0);
  console.log(`> done — ${results.length} blobs uploaded, ${total} rows updated`);
}

main().catch((err) => {
  console.error("✗ migrate-bot-avatars-to-blob failed:", err);
  process.exit(1);
});
