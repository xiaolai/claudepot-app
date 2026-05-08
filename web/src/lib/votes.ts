/**
 * Core vote + save operations.
 *
 * Auth happens at each surface's boundary; these functions trust the
 * userId they're given. See the lib/comments.ts header for the same
 * three-surface rationale.
 */

import { revalidatePath } from "next/cache";
import { and, eq, isNull } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { saves, submissions, users, votes } from "@/db/schema";
import { recordEngagement } from "@/lib/engagement";

const KARMA_DOWNVOTE_THRESHOLD = 100;

export const voteInputSchema = z.object({
  submissionId: z.uuid(),
  value: z.union([z.literal(1), z.literal(-1), z.literal(0)]),
});

export type VoteInput = z.infer<typeof voteInputSchema>;

export type VoteResult =
  | { ok: true; value: 1 | -1 | 0 }
  | {
      ok: false;
      reason: "karma_gate" | "locked" | "not_found" | "missing_user";
    };

export async function castVote(
  userId: string,
  input: VoteInput,
): Promise<VoteResult> {
  const { submissionId, value } = input;

  const [voter] = await db
    .select({ karma: users.karma, role: users.role })
    .from(users)
    .where(eq(users.id, userId))
    .limit(1);
  if (!voter) return { ok: false, reason: "missing_user" };
  if (voter.role === "locked") return { ok: false, reason: "locked" };

  if (
    value === -1 &&
    voter.role !== "staff" &&
    voter.karma < KARMA_DOWNVOTE_THRESHOLD
  ) {
    return { ok: false, reason: "karma_gate" };
  }

  // Defense in depth: votes only render on approved + non-deleted
  // submissions, but the action can be invoked directly. Confirm the
  // target is votable so moderated content cannot keep accumulating
  // signal.
  const [target] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(
      and(
        eq(submissions.id, submissionId),
        eq(submissions.state, "approved"),
        isNull(submissions.deletedAt),
      ),
    )
    .limit(1);
  if (!target) return { ok: false, reason: "not_found" };

  if (value === 0) {
    await db
      .delete(votes)
      .where(and(eq(votes.userId, userId), eq(votes.submissionId, submissionId)));
  } else {
    await db
      .insert(votes)
      .values({ userId, submissionId, value })
      .onConflictDoUpdate({
        target: [votes.userId, votes.submissionId],
        set: { value, createdAt: new Date() },
      });
  }

  // Engagement event for the office's analytics. Best-effort —
  // never blocks or rolls back the vote write itself.
  await recordEngagement({
    submissionId,
    kind: "vote",
    actorId: userId,
    metadata: { value },
  });

  revalidatePath(`/post/${submissionId}`);
  return { ok: true, value };
}

/* ── save (private bookmark, orthogonal to vote) ──────────────── */

export const saveInputSchema = z.object({
  submissionId: z.uuid(),
  saved: z.boolean(),
});

export type SaveInput = z.infer<typeof saveInputSchema>;

export type SaveResult =
  | { ok: true; saved: boolean }
  | { ok: false; reason: "not_found" };

export async function setSave(
  userId: string,
  input: SaveInput,
): Promise<SaveResult> {
  if (input.saved) {
    const [target] = await db
      .select({ id: submissions.id })
      .from(submissions)
      .where(
        and(
          eq(submissions.id, input.submissionId),
          eq(submissions.state, "approved"),
          isNull(submissions.deletedAt),
        ),
      )
      .limit(1);
    if (!target) return { ok: false, reason: "not_found" };

    await db
      .insert(saves)
      .values({ userId, submissionId: input.submissionId })
      .onConflictDoNothing();
    // Save events are recorded; unsaves intentionally are not — the
    // office cares about positive engagement signal, not undo noise.
    await recordEngagement({
      submissionId: input.submissionId,
      kind: "save",
      actorId: userId,
    });
  } else {
    await db
      .delete(saves)
      .where(
        and(
          eq(saves.userId, userId),
          eq(saves.submissionId, input.submissionId),
        ),
      );
  }

  revalidatePath(`/post/${input.submissionId}`);
  revalidatePath("/saved");
  return { ok: true, saved: input.saved };
}
