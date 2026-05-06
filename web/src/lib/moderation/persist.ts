/**
 * Persistence helpers for moderation verdicts.
 *
 * Per dev-docs/policy-moderator-plan.md §4.5, the rule is:
 *
 *   - Every real model call → one policy_decisions row.
 *   - Terminal state-changing rejects → also one moderation_log row
 *     with staff_id = system user id.
 *
 * Synthetic verdicts (MODERATION_ENABLED=0, fail-open dev path)
 * write nothing — there's no real decision to record. The caller
 * checks `verdict.synthetic` before calling persistDecision.
 *
 * createSubmission is non-transactional today (the submission
 * insert and tag insert run sequentially), so these helpers write
 * directly through the shared `db` client. If a transaction is
 * later introduced in that path, retype `exec` to accept `tx` too.
 */

import { db } from "@/db/client";
import { moderationLog, policyDecisions } from "@/db/schema";

import { getSystemUserId } from "./system-user";
import type {
  ModerationKind,
  ModerationVerdict,
  PolicyCategory,
} from "./types";

export interface PersistDecisionParams {
  authorId: string;
  targetType: ModerationKind;
  /** Null only for the illegal-comment block path. */
  targetId: string | null;
  verdict: ModerationVerdict;
  /** 1 for the initial pass; 2 for the comment confirmation pass. */
  passNumber?: number;
}

/**
 * Writes one policy_decisions row. Returns the row id so the caller
 * can deep-link to the appeal page (/appeal/[id]).
 *
 * No-op for synthetic verdicts — caller should check first to skip
 * the write entirely. Throws if invoked with one anyway, so a
 * future caller misuse is loud rather than silent.
 */
export async function writePolicyDecision(
  params: PersistDecisionParams,
): Promise<string> {
  if (params.verdict.synthetic) {
    throw new Error(
      "writePolicyDecision called with a synthetic verdict — caller must skip persist",
    );
  }

  const [row] = await db
    .insert(policyDecisions)
    .values({
      authorId: params.authorId,
      targetType: params.targetType,
      targetId: params.targetId,
      verdict: params.verdict.verdict,
      category: params.verdict.category,
      confidence: params.verdict.confidence,
      oneLineWhy: params.verdict.oneLineWhy,
      modelId: params.verdict.modelId,
      promptVersion: params.verdict.promptVersion,
      costUsd:
        params.verdict.costUsd === null
          ? null
          : params.verdict.costUsd.toFixed(6),
      passNumber: params.passNumber ?? 1,
    })
    .returning({ id: policyDecisions.id });

  if (!row) {
    throw new Error("policy_decisions insert returned no row");
  }
  return row.id;
}

export interface ModerationLogParams {
  targetType: ModerationKind;
  targetId: string;
  category: PolicyCategory;
  oneLineWhy: string;
  /** 1 = initial reject; 2 = comment confirmation-pass retract. */
  passNumber?: number;
}

/**
 * Writes one moderation_log row with the system user as actor.
 * Call only on terminal rejects per §4.5 — the rule is "state-
 * changing terminal events only", which keeps /admin/log readable.
 *
 * Note content: only the category (and optional pass=2 marker) goes
 * into the public-facing `note` field. The moderator's verbatim
 * one_line_why CAN contain the very PII the model just classified
 * (e.g. on a doxxing reject the model may quote the address that
 * triggered the rule), and /admin/log is visible to any signed-in
 * user. Staff who need the full reasoning drill into the joined
 * policy_decisions row by target_id.
 */
export async function writeModerationLogForReject(
  params: ModerationLogParams,
): Promise<void> {
  const staffId = await getSystemUserId();
  const noteParts: string[] = [params.category];
  if (params.passNumber === 2) noteParts.push("[pass=2]");
  const note = noteParts.join(" ").slice(0, 500);

  await db.insert(moderationLog).values({
    staffId,
    action: "reject",
    targetType: params.targetType,
    targetId: params.targetId,
    note,
  });
}
