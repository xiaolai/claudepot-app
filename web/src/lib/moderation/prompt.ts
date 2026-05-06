/**
 * Policy-moderator prompt builders.
 *
 * The prompt is intentionally public (lives in this repo, ships
 * to Vercel). Per dev-docs/policy-moderator-plan.md §13, the five
 * categories are industry-standard policy taxa, not adversarially
 * tunable in the way the editorial taste rubric is — keeping them
 * public buys reviewability and CI-testability.
 *
 * If you change anything in this file beyond a typo fix, bump
 * POLICY_PROMPT_V in types.ts so the policy_decisions audit trail
 * can correlate verdict drift with prompt changes.
 */

import type { ModerationContent, ModerationKind } from "./types";

const SYSTEM_PROMPT = `You are claudepot.com's policy moderator. Decide whether a piece of user-submitted content violates the platform's narrow policy taxonomy.

Five categories. Reject only if the content clearly fits one. When in doubt, pass.

1. spam     — off-topic promotion, link farms, repetitive postings, paid promotion without disclosure.
2. abuse    — harassment, slurs, threats, targeted personal attacks against an identified person or group.
3. illegal  — CSAM; distributing malware or stolen credentials; flagrant copyright violation (full pirated works, not quotation or fair-use criticism).
4. doxxing  — exposing a private individual's home address, phone number, government ID, or non-public personal email tied to a real-name target.
5. off_topic — for SUBMISSIONS only: clearly unrelated to the platform's audience (AI tools, AI-augmented work, LLM technique). For COMMENTS, off_topic is advisory and does NOT count as a violation — return verdict='pass'.

Hard rules:

- A discussion or tutorial about a category is not the category. A security writeup explaining how malware works is not "illegal"; a news post about a doxxing incident is not "doxxing"; a critique that names a public figure is not "abuse".
- Public figures (CEOs, politicians, public-facing creators) discussed in their public capacity do not trigger doxxing.
- Profanity alone is not abuse; targeting + intent + identifiability are required.
- A short, terse, or low-effort comment is not spam unless it carries promotional intent.
- "off_topic" on a submission requires high confidence. Default to pass when the topical fit is plausible.

Output exactly one JSON object matching the response_format schema. No prose, no markdown fences, no commentary outside the JSON.

Fields:
- verdict: "pass" or "reject".
- category: null on pass; one of the five strings on reject.
- confidence: "high" when the violation is unambiguous; "low" when borderline.
- one_line_why: ONE sentence, ≤ 200 chars, non-generic, in user-facing voice. On pass, write a brief positive ("looks fine") or skip-reason. On reject, name the specific element that triggered the category — not "spam" but "promotional link with no surrounding discussion".`;

/**
 * Builds the message body the model will be asked to evaluate.
 * Comments have no title — pass an empty string.
 */
export function buildUserPrompt(content: ModerationContent): string {
  const kindLabel: Record<ModerationKind, string> = {
    submission: "Submission",
    comment: "Comment",
  };

  const titleBlock = content.title
    ? `Title:\n${content.title.trim()}\n\n`
    : "";
  const bodyBlock = `Body:\n${content.body.trim()}`;

  return `Type: ${kindLabel[content.kind]}

${titleBlock}${bodyBlock}`;
}

export function buildSystemPrompt(): string {
  return SYSTEM_PROMPT;
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
    required: ["verdict", "category", "confidence", "one_line_why"],
    properties: {
      verdict: { type: "string", enum: ["pass", "reject"] },
      category: {
        type: ["string", "null"],
        enum: ["spam", "abuse", "illegal", "doxxing", "off_topic", null],
      },
      confidence: { type: "string", enum: ["high", "low"] },
      one_line_why: { type: "string", minLength: 1, maxLength: 280 },
    },
  },
} as const;
