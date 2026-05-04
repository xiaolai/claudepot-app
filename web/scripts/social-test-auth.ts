/**
 * Verify credentials for Bluesky and/or X without publishing anything.
 *
 *   pnpm social:test                     # check both
 *   pnpm social:test --platforms bsky    # only Bluesky
 *
 * Exits 0 only if every requested platform with configured credentials
 * authenticates successfully. Platforms with missing credentials report
 * that as a non-zero exit.
 */

import { parseArgs } from "node:util";
import { verifyAuth } from "@/lib/social/publish";
import type { Platform } from "@/lib/social/types";

const PLATFORM_ALIASES: Record<string, Platform> = {
  bsky: "bluesky",
  bluesky: "bluesky",
  x: "x",
  twitter: "x",
};

function parsePlatforms(raw: string): Platform[] {
  const seen = new Set<Platform>();
  for (const token of raw.split(",")) {
    const norm = PLATFORM_ALIASES[token.trim().toLowerCase()];
    if (!norm) {
      console.error(`✗ unknown platform "${token.trim()}". Use: bsky, x.`);
      process.exit(2);
    }
    seen.add(norm);
  }
  return [...seen];
}

const { values } = parseArgs({
  options: {
    platforms: { type: "string", short: "p", default: "bsky,x" },
  },
});

const platforms = parsePlatforms(values.platforms!);
const results = await verifyAuth(platforms);

let anyFailed = false;
for (const r of results) {
  if (r.ok) {
    console.log(`✓ ${r.platform}: authenticated as ${r.identity}`);
  } else {
    console.error(`✗ ${r.platform}: ${r.error}`);
    anyFailed = true;
  }
}

process.exit(anyFailed ? 1 : 0);
