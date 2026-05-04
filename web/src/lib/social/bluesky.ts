import { BskyAgent, RichText } from "@atproto/api";
import type { Post, PublishResult, AuthCheckResult } from "./types";
import { formatForBluesky } from "./format";

const BLUESKY_SERVICE = "https://bsky.social";

function readCredentials(): { handle: string; password: string } | string {
  const handle = process.env.BLUESKY_HANDLE;
  const password = process.env.BLUESKY_APP_PASSWORD;
  if (!handle || !password) {
    return "Missing BLUESKY_HANDLE or BLUESKY_APP_PASSWORD in .env.local";
  }
  return { handle, password };
}

export async function publishToBluesky(post: Post): Promise<PublishResult> {
  const creds = readCredentials();
  if (typeof creds === "string") {
    return { platform: "bluesky", ok: false, error: creds };
  }

  const agent = new BskyAgent({ service: BLUESKY_SERVICE });
  try {
    await agent.login({ identifier: creds.handle, password: creds.password });
    const formatted = formatForBluesky(post);
    const rt = new RichText({ text: formatted.text });
    await rt.detectFacets(agent);
    const result = await agent.post({ text: rt.text, facets: rt.facets });
    const rkey = result.uri.split("/").pop() ?? "";
    return {
      platform: "bluesky",
      ok: true,
      url: `https://bsky.app/profile/${creds.handle}/post/${rkey}`,
      truncated: formatted.truncated,
    };
  } catch (err) {
    return {
      platform: "bluesky",
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

export async function verifyBlueskyAuth(): Promise<AuthCheckResult> {
  const creds = readCredentials();
  if (typeof creds === "string") {
    return { platform: "bluesky", ok: false, error: creds };
  }
  const agent = new BskyAgent({ service: BLUESKY_SERVICE });
  try {
    const session = await agent.login({ identifier: creds.handle, password: creds.password });
    return { platform: "bluesky", ok: true, identity: `@${session.data.handle}` };
  } catch (err) {
    return {
      platform: "bluesky",
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}
