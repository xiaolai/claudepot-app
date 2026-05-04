import { TwitterApi } from "twitter-api-v2";
import type { Post, PublishResult, AuthCheckResult } from "./types";
import { formatForX } from "./format";

const MISSING = "Missing X_API_KEY / X_API_SECRET / X_ACCESS_TOKEN / X_ACCESS_SECRET in .env.local";

function makeClient(): TwitterApi | null {
  const { X_API_KEY, X_API_SECRET, X_ACCESS_TOKEN, X_ACCESS_SECRET } = process.env;
  if (!X_API_KEY || !X_API_SECRET || !X_ACCESS_TOKEN || !X_ACCESS_SECRET) return null;
  return new TwitterApi({
    appKey: X_API_KEY,
    appSecret: X_API_SECRET,
    accessToken: X_ACCESS_TOKEN,
    accessSecret: X_ACCESS_SECRET,
  });
}

export async function publishToX(post: Post): Promise<PublishResult> {
  const client = makeClient();
  if (!client) return { platform: "x", ok: false, error: MISSING };
  try {
    const formatted = formatForX(post);
    const result = await client.v2.tweet({ text: formatted.text });
    return {
      platform: "x",
      ok: true,
      url: `https://x.com/i/web/status/${result.data.id}`,
      truncated: formatted.truncated,
    };
  } catch (err) {
    return {
      platform: "x",
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

export async function verifyXAuth(): Promise<AuthCheckResult> {
  const client = makeClient();
  if (!client) return { platform: "x", ok: false, error: MISSING };
  try {
    const me = await client.v2.me();
    return { platform: "x", ok: true, identity: `@${me.data.username}` };
  } catch (err) {
    return {
      platform: "x",
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}
