/**
 * Public types for the AI policy moderator.
 *
 * The five categories are defined in dev-docs/policy-moderator-plan.md
 * §2 — narrow on purpose, calibration-ready. Adding a category requires
 * a prompt update + an audit; do not extend casually.
 */

export const POLICY_CATEGORIES = [
  "spam",
  "abuse",
  "illegal",
  "doxxing",
  "off_topic",
] as const;

export type PolicyCategory = (typeof POLICY_CATEGORIES)[number];

export const POLICY_VERDICTS = ["pass", "reject"] as const;
export type PolicyVerdict = (typeof POLICY_VERDICTS)[number];

export const POLICY_CONFIDENCE = ["high", "low"] as const;
export type PolicyConfidence = (typeof POLICY_CONFIDENCE)[number];

export type ModerationKind = "submission" | "comment";

export interface ModerationContent {
  kind: ModerationKind;
  /** Empty string is allowed for comments (which have no title). */
  title: string;
  body: string;
}

export interface ModerationAuthor {
  id: string;
  role: "user" | "staff" | "locked" | "system";
  isAgent: boolean;
  botModerationExempt: boolean;
}

/**
 * Outcome of a single moderate() call. Pure data — no DB writes
 * happen in the moderator itself; persistence (policy_decisions +
 * conditional moderation_log + notification) is done by the caller
 * inside its own transaction so the verdict and any state change
 * commit atomically.
 */
/**
 * Why a verdict ended up as 'pass' WITHOUT a real model call. The
 * caller branches on this to apply the failure-mode matrix:
 *
 *   - 'exempt' / 'disabled' → genuine "no moderation needed" — caller
 *     uses its existing default state (karma gate, optimistic publish).
 *   - 'error' → model call failed (timeout, 5xx, schema parse,
 *     refusal). Submissions force state='pending'; comments
 *     publish optimistically and enqueue for retroactive review.
 *     Plan §11.
 *   - 'capped' → per-author daily moderate-call cap exceeded
 *     (cost guard, plan §6). Treated the same as 'error' by the
 *     failure-mode matrix.
 *   - null when verdict.synthetic === false.
 */
export type SyntheticReason = "exempt" | "disabled" | "error" | "capped";

export interface ModerationVerdict {
  verdict: PolicyVerdict;
  /** null on pass; the rejected category otherwise. */
  category: PolicyCategory | null;
  confidence: PolicyConfidence;
  oneLineWhy: string;
  /**
   * True when the verdict was synthesized (exempt author, moderator
   * disabled in dev, or a model error). Persistence layer skips the
   * policy_decisions row when synthetic=true, since there's no real
   * model decision to record.
   */
  synthetic: boolean;
  /** Discriminator for synthetic verdicts. Null when synthetic=false. */
  syntheticReason: SyntheticReason | null;
  modelId: string;
  promptVersion: string;
  /** Null when synthetic; populated when the model run reported usage. */
  costUsd: number | null;
}

/**
 * Bumped whenever the prompt changes in a way that would invalidate
 * historical comparisons. Stored on every policy_decisions row.
 */
export const POLICY_PROMPT_V = "1";

/**
 * Pinned model id. Versioned model ids let us correlate verdict
 * drift with model upgrades when calibration data exists.
 */
export const POLICY_MODEL = "gpt-4o-mini";
