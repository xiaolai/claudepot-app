/**
 * Engagement event recording.
 *
 * Polity-side primitive: every vote / comment / save fires a
 * fire-and-forget INSERT into engagement_records. The office reads
 * the resulting time-series via GET /api/v1/submissions/{id}/engagement
 * to drive its analytics (Layer 3 EIC drift watch, Layer 5 adversary
 * picks, etc.).
 *
 * Design choices:
 *   - The recorder is best-effort — it never throws into the calling
 *     handler. A failed engagement INSERT must NOT roll back the vote
 *     it accompanies; the public counter (already updated via the
 *     vote trigger) is the source of truth, and engagement_records
 *     is a parallel audit. Errors are warned to console.
 *   - kind is free-form text on the schema (matches the
 *     applied_persona convention). The polity emits a closed set of
 *     primitive kinds: 'vote', 'comment', 'save'. The office can
 *     append semantic kinds via POST /api/v1/engagement
 *     (engagement:write scope). Both share the same table.
 *   - actor_id is recorded but NEVER returned by the public read
 *     endpoint — privacy. See app/api/v1/submissions/[id]/engagement.
 *   - metadata is jsonb on the schema; the recorder accepts a
 *     plain object. The public read endpoint omits metadata too;
 *     office consumers read the table directly via SQL or a future
 *     scoped endpoint.
 */

import { db } from "@/db/client";
import { engagementRecords } from "@/db/schema";

export type PrimitiveEngagementKind = "vote" | "comment" | "save";

export type RecordEngagementInput = {
  submissionId: string;
  kind: string; // open vocabulary; primitive kinds above are conventions
  actorId: string | null;
  metadata?: Record<string, unknown>;
};

/**
 * Record one engagement event. Best-effort: errors are logged but
 * never propagated. Callers should NOT await this for correctness —
 * the existing counter triggers and table writes are the source
 * of truth for vote / comment / save state.
 */
export async function recordEngagement(
  input: RecordEngagementInput,
): Promise<void> {
  try {
    await db.insert(engagementRecords).values({
      submissionId: input.submissionId,
      kind: input.kind,
      actorId: input.actorId,
      metadata: input.metadata ?? null,
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.warn(
      `[engagement] failed to record kind='${input.kind}' on submission=${input.submissionId}: ${msg}`,
    );
  }
}
