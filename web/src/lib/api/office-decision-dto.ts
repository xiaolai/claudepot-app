/**
 * Public-shape DTO for /api/v1/submissions/{id}/decisions.
 *
 * The OfficeDecision struct in db/office-queries.ts is shaped for
 * server-rendered /office/ pages, which already pick which fields
 * to render. The JSON public-list endpoint cannot rely on that
 * downstream filter — it dumps whatever the struct carries. This
 * builder enforces the privacy contract from
 * editorial/transparency.md §1 explicitly:
 *
 *   - Per-criterion scores and the one-line why are public.
 *   - Weights, persona multipliers, and the weighted total stay
 *     private (paired with scores, the total leaks the weights).
 *   - Model id is internal ops; not surfaced on the SSR pages.
 *
 * Mirror this file when adding a new public read of decision rows.
 */

import type { OfficeDecision } from "@/db/office-queries";

type DecisionFinal = "accept" | "reject" | "borderline_to_human_queue";
type DecisionRouting = "feed" | "firehose" | "human_queue";

export type PublicOverrideDto = {
  overrideDecision: DecisionFinal;
  overrideRouting: DecisionRouting;
  reviewerKind: "human" | "bot";
  reason: string;
  createdAt: string;
};

export type PublicOfficeDecisionDto = {
  id: string;
  submissionId: string;
  appliedPersona: string;
  perCriterionScores: Record<string, number>;
  hardRejectsHit: string[];
  inclusionGates: Record<string, boolean>;
  typeInferred: string;
  subSegmentInferred: string;
  confidence: "high" | "low";
  oneLineWhy: string;
  finalDecision: DecisionFinal;
  routing: DecisionRouting;
  effectiveRouting: DecisionRouting;
  rubricVersion: string;
  audienceDocVersion: string;
  scoredAt: string;
  latestOverride: PublicOverrideDto | null;
};

export function buildPublicOfficeDecisionDto(
  d: OfficeDecision,
): PublicOfficeDecisionDto {
  return {
    id: d.id,
    submissionId: d.submissionId,
    appliedPersona: d.appliedPersona,
    perCriterionScores: d.perCriterionScores,
    hardRejectsHit: d.hardRejectsHit,
    inclusionGates: d.inclusionGates,
    typeInferred: d.typeInferred,
    subSegmentInferred: d.subSegmentInferred,
    confidence: d.confidence,
    oneLineWhy: d.oneLineWhy,
    finalDecision: d.finalDecision,
    routing: d.routing,
    // Effective routing folds in the latest override so consumers
    // don't have to choose between the original verdict and the
    // post-override state. UI surfaces should render BOTH (history
    // matters), but a CLI scraper that only wants "where did this
    // submission end up?" reads effectiveRouting and is right.
    effectiveRouting: d.latestOverride?.overrideRouting ?? d.routing,
    rubricVersion: d.rubricVersion,
    audienceDocVersion: d.audienceDocVersion,
    scoredAt: d.scoredAt.toISOString(),
    latestOverride: d.latestOverride
      ? {
          overrideDecision: d.latestOverride.overrideDecision,
          overrideRouting: d.latestOverride.overrideRouting,
          reviewerKind: d.latestOverride.reviewerKind,
          reason: d.latestOverride.reason,
          createdAt: d.latestOverride.createdAt.toISOString(),
        }
      : null,
  };
}
