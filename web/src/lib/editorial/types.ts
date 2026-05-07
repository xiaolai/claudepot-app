export type SubmissionType =
  | "news"
  | "release"
  | "tool"
  | "podcast"
  | "tutorial"
  | "paper"
  | "interview"
  | "discussion"
  | "workflow"
  | "case_study"
  | "prompt_pattern";

export type SubSegment =
  | "knowledge_workers"
  | "engineers"
  | "operators"
  | "learners"
  | "cross_cutting";

export type Persona = "base" | "ada" | "historian" | "scout";

export type CriterionId =
  | "mechanism_specificity"
  | "evidence_quality"
  | "practitioner_fit"
  | "domain_legibility"
  | "counter_current"
  | "author_credibility"
  | "recency_bonus"
  | "diversity_bonus";

export type FinalDecision = "accept" | "reject" | "borderline_to_human_queue";

export type Routing = "feed" | "firehose" | "human_queue";

export interface SubmissionInput {
  title: string;
  body: string;
  source_url: string;
  type?: SubmissionType;
}

export interface InclusionGates {
  primary_source_identifiable: boolean;
  testable_or_demonstrable: boolean;
  actionable_within_one_week: boolean;
  within_recency_window: boolean;
}

export interface ScoreResponse {
  hard_rejects_hit: string[];
  inclusion_gates: InclusionGates;
  scores: Record<CriterionId, number>;
  type_inferred: SubmissionType;
  sub_segment_inferred: SubSegment;
  confidence: "high" | "low";
  one_line_why: string;
}

export interface DecisionRecord {
  rubric_version: string;
  audience_doc_version: string;
  applied_persona: Persona;
  per_criterion_scores: Record<CriterionId, number>;
  weighted_total: number;
  hard_rejects_hit: string[];
  inclusion_gates: InclusionGates;
  type_inferred: SubmissionType;
  sub_segment_inferred: SubSegment;
  confidence: "high" | "low";
  one_line_why: string;
  final_decision: FinalDecision;
  routing: Routing;
  source_url: string;
  scored_at: string;
}

export interface Rubric {
  version: string;
  audience: {
    doc: string;
    doc_version_pinned: string;
    sub_segment_ids: SubSegment[];
  };
  routing: {
    feed_threshold: number;
    borderline_threshold: number;
    destinations: Record<string, string>;
  };
  hard_rejects: { id: string; why: string }[];
  inclusion_gates: { id: string; check: string }[];
  quality_score: Record<CriterionId, { weight: number; scale: [number, number]; rubric: string }>;
  persona_overlays: Record<
    Exclude<Persona, "base">,
    { description: string; multipliers: Partial<Record<CriterionId, number>> }
  >;
}
