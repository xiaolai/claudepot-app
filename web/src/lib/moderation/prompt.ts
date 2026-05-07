/**
 * Policy-moderator prompt builders.
 *
 * The prompt is intentionally public (lives in this repo, ships
 * to Vercel). Per dev-docs/policy-moderator-plan.md §13, the five
 * categories are industry-standard policy taxa, not adversarially
 * tunable in the way the editorial taste rubric is — keeping them
 * public buys reviewability and CI-testability.
 *
 * Migration 0021 added a DB-backed editor at /admin/policy-prompt.
 * The constant below is the FALLBACK that ships with the deploy;
 * once staff saves a row in moderation_prompts via the editor,
 * lib/moderation/prompt-store.ts returns the active row instead.
 *
 * If you change FALLBACK_SYSTEM_PROMPT, bump the version label in
 * the editor's first save (or in the seed) so policy_decisions.
 * prompt_version stays meaningful as an audit trail.
 */

import type { ModerationContent, ModerationKind } from "./types";

export const FALLBACK_SYSTEM_PROMPT = `You are claudepot.com's policy moderator. Decide whether a piece of user-submitted content violates the platform's universal trust-and-safety taxonomy.

Four categories. Reject only if the content clearly fits one. When in doubt, pass.

1. spam     — promotional content with no surrounding discussion, link farms, repetitive postings, paid promotion without disclosure.
2. abuse    — harassment, slurs, threats, targeted personal attacks against an identified person or group.
3. illegal  — CSAM; distributing malware or stolen credentials; flagrant copyright violation (full pirated works, not quotation or fair-use criticism).
4. doxxing  — exposing a private individual's home address, phone number, government ID, or non-public personal email tied to a real-name target.

Topical fit is NOT your concern. A submission that's a poor fit for the audience is handled by the editorial scoring layer and by community voting — your gate is universal trust-and-safety only. If a submission is legal, non-abusive, non-spam, and non-doxxing, pass it.

Hard rules:

- A discussion or tutorial about a category is not the category. A security writeup explaining how malware works is not "illegal"; a news post about a doxxing incident is not "doxxing"; a critique that names a public figure is not "abuse".
- Public figures (CEOs, politicians, public-facing creators) discussed in their public capacity do not trigger doxxing.
- Profanity alone is not abuse; targeting + intent + identifiability are required.
- A short, terse, or low-effort comment is not spam unless it carries promotional intent.
- A submission about an unfamiliar topic, or one that seems out of place for the audience, is NOT a violation. Pass it.

Output exactly one JSON object matching the response_format schema. No prose, no markdown fences, no commentary outside the JSON.

Fields:
- verdict: "pass" or "reject".
- category: null on pass; one of the five strings on reject.
- confidence: "high" when the violation is unambiguous; "low" when borderline.
- one_line_why: ONE sentence, ≤ 200 chars, non-generic, in user-facing voice. On pass, write a brief positive ("looks fine") or skip-reason. On reject, name the specific element that triggered the category — not "spam" but "promotional link with no surrounding discussion".
- tags: an array of 0–2 topic tags. REQUIRED FIELD. See the tagging section below.

Tagging (submissions on PASS only):

- For ACCEPTED SUBMISSIONS, propose up to 2 topical tags in the "tags" array (0–2 entries). For REJECTS or COMMENTS, return "tags": [].
- Tags describe what the submission is ABOUT (e.g. "rag", "evals", "agents", "prompting"), not its sentiment or quality. One concept per tag.
- Slug shape: lowercase ASCII, kebab-case, matching ^[a-z][a-z0-9-]{1,40}$. No spaces, no punctuation, no leading digit, no trailing hyphen.
- Prefer existing tags from the vocabulary list provided in the user message. Set is_new=false when picking from the list.
- If no existing tag fits, you MAY propose a new tag — set is_new=true. New tags are held for staff review before becoming public.
- Do NOT propose near-duplicates of existing tags. If an existing tag is close enough, use it.
- 0 tags is a valid answer when nothing fits well. Quality over coverage.`;

/**
 * Builds the message body the model will be asked to evaluate.
 * Comments have no title — pass an empty string.
 *
 * For submissions, the active tag vocabulary is injected verbatim
 * so Ada can match against existing tags. Comments and an empty
 * vocabulary skip the block entirely. The list is intentionally
 * inert text (not JSON) so a thousand-tag dictionary doesn't blow
 * the prompt budget.
 */
export function buildUserPrompt(
  content: ModerationContent,
  availableTags: readonly string[] = [],
): string {
  const kindLabel: Record<ModerationKind, string> = {
    submission: "Submission",
    comment: "Comment",
  };

  const titleBlock = content.title
    ? `Title:\n${content.title.trim()}\n\n`
    : "";
  const bodyBlock = `Body:\n${content.body.trim()}`;

  const tagBlock =
    content.kind === "submission" && availableTags.length > 0
      ? `\n\nExisting tag vocabulary (prefer these; set is_new=false when reused):\n${availableTags.join(", ")}`
      : content.kind === "submission"
        ? "\n\nExisting tag vocabulary: (empty — every tag you choose is new; set is_new=true)"
        : "";

  return `Type: ${kindLabel[content.kind]}

${titleBlock}${bodyBlock}${tagBlock}`;
}

/**
 * Returns the fallback system prompt. lib/moderation/prompt-store.ts
 * is the runtime path — it reads the active DB row (migration 0021)
 * and falls back to this constant when the table is empty.
 */
export function buildSystemPrompt(): string {
  return FALLBACK_SYSTEM_PROMPT;
}

/**
 * The JSON-schema sent to OpenAI's structured-output endpoint.
 * Kept here (not in schema.ts which is the Zod runtime check) so the
 * prompt + schema travel together — they're a single contract.
 */
export const POLICY_RESPONSE_JSON_SCHEMA = {
  name: "policy_decision",
  strict: true,
  schema: {
    type: "object",
    additionalProperties: false,
    required: ["verdict", "category", "confidence", "one_line_why", "tags"],
    properties: {
      verdict: { type: "string", enum: ["pass", "reject"] },
      category: {
        type: ["string", "null"],
        enum: ["spam", "abuse", "illegal", "doxxing", null],
      },
      confidence: { type: "string", enum: ["high", "low"] },
      one_line_why: { type: "string", minLength: 1, maxLength: 280 },
      // Migration 0022 — Ada-proposed tags. Up to 2 per accepted
      // submission; empty array for rejects, comments, and synthetic
      // verdicts. Slug shape is enforced server-side by Zod
      // (TAG_SLUG_RE in schema.ts) — JSON schema can't carry a regex
      // under strict structured-output mode, so the format-level
      // check happens at parse time.
      tags: {
        type: "array",
        maxItems: 2,
        items: {
          type: "object",
          additionalProperties: false,
          required: ["slug", "is_new"],
          // Length bounds match `tagSlugSchema` in lib/tags/slug.ts
          // so JSON-schema rejection lines up with Zod parse + the
          // admin write path. Regex shape lives in Zod (JSON-schema
          // strict mode forbids `pattern`).
          properties: {
            slug: { type: "string", minLength: 2, maxLength: 40 },
            is_new: { type: "boolean" },
          },
        },
      },
    },
  },
} as const;
