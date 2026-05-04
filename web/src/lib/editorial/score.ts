import type { DecisionRecord, Persona, ScoreResponse, SubmissionInput } from "./types";
import { loadAudienceDoc, loadRubric, loadRubricForPrompt } from "./rubric";
import { buildSystemPrompt, buildUserPrompt } from "./prompt";
import { scoreWithClaude } from "./claude";
import { decideRouting } from "./routing";
import { ScoreResponseSchema } from "./schema";

export interface ScoreOptions {
  persona?: Persona;
}

export async function score(
  submission: SubmissionInput,
  options: ScoreOptions = {}
): Promise<DecisionRecord> {
  const persona = options.persona ?? "base";
  const rubric = loadRubric();
  const audience = loadAudienceDoc();
  const rubricForPrompt = loadRubricForPrompt();

  const systemPrompt = buildSystemPrompt(audience, rubricForPrompt);
  const userPrompt = buildUserPrompt(submission, persona);

  const raw = await scoreWithClaude(systemPrompt, userPrompt);
  const validated: ScoreResponse = ScoreResponseSchema.parse(raw);

  const { weighted_total, final_decision, routing } = decideRouting(
    validated,
    rubric,
    persona
  );

  return {
    rubric_version: rubric.version,
    audience_doc_version: rubric.audience.doc_version_pinned,
    applied_persona: persona,
    per_criterion_scores: validated.scores,
    weighted_total,
    hard_rejects_hit: validated.hard_rejects_hit,
    inclusion_gates: validated.inclusion_gates,
    type_inferred: validated.type_inferred,
    sub_segment_inferred: validated.sub_segment_inferred,
    confidence: validated.confidence,
    one_line_why: validated.one_line_why,
    final_decision,
    routing,
    source_url: submission.source_url,
    scored_at: new Date().toISOString(),
  };
}

export type { DecisionRecord, Persona, SubmissionInput, ScoreResponse } from "./types";
