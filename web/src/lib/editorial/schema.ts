import { z } from "zod";

export const ScoreResponseSchema = z.object({
  hard_rejects_hit: z.array(z.string()),
  inclusion_gates: z.object({
    primary_source_identifiable: z.boolean(),
    testable_or_demonstrable: z.boolean(),
    actionable_within_one_week: z.boolean(),
    within_recency_window: z.boolean(),
  }),
  scores: z.object({
    mechanism_specificity: z.number().int().min(0).max(5),
    evidence_quality: z.number().int().min(0).max(5),
    practitioner_fit: z.number().int().min(0).max(5),
    domain_legibility: z.number().int().min(0).max(3),
    counter_current: z.number().int().min(0).max(3),
    author_credibility: z.number().int().min(0).max(3),
    recency_bonus: z.number().int().min(0).max(3),
    diversity_bonus: z.number().int().min(0).max(3),
  }),
  type_inferred: z.enum([
    "news",
    "release",
    "tool",
    "podcast",
    "tutorial",
    "paper",
    "interview",
    "discussion",
    "workflow",
    "case_study",
    "prompt_pattern",
  ]),
  sub_segment_inferred: z.enum([
    "knowledge_workers",
    "engineers",
    "operators",
    "learners",
    "cross_cutting",
  ]),
  confidence: z.enum(["high", "low"]),
  one_line_why: z.string().min(1),
});

export type ParsedScoreResponse = z.infer<typeof ScoreResponseSchema>;
