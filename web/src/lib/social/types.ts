export type Platform = "bluesky" | "x";

export interface Post {
  text: string;
  url?: string;
}

export interface PublishOptions {
  platforms: Platform[];
  dryRun?: boolean;
}

export type PublishResult =
  | { platform: Platform; ok: true; url: string; truncated: boolean }
  | { platform: Platform; ok: false; error: string };

export interface AuthCheckResult {
  platform: Platform;
  ok: boolean;
  identity?: string;
  error?: string;
}
