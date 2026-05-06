/**
 * Public entry point for the AI policy moderator.
 *
 * Two paths:
 *
 *   - MODERATION_ENABLED=0 (or unset in non-production): synthesize
 *     a `pass` verdict and return immediately. No DB writes, no
 *     OpenAI call. Lets contributors run the app locally without
 *     an OpenAI key. Caller skips persistence entirely on
 *     `verdict.synthetic`.
 *
 *   - MODERATION_ENABLED=1: invoke the model with a hard 1500ms
 *     timeout. On error: fail-closed for submissions (return a
 *     synthetic 'pass' but mark synthetic=true so the caller can
 *     route the row to state='pending' as a safety net). Comments
 *     fail-open the same way; the kind-specific decision is the
 *     caller's job, not the moderator's — moderate() reports what
 *     happened, the caller decides what to do.
 *
 * No retries — see openai.ts for rationale.
 */

import { callPolicyModel } from "./openai";
import { isExemptFromModeration } from "./exempt";
import {
  POLICY_MODEL,
  POLICY_PROMPT_V,
  type ModerationAuthor,
  type ModerationContent,
  type ModerationVerdict,
} from "./types";

export type { ModerationAuthor, ModerationContent, ModerationVerdict };
export { isExemptFromModeration };
export {
  POLICY_CATEGORIES,
  POLICY_PROMPT_V,
  POLICY_MODEL,
} from "./types";
export type { PolicyCategory, PolicyVerdict } from "./types";
export { writePolicyDecision, writeModerationLogForReject } from "./persist";
export { writeModerationNotification } from "./notify";

function isEnabled(): boolean {
  const v = process.env.MODERATION_ENABLED;
  if (v === undefined) return process.env.NODE_ENV === "production";
  return v === "1" || v === "true";
}

function syntheticPass(reason: string): ModerationVerdict {
  return {
    verdict: "pass",
    category: null,
    confidence: "high",
    oneLineWhy: reason,
    synthetic: true,
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: null,
  };
}

export async function moderate(
  content: ModerationContent,
  author: ModerationAuthor,
): Promise<ModerationVerdict> {
  if (isExemptFromModeration(author)) {
    return syntheticPass("author exempt from moderation");
  }

  if (!isEnabled()) {
    return syntheticPass("moderation disabled");
  }

  try {
    const { response, costUsd } = await callPolicyModel(content);
    return {
      verdict: response.verdict,
      category: response.category,
      confidence: response.confidence,
      oneLineWhy: response.one_line_why,
      synthetic: false,
      modelId: POLICY_MODEL,
      promptVersion: POLICY_PROMPT_V,
      costUsd,
    };
  } catch (err) {
    // Surface the error so observability can pick it up; the caller
    // will see synthetic=true and route accordingly. Use console.warn
    // (not error) so a model timeout doesn't trigger Vercel's error
    // alarms — these are expected at low single-digit %.
    const msg = err instanceof Error ? err.message : String(err);
    console.warn(`[moderation] model call failed: ${msg}`);
    return syntheticPass(`moderator unavailable: ${msg.slice(0, 120)}`);
  }
}
