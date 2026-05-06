/**
 * Mock human-review queue + AI audit log for prototype views.
 *
 * The real version of these surfaces — populated from the
 * decision_records and policy_decisions tables — replaces this
 * module once the production read paths are wired (Slice 1b for
 * policy aggregates, ongoing for editorial).
 */

import type { AuditEntry, ModQueueItem } from "./types";

/**
 * Human review queue — populated when AI confidence is low or a user flags
 * an already-published item, or when the author appeals a rejection.
 */
export function getModQueue(): ModQueueItem[] {
  return [
    {
      id: "q1",
      target_type: "submission",
      target_id: "p-pending-1",
      trigger: "low-confidence",
      ai_confidence: 0.62,
      ai_proposed_action: "approve",
      ai_reason:
        "Borderline self-promotion. Not a clear policy violation but the user has 3 prior posts to the same domain in the last week.",
      at: "2026-04-29T13:48:00Z",
    },
    {
      id: "q2",
      target_type: "submission",
      target_id: "p-rejected-1",
      trigger: "appeal",
      ai_confidence: 0.91,
      ai_proposed_action: "reject",
      ai_reason:
        "Affiliate link in body without disclosure. Author appealed, claims it's a personal-use tracker, not affiliate.",
      flagged_by: "ren",
      at: "2026-04-29T12:15:00Z",
    },
    {
      id: "q3",
      target_type: "comment",
      target_id: "c5-3",
      trigger: "user-flag",
      ai_confidence: 0.78,
      ai_proposed_action: "approve",
      ai_reason:
        "Heated but on-topic. User-flagged as personal attack; AI reads it as direct critique without ad hominem.",
      flagged_by: "ada",
      at: "2026-04-28T11:00:00Z",
    },
    {
      id: "q4",
      target_type: "submission",
      target_id: "p-pending-2",
      trigger: "low-confidence",
      ai_confidence: 0.71,
      ai_proposed_action: "approve",
      ai_reason:
        "Topic relevance uncertain — post is about general LLM ops, not Claude-specific. Could go either way.",
      at: "2026-04-29T14:22:00Z",
    },
  ];
}

export function getAuditLog(): AuditEntry[] {
  return [
    {
      id: "a1",
      target_type: "submission",
      target_id: "1",
      action: "approve",
      reason: "Official Anthropic news; tagged release-watch + long-context.",
      confidence: 0.99,
      decided_at: "2026-04-28T09:14:30Z",
    },
    {
      id: "a2",
      target_type: "submission",
      target_id: "p-rejected-1",
      action: "reject",
      reason: "Affiliate link in body without disclosure (rule 4).",
      confidence: 0.91,
      decided_at: "2026-04-29T11:50:00Z",
    },
    {
      id: "a3",
      target_type: "submission",
      target_id: "p-pending-2",
      action: "approve",
      reason: "Topic relevance borderline; routed to human queue.",
      confidence: 0.71,
      decided_at: "2026-04-29T14:22:00Z",
    },
    {
      id: "a4",
      target_type: "comment",
      target_id: "c8-2",
      action: "reject",
      reason: "Spam — third near-identical promotional reply this week.",
      confidence: 0.97,
      decided_at: "2026-04-29T07:42:00Z",
      overridden: {
        by: "ada",
        new_action: "approve",
        at: "2026-04-29T08:10:00Z",
        note: "False positive — author is a known contributor; spam classifier mis-fired on URL pattern.",
      },
    },
    {
      id: "a5",
      target_type: "submission",
      target_id: "30",
      action: "approve",
      reason: "Clear practical tip; tagged prompt-caching.",
      confidence: 0.94,
      decided_at: "2026-04-29T10:18:30Z",
    },
  ];
}
