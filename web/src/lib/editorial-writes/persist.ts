/**
 * Persistence for the office's editorial-write surface (migration
 * 0036). Three writers:
 *
 *   - persistDecision: INSERT decision_records with idempotent
 *     ON CONFLICT DO NOTHING on idx_decision_records_idempotency
 *     (submission_id, applied_persona, model_id, COALESCE(prompt_hash,'')).
 *     If the conflict fires, SELECT and return the existing id —
 *     re-POSTing the same tuple is a no-op for the office. The
 *     write is wrapped in a single db.transaction so the
 *     conflict-then-select path can't observe a torn state from a
 *     concurrent insert.
 *
 *   - persistOverride: INSERT override_records keyed off the
 *     decision_records.id from the URL. reviewer_kind is forced to
 *     'bot' here — the human-staff override flow stays in
 *     /admin/console and never calls this code path.
 *
 *   - persistScoutRun: INSERT scout_runs row.
 *
 * NEITHER decision write nor override write touches submissions.state.
 * Publishing a draft is a separate primitive (POST
 * /api/v1/submissions/{id}/publish, scope submission:publish) so the
 * polity does not encode editorial policy ("one accept = publish").
 * The office orchestrator decides when its policy is satisfied and
 * calls publish explicitly. See dev-docs/2026-05-08-polity-api-replies.md
 * for the boundary discussion.
 *
 * Authorship is per-token: callers pass auth.user.id which becomes
 * override_records.reviewer_id. decision_records and scout_runs do
 * not record an author column today — the rubric/audience versions
 * + model_id are the provenance trail, and the office writes from a
 * single PAT per persona, so adding a denormalized author would
 * duplicate what the join already recovers.
 */

import { and, eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  decisionRecords,
  overrideRecords,
  scoutRuns,
  submissions,
} from "@/db/schema";

import type {
  DecisionInput,
  OverrideInput,
  ScoutRunInput,
} from "./schemas";

export type PersistDecisionResult =
  | { ok: true; decisionId: string; created: boolean }
  | { ok: false; reason: "submission_not_found" }
  | { ok: false; reason: "validation"; detail: string };

export type PersistOverrideResult =
  | { ok: true; overrideId: string }
  | { ok: false; reason: "decision_not_found" }
  | { ok: false; reason: "validation"; detail: string };

export type PersistScoutRunResult =
  | { ok: true; scoutRunId: string }
  | { ok: false; reason: "validation"; detail: string };

export async function persistDecision(
  input: DecisionInput,
): Promise<PersistDecisionResult> {
  // Confirm the submission exists before opening the transaction —
  // the FK would catch it inside the tx, but we'd rather surface a
  // 404 than a generic 500 and we want to avoid an empty rollback.
  const [sub] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(eq(submissions.id, input.submissionId))
    .limit(1);
  if (!sub) return { ok: false, reason: "submission_not_found" };

  return db.transaction(async (tx): Promise<PersistDecisionResult> => {
    const inserted = await tx
      .insert(decisionRecords)
      .values({
        submissionId: input.submissionId,
        rubricVersion: input.rubricVersion,
        audienceDocVersion: input.audienceDocVersion,
        appliedPersona: input.appliedPersona,
        perCriterionScores: input.perCriterionScores,
        weightedTotal: String(input.weightedTotal),
        hardRejectsHit: input.hardRejectsHit,
        inclusionGates: input.inclusionGates,
        typeInferred: input.typeInferred,
        subSegmentInferred: input.subSegmentInferred,
        confidence: input.confidence,
        oneLineWhy: input.oneLineWhy,
        finalDecision: input.finalDecision,
        routing: input.routing,
        modelId: input.modelId,
        promptHash: input.promptHash ?? null,
        costUsd: input.costUsd != null ? String(input.costUsd) : null,
      })
      // Bare ON CONFLICT DO NOTHING: the only non-PK unique on
      // decision_records is idx_decision_records_idempotency, so any
      // conflict here is the idempotency unique firing. The PK is on
      // id (defaultRandom) and never collides on insert.
      .onConflictDoNothing()
      .returning({ id: decisionRecords.id });

    if (inserted.length > 0) {
      return { ok: true, decisionId: inserted[0].id, created: true };
    }

    // Conflict on the idempotency unique — SELECT the existing row.
    // promptHash is nullable, so a NULL retry must collide with the
    // prior NULL row (matching the unique's COALESCE-to-empty-string
    // semantics on the column-expression index).
    const promptHashClause =
      input.promptHash == null
        ? sql`${decisionRecords.promptHash} IS NULL`
        : eq(decisionRecords.promptHash, input.promptHash);
    const [existing] = await tx
      .select({ id: decisionRecords.id })
      .from(decisionRecords)
      .where(
        and(
          eq(decisionRecords.submissionId, input.submissionId),
          eq(decisionRecords.appliedPersona, input.appliedPersona),
          eq(decisionRecords.modelId, input.modelId),
          promptHashClause,
        ),
      )
      .limit(1);
    if (!existing) {
      // Unreachable — the conflict implies the row exists. If we got
      // here, the row was deleted between the conflict and the
      // re-read; surface as validation rather than crashing.
      return {
        ok: false,
        reason: "validation",
        detail: "Idempotency conflict but no matching record found.",
      };
    }
    return { ok: true, decisionId: existing.id, created: false };
  });
}

export async function persistOverride(
  decisionId: string,
  reviewerId: string,
  input: OverrideInput,
): Promise<PersistOverrideResult> {
  const [decision] = await db
    .select({
      id: decisionRecords.id,
      finalDecision: decisionRecords.finalDecision,
    })
    .from(decisionRecords)
    .where(eq(decisionRecords.id, decisionId))
    .limit(1);
  if (!decision) return { ok: false, reason: "decision_not_found" };

  const [row] = await db
    .insert(overrideRecords)
    .values({
      decisionRecordId: decisionId,
      reviewerId,
      originalDecision: decision.finalDecision,
      overrideDecision: input.overrideDecision,
      overrideRouting: input.overrideRouting,
      reviewerScores: input.reviewerScores ?? null,
      reason: input.reason,
      reviewerKind: "bot",
    })
    .returning({ id: overrideRecords.id });

  return { ok: true, overrideId: row.id };
}

export async function persistScoutRun(
  input: ScoutRunInput,
): Promise<PersistScoutRunResult> {
  const [row] = await db
    .insert(scoutRuns)
    .values({
      sourceId: input.sourceId,
      startedAt: new Date(input.startedAt),
      finishedAt: new Date(input.finishedAt),
      itemsPulled: input.itemsPulled,
      itemsKept: input.itemsKept,
      itemsDropped: input.itemsDropped,
      error: input.error ?? null,
    })
    .returning({ id: scoutRuns.id });

  return { ok: true, scoutRunId: row.id };
}
