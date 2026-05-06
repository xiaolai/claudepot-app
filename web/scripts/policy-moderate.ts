/**
 * Smoke-test the AI policy moderator against arbitrary text.
 *
 *   pnpm tsx --env-file=.env.local scripts/policy-moderate.ts <kind> "<title>" "<body>"
 *
 *     <kind>  - "submission" | "comment"
 *     <title> - submission title (use "" for comments — they have no title)
 *     <body>  - the content to moderate
 *
 * Calls moderate() directly with the OPENAI_API_KEY from
 * web/.env.local. Prints the verdict, category, confidence,
 * one-line-why, and the estimated cost. Does NOT write to
 * policy_decisions or moderation_log — this is a calibration
 * tool, not a live moderation path.
 *
 * Examples (from web/):
 *
 *   pnpm tsx --env-file=.env.local scripts/policy-moderate.ts submission \
 *     "Tutorial: prompt patterns for legal review" \
 *     "Step 1: gather the contract. Step 2: ..."
 *
 *   pnpm tsx --env-file=.env.local scripts/policy-moderate.ts comment "" \
 *     "Buy followers cheap! www.spammy.example/promo"
 *
 * Requires MODERATION_ENABLED=1 in .env.local OR set inline:
 *   MODERATION_ENABLED=1 pnpm tsx --env-file=.env.local scripts/policy-moderate.ts ...
 */

import { moderate } from "@/lib/moderation";
import type {
  ModerationAuthor,
  ModerationContent,
  ModerationKind,
} from "@/lib/moderation";

function usage(): never {
  console.error(
    `Usage: pnpm tsx --env-file=.env.local scripts/policy-moderate.ts <submission|comment> "<title>" "<body>"`,
  );
  process.exit(2);
}

const [, , rawKind, title, body] = process.argv;

if (!rawKind || body === undefined) usage();
if (rawKind !== "submission" && rawKind !== "comment") {
  console.error(`First arg must be "submission" or "comment"; got "${rawKind}"`);
  usage();
}

if (!process.env.OPENAI_API_KEY) {
  console.error(
    "OPENAI_API_KEY is not set. Add it to web/.env.local (see web/.env.example).",
  );
  process.exit(2);
}
if (process.env.MODERATION_ENABLED !== "1") {
  console.error(
    "MODERATION_ENABLED is not '1'. Set MODERATION_ENABLED=1 in web/.env.local " +
      "or inline (`MODERATION_ENABLED=1 pnpm tsx ...`) — otherwise the moderator " +
      "short-circuits to a synthetic 'pass' verdict.",
  );
  process.exit(2);
}

const kind = rawKind as ModerationKind;

const content: ModerationContent = {
  kind,
  title: title ?? "",
  body,
};

// Synthetic author — the smoke test bypasses DB lookups. Role 'user'
// with isAgent=false + botModerationExempt=false matches a regular
// signed-in human, the most common shape of moderation traffic.
const author: ModerationAuthor = {
  id: "00000000-0000-0000-0000-000000000000",
  role: "user",
  isAgent: false,
  botModerationExempt: false,
};

(async () => {
  const t0 = Date.now();
  const verdict = await moderate(content, author);
  const elapsed = Date.now() - t0;

  console.log(
    JSON.stringify(
      {
        kind,
        title,
        bodyPreview: body.slice(0, 120) + (body.length > 120 ? "…" : ""),
        verdict: verdict.verdict,
        category: verdict.category,
        confidence: verdict.confidence,
        oneLineWhy: verdict.oneLineWhy,
        synthetic: verdict.synthetic,
        modelId: verdict.modelId,
        promptVersion: verdict.promptVersion,
        costUsd: verdict.costUsd,
        elapsedMs: elapsed,
      },
      null,
      2,
    ),
  );
})().catch((err) => {
  console.error(`Smoke test failed: ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
});
