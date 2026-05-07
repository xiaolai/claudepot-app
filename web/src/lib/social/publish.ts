import type { Post, Platform, PublishOptions, PublishResult, AuthCheckResult } from "./types";
import { publishToBluesky, verifyBlueskyAuth } from "./bluesky";
import { publishToX, verifyXAuth } from "./x";
import { formatForBluesky, formatForX } from "./format";

const PUBLISHERS: Record<Platform, (post: Post) => Promise<PublishResult>> = {
  bluesky: publishToBluesky,
  x: publishToX,
};

const VERIFIERS: Record<Platform, () => Promise<AuthCheckResult>> = {
  bluesky: verifyBlueskyAuth,
  x: verifyXAuth,
};

export async function publish(post: Post, options: PublishOptions): Promise<PublishResult[]> {
  if (options.dryRun) {
    return options.platforms.map((platform) => {
      const formatted = platform === "bluesky" ? formatForBluesky(post) : formatForX(post);
      return {
        platform,
        ok: true,
        url: `dry-run://${platform}/${encodeURIComponent(formatted.text.slice(0, 30))}`,
        truncated: formatted.truncated,
      };
    });
  }
  return Promise.all(options.platforms.map((p) => PUBLISHERS[p](post)));
}

export async function verifyAuth(platforms: Platform[]): Promise<AuthCheckResult[]> {
  return Promise.all(platforms.map((p) => VERIFIERS[p]()));
}

export type { Post, Platform, PublishOptions, PublishResult, AuthCheckResult } from "./types";
