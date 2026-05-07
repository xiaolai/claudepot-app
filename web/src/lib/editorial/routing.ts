import type {
  CriterionId,
  FinalDecision,
  Persona,
  Routing,
  Rubric,
  ScoreResponse,
} from "./types";

export interface RoutingResult {
  weighted_total: number;
  final_decision: FinalDecision;
  routing: Routing;
}

export function computeWeightedTotal(
  scores: Record<CriterionId, number>,
  rubric: Rubric,
  persona: Persona
): number {
  const multipliers =
    persona === "base" ? {} : rubric.persona_overlays[persona].multipliers;

  let total = 0;
  for (const [criterion, score] of Object.entries(scores) as [CriterionId, number][]) {
    const weight = rubric.quality_score[criterion].weight;
    const multiplier = multipliers[criterion] ?? 1.0;
    total += score * weight * multiplier;
  }
  return total;
}

export function decideRouting(
  response: ScoreResponse,
  rubric: Rubric,
  persona: Persona
): RoutingResult {
  if (response.hard_rejects_hit.length > 0) {
    return { weighted_total: 0, final_decision: "reject", routing: "firehose" };
  }

  const gates = response.inclusion_gates;
  const allGatesPass =
    gates.primary_source_identifiable &&
    gates.testable_or_demonstrable &&
    gates.actionable_within_one_week &&
    gates.within_recency_window;

  if (!allGatesPass) {
    return { weighted_total: 0, final_decision: "reject", routing: "firehose" };
  }

  const total = computeWeightedTotal(response.scores, rubric, persona);
  const { feed_threshold, borderline_threshold } = rubric.routing;
  const borderlineLow = feed_threshold - borderline_threshold;
  const borderlineHigh = feed_threshold + borderline_threshold;

  if (total < borderlineLow) {
    return { weighted_total: total, final_decision: "reject", routing: "firehose" };
  }
  if (total <= borderlineHigh) {
    return {
      weighted_total: total,
      final_decision: "borderline_to_human_queue",
      routing: "human_queue",
    };
  }
  return { weighted_total: total, final_decision: "accept", routing: "feed" };
}
