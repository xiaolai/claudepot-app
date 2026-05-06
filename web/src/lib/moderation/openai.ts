/**
 * OpenAI SDK wrapper for the policy moderator.
 *
 * One call shape — chat.completions with structured-output JSON
 * schema. Hard 3000ms timeout via AbortController; anything slower
 * is treated as a model error per the failure-mode matrix in
 * dev-docs/policy-moderator-plan.md §12.
 *
 * The 3s ceiling is calibrated against real probe runs: warm-call
 * latency for gpt-4o-mini + structured-output spans 800–2100ms (5
 * cases sampled, median ~1500ms). The earlier 1500ms ceiling tripped
 * AbortController on roughly half of legitimate calls. 3000ms covers
 * observed max with margin without tipping into perceptible-delay
 * territory (~5s).
 *
 * No retries. The caller decides how to respond to errors based on
 * the content kind (fail-open for comments, fail-closed for
 * submissions); silently retrying here would compound latency in
 * the path that's most sensitive to it.
 */

import OpenAI from "openai";
import {
  POLICY_RESPONSE_JSON_SCHEMA,
  buildSystemPrompt,
  buildUserPrompt,
} from "./prompt";
import { PolicyResponseSchema, reconcileCategory, type PolicyResponse } from "./schema";
import { POLICY_MODEL, type ModerationContent } from "./types";

const TIMEOUT_MS = 3000;

let client: OpenAI | null = null;

function getClient(): OpenAI {
  if (client) return client;
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) {
    throw new Error(
      "OPENAI_API_KEY missing — set it or unset MODERATION_ENABLED",
    );
  }
  client = new OpenAI({ apiKey });
  return client;
}

export interface ModelCallResult {
  response: PolicyResponse;
  costUsd: number | null;
}

export async function callPolicyModel(
  content: ModerationContent,
): Promise<ModelCallResult> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), TIMEOUT_MS);

  try {
    const completion = await getClient().chat.completions.create(
      {
        model: POLICY_MODEL,
        messages: [
          { role: "system", content: buildSystemPrompt() },
          { role: "user", content: buildUserPrompt(content) },
        ],
        response_format: {
          type: "json_schema",
          json_schema: POLICY_RESPONSE_JSON_SCHEMA,
        },
        // Deterministic-as-possible — keep verdict drift attributable
        // to model upgrades or prompt changes, not sampling noise.
        temperature: 0,
      },
      { signal: ctrl.signal },
    );

    const choice = completion.choices[0];
    // OpenAI surfaces structured-output refusals on a separate
    // `refusal` field (see https://platform.openai.com/docs/guides/structured-outputs).
    // Treat as a model error so the caller falls into the failure-
    // mode matrix (submissions → pending, comments → optimistic
    // publish + retroactive). DO NOT translate to a synthetic pass
    // verdict — a refusal on user content is surprising and worth
    // investigating in /admin/log instead of silently published.
    if (choice?.message?.refusal) {
      throw new Error(
        `Policy model refused: ${String(choice.message.refusal).slice(0, 200)}`,
      );
    }
    if (!choice?.message?.content) {
      throw new Error("Policy model returned empty content");
    }

    const raw = JSON.parse(choice.message.content);
    const parsed = reconcileCategory(PolicyResponseSchema.parse(raw));

    return {
      response: parsed,
      costUsd: estimateCostUsd(completion.usage),
    };
  } finally {
    clearTimeout(timer);
  }
}

/**
 * gpt-4o-mini pricing as of 2024-07-18 (per million tokens):
 *   input: $0.150  · output: $0.600
 * Values are denominated in USD; cost is reported per call so the
 * policy_decisions row can roll up monthly spend without a per-call
 * pricing table. Update if OpenAI publishes a new tier.
 */
const PRICE_INPUT_PER_M = 0.15;
const PRICE_OUTPUT_PER_M = 0.6;

function estimateCostUsd(
  usage: { prompt_tokens?: number; completion_tokens?: number } | undefined,
): number | null {
  if (!usage) return null;
  const inTok = usage.prompt_tokens ?? 0;
  const outTok = usage.completion_tokens ?? 0;
  if (inTok === 0 && outTok === 0) return null;
  const cost = (inTok / 1_000_000) * PRICE_INPUT_PER_M + (outTok / 1_000_000) * PRICE_OUTPUT_PER_M;
  return Number(cost.toFixed(6));
}
