# src/lib/social/

Cross-platform publishing for sha.com's marketing channels (Bluesky + X).

This is a publishing layer, not an editorial layer. Editorial logic (what to post, when, why) lives elsewhere; this directory only knows how to take a `{ text, url }` and put it on each platform safely.

## Why two adapters

Bluesky and X are fundamentally different in posture. Bluesky is an open federated protocol with free, generous API access. X is a closed paid API with hard rate limits. The adapters absorb that asymmetry so callers see one `publish(post, { platforms })` interface.

## Files

| Path | Purpose |
|---|---|
| `types.ts` | `Post`, `Platform`, `PublishResult`, `AuthCheckResult`. Discriminated union on `ok` so success / failure narrow cleanly. |
| `format.ts` | Per-platform truncation + URL-overhead accounting (X auto-shortens to 23 chars; Bluesky counts URL literally). |
| `bluesky.ts` | `publishToBluesky` + `verifyBlueskyAuth`. Uses `@atproto/api` `BskyAgent` and `RichText` (RichText auto-detects URL facets so links render as embeds). |
| `x.ts` | `publishToX` + `verifyXAuth`. Uses `twitter-api-v2`. |
| `publish.ts` | `publish(post, options)` + `verifyAuth(platforms)`. The orchestrator. Handles dry-run and parallel platform dispatch. |

## Required environment variables

In `.env.local` (see `.env.example` at the repo root for the canonical shape):

```bash
# Bluesky — free; create app password at https://bsky.app/settings/app-passwords
BLUESKY_HANDLE=sha.com
BLUESKY_APP_PASSWORD=

# X — paid API access required (Basic ≈ $200/mo). Skip until tier is provisioned.
# Create at https://developer.x.com/en/portal/dashboard with read+write scopes.
X_API_KEY=
X_API_SECRET=
X_ACCESS_TOKEN=
X_ACCESS_SECRET=
```

Missing credentials are not crashes — they cause the affected platform to return a `PublishResult` with `ok: false` and a message naming what's missing. Callers can still publish to platforms whose credentials are configured.

## Programmatic usage

```ts
import { publish } from "@/lib/social/publish";

const results = await publish(
  {
    text: "Two prompt strategies for legal review, evals shown, where each breaks.",
    url: "https://claudepot.com/post/123",
  },
  { platforms: ["bluesky", "x"] }
);

for (const r of results) {
  if (r.ok) console.log(`${r.platform} → ${r.url}${r.truncated ? " (truncated)" : ""}`);
  else console.error(`${r.platform} failed: ${r.error}`);
}
```

The `publish()` function never throws — every platform returns a `PublishResult` regardless of success. Callers decide what to do with partial failure.

## CLI

Two scripts wrap the library for hand-publishing and credential testing:

```bash
# Publish (defaults to bluesky-only since X needs paid API access)
pnpm social:post --text "your message" --url "https://example.com"
pnpm social:post --text "test" --platforms bsky,x --dry-run

# Verify credentials (no posts created)
pnpm social:test
pnpm social:test --platforms bsky
```

See `scripts/social-post.ts` and `scripts/social-test-auth.ts`.

## What this layer is NOT

- **Not the editorial layer.** It doesn't decide what to post or when. That's the agentic editorial team's job (see `editorial/`).
- **Not a scheduler.** No cron, no retry policy, no rate-limit backoff. Caller orchestrates.
- **Not a media uploader.** Text + URL only for v0. Image upload is a future addition (Bluesky `app.bsky.embed.images`, X `v1.uploadMedia` then attach).
- **Not a thread builder.** Single post per call. Threading lives in a future layer.
- **Not a reply / quote-post handler.** Both adapters could grow these; deferred until the use case is real.

## Safe-by-default choices

- Both adapters are **single-attempt** — no auto-retry on failure. Network errors surface immediately, callers decide policy.
- Truncation is **silent at the API call** but **visible in the result** (`truncated: true`). The CLI surfaces this; programmatic callers should check.
- Dry-run mode formats but does not authenticate or call platform APIs. Useful for verifying truncation and routing without spending API quota or risking accidental posts.
- The X adapter requires a fully scoped client (read + write). The verify endpoint (`v2.me`) is the cheapest read; verify before posting to catch credential drift early.

## Future shape

When the editorial pipeline lands, callers will look like:

```ts
// in some editorial cron handler:
import { publish } from "@/lib/social/publish";
import { getTopOfDayPick } from "@/db/queries";

const pick = await getTopOfDayPick();
const post = formatPick(pick); // editorial framing
await publish(post, { platforms: ["bluesky", "x"] });
```

That `formatPick` function is the editorial layer. It applies `audience.md` voice rules to produce the `text` field.
