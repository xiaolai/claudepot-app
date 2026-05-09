/**
 * Generate + upload pixel-space-invader avatars for the seven reader
 * bots, then point users.image / users.avatar_url at the blob URLs.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-reader-bot-avatars.ts [--dry-run]
 *
 * Each invader is a 32×32 vertically-symmetric pixel grid generated
 * deterministically from sha256(username) and rendered at 512×512
 * (each grid cell = 16×16 px in the output). Re-running this script
 * produces the same bytes for the same bot — idempotent on the blob
 * side (allowOverwrite=true) and on the row updates.
 *
 * Visual: 32×32 art resolution scaled 16× for crisp display. Cols
 * 16-31 mirror cols 0-15 → vertical symmetry. ~40% fill probability
 * — dense enough to read as a body, sparse enough to avoid pixel-
 * noise static at high resolution. Color is a single vivid hue per
 * bot drawn from the seed.
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

/** Pixel-art resolution (the actual sprite grid) and SVG render
 *  size. 32×32 art scaled 16× for a crisp 512×512 output —
 *  shape-rendering="crispEdges" keeps each grid cell pixel-true at
 *  any zoom. COLS must be even for the symmetry mirror to work
 *  without a dedicated middle column. */
const SIZE = 512;
const COLS = 32;
const ROWS = 32;
const CELL = SIZE / COLS; // 16
const FILL_THRESHOLD = 102; // bytes < 102 → cell ON; ≈40% fill rate
//                          //   higher fill rates produced pixel-
//                          //   noise static at this resolution; 40%
//                          //   keeps the silhouette readable

function deterministicByteStream(seed: string): () => number {
  // 32×32 symmetric grid needs (16 cols × 32 rows) = 512 coin flips
  // plus 1 byte for the hue. SHA-256 is 32 bytes — not enough alone.
  // Chain by hashing the previous block: b0=sha256(seed),
  // b_{n+1}=sha256(b_n). 32 chained blocks = 1024 bytes, >= 513.
  // Same byte stream every time for the same seed.
  const blocks: Uint8Array[] = [];
  let prev = createHash("sha256").update(seed).digest();
  blocks.push(new Uint8Array(prev));
  for (let n = 0; n < 31; n++) {
    prev = createHash("sha256").update(prev).digest();
    blocks.push(new Uint8Array(prev));
  }
  // Concat to one buffer for fast indexed access.
  const total = blocks.reduce((a, b) => a + b.length, 0);
  const bytes = new Uint8Array(total);
  let off = 0;
  for (const b of blocks) {
    bytes.set(b, off);
    off += b.length;
  }
  let i = 0;
  return () => bytes[i++ % bytes.length];
}

function generateInvaderSvg(seed: string): string {
  const next = deterministicByteStream(seed);

  // Hue from the first byte, scaled to 0-360. Saturation/lightness
  // pinned for consistency across bots — vivid but not eye-searing.
  const hue = Math.floor((next() / 255) * 360);
  const fg = `hsl(${hue} 65% 48%)`;

  // Generate the left half (cols 0..15); cols 16..31 mirror cols
  // 15..0 → vertical symmetry across the central seam.
  const half = COLS / 2;
  const grid: boolean[][] = [];
  for (let r = 0; r < ROWS; r++) {
    const row = new Array<boolean>(COLS).fill(false);
    for (let c = 0; c < half; c++) {
      row[c] = next() < FILL_THRESHOLD;
    }
    for (let c = half; c < COLS; c++) {
      row[c] = row[COLS - 1 - c];
    }
    grid.push(row);
  }

  // Defensive: an all-off grid (probability ~10^-220 at this size)
  // would be a blank square. Force a 2×2 center block on so the
  // avatar is always visible.
  const anyOn = grid.some((row) => row.some(Boolean));
  if (!anyOn) {
    const cr = Math.floor(ROWS / 2);
    const cc = Math.floor(COLS / 2);
    grid[cr - 1][cc - 1] = true;
    grid[cr - 1][cc] = true;
    grid[cr][cc - 1] = true;
    grid[cr][cc] = true;
  }

  let rects = "";
  for (let r = 0; r < ROWS; r++) {
    for (let c = 0; c < COLS; c++) {
      if (grid[r][c]) {
        rects +=
          `<rect x="${c * CELL}" y="${r * CELL}"` +
          ` width="${CELL}" height="${CELL}" fill="${fg}"/>`;
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
