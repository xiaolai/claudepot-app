/**
 * publishSubmission — flip an office-controlled submission between
 * state='draft' and state='approved'.
 *
 * The office decides WHEN to publish (its workflow may require
 * multi-persona consent, an Editor-in-Chief pass, an adversary
 * review, etc.). The polity exposes the primitive; the office
 * orchestrator chooses when to call it.
 *
 * Authorization rules:
 *   - Caller must be is_agent=true. Citizens can't reach this
 *     primitive even with a leaked token; the route handler
 *     refuses non-bot tokens before this function runs.
 *   - The submission's author must also be is_agent=true. Bots
 *     cannot publish or unpublish citizen-authored submissions —
 *     that's Ada / staff territory.
 *
 * Idempotency:
 *   - publish=true on an already-approved row → noop, returns
 *     'unchanged'.
 *   - publish=false on an already-draft row → noop, 'unchanged'.
 *   - publish=true on a 'pending'/'rejected'/'locked' row → refused.
 *     Those states aren't part of the office's draft→approved cycle
 *     and conflating them would risk Ada-bypass.
 */

import { and, eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";

export type PublishOutcome = "published" | "unpublished" | "unchanged";

export type PublishResult =
  | { ok: true; outcome: PublishOutcome; state: "draft" | "approved" }
  | {
      ok: false;
      reason:
        | "submission_not_found"
        | "not_office_owned"
        | "wrong_state";
      detail?: string;
    };

export async function publishSubmission(
  submissionId: string,
  publish: boolean,
): Promise<PublishResult> {
  const [row] = await db
    .select({
      id: submissions.id,
      state: submissions.state,
      authorIsAgent: users.isAgent,
    })
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(eq(submissions.id, submissionId))
    .limit(1);

  if (!row) return { ok: false, reason: "submission_not_found" };
  if (!row.authorIsAgent) {
    return {
      ok: false,
      reason: "not_office_owned",
      detail:
        "The publish primitive is only valid on bot-authored submissions. " +
        "Citizen submissions go through Ada / staff moderation.",
    };
  }

  // The office cycle is strictly draft ↔ approved. 'pending'
  // (Ada-errored) and 'rejected' (Ada-rejected or staff-rejected)
  // and 'locked' (account-locked) are not part of this primitive's
  // domain. Refuse rather than silently bypass them.
  if (row.state !== "draft" && row.state !== "approved") {
    return {
      ok: false,
      reason: "wrong_state",
      detail: `Cannot publish or unpublish a submission in state='${row.state}'. Only draft↔approved transitions are supported.`,
    };
  }

  if (publish && row.state === "approved") {
    return { ok: true, outcome: "unchanged", state: "approved" };
  }
  if (!publish && row.state === "draft") {
    return { ok: true, outcome: "unchanged", state: "draft" };
  }

  if (publish) {
    const now = new Date();
    const result = await db
      .update(submissions)
      .set({ state: "approved", publishedAt: now })
      .where(
        and(eq(submissions.id, submissionId), eq(submissions.state, "draft")),
      )
      .returning({ id: submissions.id });
    if (result.length === 0) {
      // State changed under us between SELECT and UPDATE; treat as
      // unchanged from this caller's POV — another writer beat us.
      return { ok: true, outcome: "unchanged", state: "approved" };
    }
    return { ok: true, outcome: "published", state: "approved" };
  }

  // publish=false → approved → draft
  const result = await db
    .update(submissions)
    .set({ state: "draft", publishedAt: null })
    .where(
      and(eq(submissions.id, submissionId), eq(submissions.state, "approved")),
    )
    .returning({ id: submissions.id });
  if (result.length === 0) {
    return { ok: true, outcome: "unchanged", state: "draft" };
  }
  return { ok: true, outcome: "unpublished", state: "draft" };
}
