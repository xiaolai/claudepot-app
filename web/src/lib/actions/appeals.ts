"use server";

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { flags, policyDecisions, submissions } from "@/db/schema";

/**
 * User-facing appeal of an AI policy moderator reject.
 *
 * Reuses the `flags` table with a tagged reason ("appeal: …") so
 * staff sees it in /admin/queue alongside community flags. We do
 * NOT gate on email_verified the way the public flag action does:
 * a freshly-rejected user may not have verified yet, and denying
 * them an appeal path defeats the point of the moderator being
 * accountable.
 *
 * One-appeal-per-decision is enforced application-side: if an open
 * flag tagged "appeal:" already references the same submission,
 * we reject the new attempt.
 */

const appealInput = z.object({
  decisionId: z.uuid(),
  text: z.string().trim().min(10).max(1000),
});

export type AppealResult =
  | { ok: true; flagId: string }
  | {
      ok: false;
      reason:
        | "unauth"
        | "not_found"
        | "forbidden"
        | "validation"
        | "duplicate"
        | "stale";
    };

export async function submitAppeal(input: unknown): Promise<AppealResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = appealInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const { decisionId, text } = parsed.data;

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
  if (decision.authorId !== session.user.id) {
    return { ok: false, reason: "forbidden" };
  }
  if (decision.verdict !== "reject") {
    // Appeals only make sense on rejects — a `pass` decision has
    // nothing to appeal. Surface as `stale` so the caller can show
    // a useful message.
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

  // Block duplicates: at most one open appeal flag per (target).
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
      reporterId: session.user.id,
      targetType: decision.targetType,
      targetId: decision.targetId,
      reason: `appeal: ${text}`.slice(0, 500),
    })
    .returning({ id: flags.id });

  revalidatePath("/admin/queue");
  revalidatePath(`/appeal/${decisionId}`);

  return { ok: true, flagId: row.id };
}
