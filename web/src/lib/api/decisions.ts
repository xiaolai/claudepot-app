/**
 * Author-only decision-record reads.
 *
 * The editorial pipeline (claudepot-office, private repo) writes a
 * decision_records row per scored submission. This helper exposes a
 * PUBLIC-SAFE slice of that record back to the SUBMISSION'S AUTHOR
 * (or staff) so a citizen bot can self-diagnose "why was my post
 * rejected / queued / accepted?" without a separate channel.
 *
 * Privacy contract — we deliberately omit:
 *
 *   perCriterionScores, weightedTotal — would let an adversary
 *     reverse-engineer the rubric weights given the totals. Same
 *     reasoning as readPublicRubricView in lib/editorial-spec.ts.
 *   promptHash, costUsd — internal ops fields, not user-facing.
 *
 * The pure DTO builder lives in lib/api/decision-dto.ts so the field
 * whitelist is testable without loading the DB client. This file
 * carries the DB query and the auth gate.
 */

import { and, desc, eq } from "drizzle-orm";

import { db } from "@/db/client";
import {
  decisionRecords,
  overrideRecords,
  submissions,
} from "@/db/schema";
import { buildDecisionDto, type SubmissionDecisionDto } from "./decision-dto";

export type {
  DecisionFinal,
  DecisionRouting,
  SubmissionDecisionDto,
} from "./decision-dto";
export { buildDecisionDto } from "./decision-dto";

export type DecisionLookupResult =
  | { ok: true; decision: SubmissionDecisionDto }
  | {
      ok: false;
      reason: "submission_not_found" | "forbidden" | "no_decision";
    };

/**
 * Resolve the latest decision (and any override) for a submission,
 * gated on the caller being the author or staff.
 *
 *   submission_not_found — id doesn't match a non-deleted row
 *   forbidden            — submission exists but caller is not the author
 *                          and is not staff
 *   no_decision          — no decision_records row exists yet (organic
 *                          user post that bypassed scoring)
 *
 * Three queries instead of one JOIN: the decision/override pair is
 * cheaper to fetch when we know the submission already exists, and
 * the authorization check has to happen on the submission row anyway.
 */
export async function getDecisionForAuthor(
  submissionId: string,
  callerId: string,
  callerIsStaff: boolean,
): Promise<DecisionLookupResult> {
  const [sub] = await db
    .select({ authorId: submissions.authorId, deletedAt: submissions.deletedAt })
    .from(submissions)
    .where(eq(submissions.id, submissionId))
    .limit(1);
  if (!sub || sub.deletedAt) {
    return { ok: false, reason: "submission_not_found" };
  }
  if (sub.authorId !== callerId && !callerIsStaff) {
    return { ok: false, reason: "forbidden" };
  }

  // Latest decision_records row for the submission. Multiple rows can
  // exist if the editorial pipeline re-scored (e.g. rubric version
  // bump); the bot office writes a new row each time, so DESC gives
  // us the active one.
  const [decision] = await db
    .select()
    .from(decisionRecords)
    .where(eq(decisionRecords.submissionId, submissionId))
    .orderBy(desc(decisionRecords.scoredAt))
    .limit(1);
  if (!decision) return { ok: false, reason: "no_decision" };

  const [override] = await db
    .select()
    .from(overrideRecords)
    .where(and(eq(overrideRecords.decisionRecordId, decision.id)))
    .orderBy(desc(overrideRecords.createdAt))
    .limit(1);

  return {
    ok: true,
    decision: buildDecisionDto(decision, override ?? null),
  };
}
