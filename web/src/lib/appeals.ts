/**
 * Core appeal-submission logic.
 *
 * Lives in lib/ (not lib/actions/) so both surfaces share it:
 *
 *   - Web UI server action (lib/actions/appeals.ts:submitAppeal) calls
 *     this with the cookie-authenticated user id.
 *   - REST endpoint (app/api/v1/appeals/route.ts) calls it with the
 *     PAT-authenticated user id.
 *
 * Auth happens at each surface's boundary; this function trusts the
 * authorId it's given.
 *
 * Reuses the `flags` table with a tagged reason ("appeal: …") so
 * staff sees appeals in /admin/queue alongside community flags. The
 * email-verified gate that the public flag action enforces is
 * skipped here — a freshly-rejected user may not have verified yet
 * and denying them an appeal path defeats the moderator's
 * accountability.
 *
 * One-appeal-per-decision is enforced application-side: if an open
 * flag tagged "appeal:" already references the same target, the new
 * attempt is rejected.
 */

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { flags, policyDecisions, submissions } from "@/db/schema";

export const appealInputSchema = z.object({
  decisionId: z.uuid(),
  text: z.string().trim().min(10).max(1000),
});

export type AppealInput = z.infer<typeof appealInputSchema>;

export type AppealCoreResult =
  | { ok: true; flagId: string }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "duplicate" | "stale";
    };

export async function submitAppealAsAuthor(
  authorId: string,
  input: AppealInput,
): Promise<AppealCoreResult> {
  const { decisionId, text } = input;

  const [decision] = await db
    .select({
      id: policyDecisions.id,
      authorId: policyDecisions.authorId,
      targetType: policyDecisions.targetType,
      targetId: policyDecisions.targetId,
      verdict: policyDecisions.verdict,
    })
    .from(policyDecisions)
    .where(eq(policyDecisions.id, decisionId))
    .limit(1);

  if (!decision) return { ok: false, reason: "not_found" };
  if (decision.authorId !== authorId) {
    return { ok: false, reason: "forbidden" };
  }
  if (decision.verdict !== "reject") {
    // Appeals only make sense on rejects — a `pass` decision has
    // nothing to appeal. Surface as `stale` so callers can show a
    // useful message.
    return { ok: false, reason: "stale" };
  }
  if (!decision.targetId) {
    // illegal-comment block path: the comment was never inserted,
    // so there's no row for staff to act on. These appeals require
    // a different (manual / email) path; surface as stale.
    return { ok: false, reason: "stale" };
  }

  // For submissions, also confirm the submission still exists and
  // is in 'rejected' state — if staff already approved or the row
  // was deleted, an appeal is moot.
  if (decision.targetType === "submission") {
    const [sub] = await db
      .select({ state: submissions.state, deletedAt: submissions.deletedAt })
      .from(submissions)
      .where(eq(submissions.id, decision.targetId))
      .limit(1);
    if (!sub || sub.deletedAt) return { ok: false, reason: "stale" };
    if (sub.state !== "rejected") return { ok: false, reason: "stale" };
  }

  // Block duplicates: at most one open appeal flag per target.
  const [existing] = await db
    .select({ id: flags.id, reason: flags.reason })
    .from(flags)
    .where(
      and(
        eq(flags.targetType, decision.targetType),
        eq(flags.targetId, decision.targetId),
        eq(flags.status, "open"),
      ),
    )
    .limit(1);
  if (existing && existing.reason.startsWith("appeal:")) {
    return { ok: false, reason: "duplicate" };
  }

  const [row] = await db
    .insert(flags)
    .values({
      reporterId: authorId,
      targetType: decision.targetType,
      targetId: decision.targetId,
      reason: `appeal: ${text}`.slice(0, 500),
    })
    .returning({ id: flags.id });

  revalidatePath("/admin/queue");
  revalidatePath(`/appeal/${decisionId}`);

  return { ok: true, flagId: row.id };
}
