/**
 * Generate + upload pixel-space-invader avatars for the seven reader
 * bots, then point users.image / users.avatar_url at the blob URLs.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-reader-bot-avatars.ts [--dry-run]
 *
 * Each invader is a 5×5 vertically-symmetric pixel grid generated
 * deterministically from sha256(username), so re-running this script
 * produces the same bytes for the same bot — idempotent on the blob
 * side (allowOverwrite=true) and on the row updates.
 *
 * Visual: classic space-invader silhouette. Vertical symmetry means
 * the right two columns mirror the left two; the middle column is
 * its own. ~60% fill probability lands roughly where the canonical
 * sprites do (more cells lit than dark). Color is a single vivid
 * hue per bot drawn from the seed.
 *
 * Storage: avatars/{username-without-@}.svg in Vercel Blob, public
 * read, immutable cache headers.
 */

import { createHash } from "node:crypto";
import { put } from "@vercel/blob";
import { eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { users } from "@/db/schema";

const READER_BOTS = [
  "mira@reader",
  "dax@reader",
  "kang@reader",
  "theo@reader",
  "iris@reader",
  "noor@reader",
  "robin@reader",
];

/** SVG dimension. 70px matches the bot avatar render size we use
 *  on /office/. crispEdges keeps the pixel grid pixel-true at any
 *  zoom level. */
const SIZE = 70;
const COLS = 5;
const ROWS = 5;
const FILL_THRESHOLD = 153; // bytes < 153 → cell ON; ≈60% fill rate

function deterministicByteStream(seed: string): () => number {
  // sha256 = 32 bytes; we re-use the digest cyclically. A 5×5
  // symmetric grid needs 15 coin flips + 1 byte for the hue, so 16
  // bytes is plenty — re-cycling is just defensive.
  const digest = createHash("sha256").update(seed).digest();
  let i = 0;
  return () => digest[i++ % digest.length];
}

function generateInvaderSvg(seed: string): string {
  const next = deterministicByteStream(seed);

  // Hue from the first byte, scaled to 0-360. Saturation/lightness
  // pinned for consistency across bots — vivid but not eye-searing.
  const hue = Math.floor((next() / 255) * 360);
  const fg = `hsl(${hue} 65% 48%)`;

  // Generate left half (cols 0,1,2). Cols 3,4 mirror cols 1,0.
  const grid: boolean[][] = [];
  for (let r = 0; r < ROWS; r++) {
    const row = new Array<boolean>(COLS).fill(false);
    for (let c = 0; c <= 2; c++) {
      row[c] = next() < FILL_THRESHOLD;
    }
    row[3] = row[1];
    row[4] = row[0];
    grid.push(row);
  }

  // Defensive: an all-off grid (probability ~1e-6) would be a blank
  // square. Force the center cell on so the avatar is always visible.
  const anyOn = grid.some((row) => row.some(Boolean));
  if (!anyOn) grid[Math.floor(ROWS / 2)][Math.floor(COLS / 2)] = true;

  const cell = SIZE / COLS;
  let rects = "";
  for (let r = 0; r < ROWS; r++) {
    for (let c = 0; c < COLS; c++) {
      if (grid[r][c]) {
        rects +=
          `<rect x="${c * cell}" y="${r * cell}"` +
          ` width="${cell}" height="${cell}" fill="${fg}"/>`;
      }
    }
  }

  return (
    `<svg xmlns="http://www.w3.org/2000/svg"` +
    ` width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}"` +
    ` shape-rendering="crispEdges">${rects}</svg>`
  );
}

function blobPathFor(username: string): string {
  // Vercel Blob paths accept @, but "@" in a URL path can be misread
  // by some HTTP clients as userinfo. Substitute @-with-hyphen for
  // the path; the DB row keeps the canonical username.
  return `avatars/${username.replace("@", "-")}.svg`;
}

async function main() {
  const dryRun = process.argv.includes("--dry-run");
  console.log(`> seed-reader-bot-avatars — ${dryRun ? "DRY RUN" : "APPLY"}`);

  if (!dryRun && !process.env.BLOB_READ_WRITE_TOKEN) {
    throw new Error(
      "BLOB_READ_WRITE_TOKEN missing. Run `vercel env pull` in web/ first.",
    );
  }

  const probe = await db.execute(sql`SELECT current_database() AS db`);
  // Drizzle's neon-serverless adapter returns {rows: [...]}; the
  // neon-http adapter returns the array directly. Accept either.
  const dbName =
    (probe as unknown as { rows?: Array<{ db: string }> }).rows?.[0]?.db ??
    (probe as unknown as Array<{ db: string }>)[0]?.db;
  console.log(`> db = ${dbName}`);

  for (const username of READER_BOTS) {
    const svg = generateInvaderSvg(username);
    const path = blobPathFor(username);

    if (dryRun) {
      console.log(`  PLAN ${username.padEnd(15)} → ${path}  (${svg.length}B)`);
      continue;
    }

    const { url } = await put(path, svg, {
      access: "public",
      contentType: "image/svg+xml",
      addRandomSuffix: false,
      allowOverwrite: true,
      cacheControlMaxAge: 60 * 60 * 24 * 365, // 1y; deterministic per username
    });

    const updated = await db
      .update(users)
      .set({ image: url, avatarUrl: url, updatedAt: new Date() })
      .where(eq(users.username, username))
      .returning({ id: users.id });

    console.log(
      `  OK   ${username.padEnd(15)} → ${url}  (${updated.length} row${updated.length === 1 ? "" : "s"})`,
    );
  }

  console.log(dryRun ? "> dry run complete" : "> done");
}

void main().then(() => process.exit(0));
