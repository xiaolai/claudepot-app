/**
 * Pure DTO builder + privacy whitelist for decision records.
 *
 * Split out from lib/api/decisions.ts so the field whitelist can be
 * tested without loading lib/db/client.ts (which insists on a real
 * DATABASE_URL at module-load time). This file imports only schema
 * type aliases — no runtime DB.
 *
 * Privacy contract — see lib/api/decisions.ts for the full reasoning.
 * Forbidden fields:
 *   perCriterionScores, weightedTotal — would leak rubric weights.
 *   promptHash, costUsd               — internal ops, not user-facing.
 */

import type { decisionRecords, overrideRecords } from "@/db/schema";
import type { SubmissionType } from "./dto";

export type DecisionFinal = "accept" | "reject" | "borderline_to_human_queue";
export type DecisionRouting = "feed" | "firehose" | "human_queue";

export type SubmissionDecisionDto = {
  submissionId: string;
  finalDecision: DecisionFinal;
  routing: DecisionRouting;
  oneLineWhy: string;
  hardRejectsHit: string[];
  inclusionGates: Record<string, boolean>;
  typeInferred: SubmissionType;
  subSegmentInferred: string;
  confidence: "high" | "low";
  appliedPersona: string;
  rubricVersion: string;
  audienceDocVersion: string;
  modelId: string;
  scoredAt: string;
  override: {
    overrideDecision: DecisionFinal;
    overrideRouting: DecisionRouting;
    reason: string;
    createdAt: string;
  } | null;
};

function coerceHardRejects(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is string => typeof x === "string");
}

function coerceInclusionGates(v: unknown): Record<string, boolean> {
  if (v === null || typeof v !== "object" || Array.isArray(v)) return {};
  const out: Record<string, boolean> = {};
  for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
    if (typeof val === "boolean") out[k] = val;
  }
  return out;
}

export function buildDecisionDto(
  row: typeof decisionRecords.$inferSelect,
  override: typeof overrideRecords.$inferSelect | null,
): SubmissionDecisionDto {
  return {
    submissionId: row.submissionId,
    finalDecision: row.finalDecision,
    routing: row.routing,
    oneLineWhy: row.oneLineWhy,
    hardRejectsHit: coerceHardRejects(row.hardRejectsHit),
    inclusionGates: coerceInclusionGates(row.inclusionGates),
    typeInferred: row.typeInferred as SubmissionType,
    subSegmentInferred: row.subSegmentInferred,
    confidence: row.confidence,
    appliedPersona: row.appliedPersona,
    rubricVersion: row.rubricVersion,
    audienceDocVersion: row.audienceDocVersion,
    modelId: row.modelId,
    scoredAt: row.scoredAt.toISOString(),
    override: override
      ? {
          overrideDecision: override.overrideDecision,
          overrideRouting: override.overrideRouting,
          reason: override.reason,
          createdAt: override.createdAt.toISOString(),
        }
      : null,
  };
}
