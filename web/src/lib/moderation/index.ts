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

import { and, count, eq, gte } from "drizzle-orm";

import { db } from "@/db/client";
import { policyDecisions } from "@/db/schema";
import { callPolicyModel } from "./openai";
import { isExemptFromModeration } from "./exempt";
import {
  POLICY_MODEL,
  POLICY_PROMPT_V,
  type ModerationAuthor,
  type ModerationContent,
  type ModerationVerdict,
} from "./types";

export type {
  ModerationAuthor,
  ModerationContent,
  ModerationKind,
  ModerationVerdict,
  SyntheticReason,
} from "./types";
export { isExemptFromModeration };
export {
  POLICY_CATEGORIES,
  POLICY_PROMPT_V,
  POLICY_MODEL,
} from "./types";
export type { PolicyCategory, PolicyVerdict } from "./types";
export { writePolicyDecision, writeModerationLogForReject } from "./persist";
export { writeModerationNotification } from "./notify";
export { getSystemUserId } from "./system-user";
export {
  listMyDecisions,
  listMyDecisionsInputSchema,
  getMyDecision,
  getMyDecisionInputSchema,
  type ListMyDecisionsInput,
  type GetMyDecisionInput,
  type PolicyDecisionDto,
} from "./me-decisions";
export {
  checkBanCandidate,
  checkLadderRateLimit,
  recentRejectsForAuthor,
  LADDER_THRESHOLDS,
  type LadderRateLimitDecision,
} from "./ladder";
export {
  drainRetroQueue,
  enqueueRetroComment,
  type DrainResult as RetroDrainResult,
  type EnqueueRetroParams,
} from "./retro-queue";

function isEnabled(): boolean {
  const v = process.env.MODERATION_ENABLED;
  if (v === undefined) return process.env.NODE_ENV === "production";
  return v === "1" || v === "true";
}

function syntheticPass(
  oneLineWhy: string,
  syntheticReason: "exempt" | "disabled" | "error" | "capped",
): ModerationVerdict {
  return {
    verdict: "pass",
    category: null,
    confidence: "high",
    oneLineWhy,
    synthetic: true,
    syntheticReason,
    modelId: POLICY_MODEL,
    promptVersion: POLICY_PROMPT_V,
    costUsd: null,
  };
}

/**
 * Boot-time guard: in production with MODERATION_ENABLED=1 and no
 * OPENAI_API_KEY, the app would silently synth-pass every submission
 * — the worst possible failure mode. Force a fast crash instead.
 * Safe in dev (MODERATION_ENABLED defaults off) and in CI / test
 * (NODE_ENV !== 'production').
 */
function assertProductionConfig(): void {
  if (
    process.env.NODE_ENV === "production" &&
    isEnabled() &&
    !process.env.OPENAI_API_KEY
  ) {
    throw new Error(
      "MODERATION_ENABLED=1 in production but OPENAI_API_KEY is not set — refusing to start",
    );
  }
}
assertProductionConfig();

/**
 * Per-author cost guard. Counts policy_decisions rows the moderator
 * has produced for this user since UTC midnight today. Beyond the
 * cap, moderate() short-circuits to a synthetic 'capped' verdict
 * which the caller handles via the failure-mode matrix (plan §6).
 *
 * Strawman cap: 50/day. Tunable via env so an operator can raise
 * it for a specific incident without a code change. Constants are
 * defaulted in code so dev / CI don't need to know about it.
 */
const DEFAULT_DAILY_MODERATE_CAP = 50;

function getDailyCap(): number {
  const v = Number(process.env.MODERATION_DAILY_CAP_PER_AUTHOR);
  return Number.isFinite(v) && v > 0 ? Math.floor(v) : DEFAULT_DAILY_MODERATE_CAP;
}

async function isAtDailyCap(authorId: string): Promise<boolean> {
  const cap = getDailyCap();
  // Counts all real policy_decisions rows (synthetic verdicts don't
  // persist) since UTC midnight. The hot path is one indexed COUNT.
  const startOfDayUtc = new Date();
  startOfDayUtc.setUTCHours(0, 0, 0, 0);
  const [row] = await db
    .select({ n: count() })
    .from(policyDecisions)
    .where(
      and(
        eq(policyDecisions.authorId, authorId),
        gte(policyDecisions.decidedAt, startOfDayUtc),
      ),
    );
  return (row?.n ?? 0) >= cap;
}

/**
 * Test-injection seam. When set (only in test environments), the
 * moderate() function returns the override directly without hitting
 * exempt/enabled/cap/model paths. Production code never reads this;
 * the setter throws in production as a sanity net.
 */
let __TEST_VERDICT_OVERRIDE: ModerationVerdict | null = null;

export function __setTestVerdictOverride(
  verdict: ModerationVerdict | null,
): void {
  if (process.env.NODE_ENV === "production") {
    throw new Error(
      "__setTestVerdictOverride is forbidden in production",
    );
  }
  __TEST_VERDICT_OVERRIDE = verdict;
}

export async function moderate(
  content: ModerationContent,
  author: ModerationAuthor,
): Promise<ModerationVerdict> {
  if (
    __TEST_VERDICT_OVERRIDE &&
    process.env.NODE_ENV !== "production"
  ) {
    return __TEST_VERDICT_OVERRIDE;
  }

  if (isExemptFromModeration(author)) {
    return syntheticPass("author exempt from moderation", "exempt");
  }

  if (!isEnabled()) {
    return syntheticPass("moderation disabled", "disabled");
  }

  // Cost guard: cap at N moderate() calls per author per UTC day.
  // Beyond the cap, return synthetic 'capped' so the caller's
  // failure-mode matrix kicks in (submissions → pending, comments →
  // optimistic publish + retro queue). Avoids runaway OpenAI spend
  // from a single user blasting submissions.
  if (await isAtDailyCap(author.id)) {
    return syntheticPass(
      `daily moderation cap (${getDailyCap()}) reached for this author`,
      "capped",
    );
  }

  try {
    const { response, costUsd } = await callPolicyModel(content);
    return {
      verdict: response.verdict,
      category: response.category,
      confidence: response.confidence,
      oneLineWhy: response.one_line_why,
      synthetic: false,
      syntheticReason: null,
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
    return syntheticPass(
      `moderator unavailable: ${msg.slice(0, 120)}`,
      "error",
    );
  }
}
