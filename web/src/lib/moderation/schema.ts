/**
 * Zod schema for the OpenAI structured-output response.
 *
 * The response_format passed to chat.completions.create instructs the
 * model to emit JSON conforming to this exact shape; we re-validate
 * with Zod on receipt because (a) the OpenAI SDK accepts the schema
 * but doesn't enforce it on parse, and (b) a stricter Zod parse
 * lets us catch drift early when we bump POLICY_PROMPT_V.
 *
 * Field names use snake_case to match the JSON-schema we send, which
 * matches the model's natural output cadence and minimizes a class
 * of "the model picked the wrong key" errors.
 */

import { z } from "zod";
import { POLICY_CATEGORIES, POLICY_CONFIDENCE, POLICY_VERDICTS } from "./types";

export const PolicyResponseSchema = z.object({
  verdict: z.enum(POLICY_VERDICTS),
  // null is allowed and is the expected value when verdict='pass'.
  category: z.enum(POLICY_CATEGORIES).nullable(),
  confidence: z.enum(POLICY_CONFIDENCE),
  // 200-char ceiling matches the notification payload's display
  // budget. The prompt asks for a non-generic single sentence; if
  // the model overruns, we'd rather fail-validate than truncate
  // silently and lose context.
  one_line_why: z.string().min(1).max(280),
});

export type PolicyResponse = z.infer<typeof PolicyResponseSchema>;

/**
 * Cross-field invariant the prompt asks the model to maintain:
 * verdict='pass' iff category is null. We enforce it server-side
 * because some model outputs ship a leftover category on pass.
 */
export function reconcileCategory(parsed: PolicyResponse): PolicyResponse {
  if (parsed.verdict === "pass") {
    return { ...parsed, category: null };
  }
  if (parsed.verdict === "reject" && parsed.category === null) {
    // Reject without a category is invalid; surface it as schema-fail
    // up the stack so the caller can fail-open or fail-closed per kind.
    throw new Error(
      "Policy moderator returned verdict='reject' with category=null",
    );
  }
  return parsed;
}
