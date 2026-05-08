// 'draft' was added to the content_state enum in migration 0036
// for office-bot submissions awaiting an editorial decision.
// Comments never enter 'draft' even though they share the enum,
// so the union here covers the schema's full set of values.
export type ModerationState = "pending" | "approved" | "rejected" | "draft";

export interface AIDecision {
  reason: string;
  confidence: number;
  tags_assigned: string[];
  type_assigned?: string;
  decided_at: string;
}

interface HasState {
  state?: ModerationState;
}

interface ModerableSubmission extends HasState {
  ai_decision?: AIDecision;
  tags: string[];
  type: string;
  submitted_at: string;
}

/** Effective state — defaults to "approved" for legacy fixtures without state. */
export function effectiveState(s: HasState): ModerationState {
  return s.state ?? "approved";
}

/** Effective AI decision — synthesized for legacy fixtures without one. */
export function effectiveDecision(s: ModerableSubmission): AIDecision {
  if (s.ai_decision) return s.ai_decision;
  return {
    reason: "Auto-approved by quality classifier",
    confidence: 0.95,
    tags_assigned: s.tags,
    type_assigned: s.type,
    decided_at: s.submitted_at,
  };
}

export function commentEffectiveState(c: HasState): ModerationState {
  return c.state ?? "approved";
}
