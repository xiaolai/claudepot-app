/**
 * Publish a post to Bluesky and/or X.
 *
 *   pnpm social:post --text "..." [--url "..."] [--platforms bsky,x] [--dry-run]
 *
 * Defaults to bluesky-only since X requires paid API access.
 * Run with `--dry-run` to verify formatting + truncation without posting.
 */

import { parseArgs } from "node:util";
import { publish } from "@/lib/social/publish";
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
    text: { type: "string", short: "t" },
    url: { type: "string", short: "u" },
    platforms: { type: "string", short: "p", default: "bsky" },
    "dry-run": { type: "boolean", default: false },
  },
});

if (!values.text) {
  console.error("✗ --text required.\n  pnpm social:post --text \"...\" [--url \"...\"] [--platforms bsky,x] [--dry-run]");
  process.exit(2);
}

const platforms = parsePlatforms(values.platforms!);

const results = await publish(
  { text: values.text, url: values.url },
  { platforms, dryRun: values["dry-run"] }
);

let anyFailed = false;
for (const r of results) {
  if (r.ok) {
    const note = r.truncated ? " (truncated)" : "";
    const prefix = values["dry-run"] ? "[dry-run] " : "";
    console.log(`✓ ${prefix}${r.platform}: ${r.url}${note}`);
  } else {
    console.error(`✗ ${r.platform}: ${r.error}`);
    anyFailed = true;
  }
}

process.exit(anyFailed ? 1 : 0);
