/**
 * Public types for the AI policy moderator.
 *
 * Four universal trust-and-safety categories. Platform-identity
 * judgments (was the submission "on topic for AI builders") are
 * NOT a moderator concern — those belong to editorial scoring
 * (rubric.yml's domain_legibility, vote signal, persona overlays).
 * The moderator gate enforces what every public discussion
 * platform must enforce, and nothing more.
 *
 * History: an `off_topic` category existed in POLICY_PROMPT_V="1"
 * (taxonomy of five). It was removed in v="2" because (a) it
 * coupled the gate to a never-formalized definition of "the
 * platform's audience" that gpt-4o-mini misread, and (b) the
 * editorial scoring layer already handles topical fit better — a
 * misplaced submission gets downscored and falls out of feeds
 * without anyone deciding it should never have been posted.
 *
 * Existing rows in policy_decisions with category='off_topic'
 * persist by design — `category` is a plain text column, not an
 * enum, so legacy values stay queryable. Display surfaces
 * (/office/policy, /appeal/[id]) keep their off_topic labels for
 * historical rendering.
 */

export const POLICY_CATEGORIES = [
  "spam",
  "abuse",
  "illegal",
  "doxxing",
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

/**
 * Tag Ada proposes alongside an accepted submission (migration
 * 0022). Up to 2 per pass; empty otherwise (rejects, comments,
 * synthetic verdicts).
 *
 *   - slug: lowercase-kebab, ^[a-z][a-z0-9-]+$ (server validates).
 *   - isNew: model's hint that the slug isn't in the active
 *     vocabulary. Reconciled server-side: if isNew=false but the
 *     slug doesn't exist, the row is treated as new (and inserted
 *     with pending_review=true). Hint, not contract.
 */
export interface ModerationTag {
  slug: string;
  isNew: boolean;
}

export interface ModerationVerdict {
  verdict: PolicyVerdict;
  /** null on pass; the rejected category otherwise. */
  category: PolicyCategory | null;
  confidence: PolicyConfidence;
  oneLineWhy: string;
  /**
   * Up to 2 tags Ada chose for the accepted submission. Empty for
   * rejects, comments, and synthetic verdicts. Server-side reconcile
   * handles dedup against user-supplied tags + slug validation.
   */
  tags: ModerationTag[];
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
 *
 * v=1 — initial five-category taxonomy (spam/abuse/illegal/doxxing/off_topic).
 * v=2 — off_topic removed; minimal universal trust-and-safety floor only.
 *       Tagging instructions added (Ada-as-tagger).
 */
export const POLICY_PROMPT_V = "2";

/**
 * Pinned model id. Versioned model ids let us correlate verdict
 * drift with model upgrades when calibration data exists.
 */
export const POLICY_MODEL = "gpt-4o-mini";
