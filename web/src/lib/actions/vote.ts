"use server";

import { revalidatePath } from "next/cache";
import { and, eq, isNull } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { saves, submissions, users, votes } from "@/db/schema";

const KARMA_DOWNVOTE_THRESHOLD = 100;

const voteInput = z.object({
  submissionId: z.uuid(),
  value: z.union([z.literal(1), z.literal(-1), z.literal(0)]),
});

export type VoteResult =
  | { ok: true; value: 1 | -1 | 0 }
  | {
      ok: false;
      reason:
        | "unauth"
        | "validation"
        | "karma_gate"
        | "locked"
        | "not_found";
    };

export async function vote(input: unknown): Promise<VoteResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = voteInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };
  const { submissionId, value } = parsed.data;

  const [voter] = await db
    .select({ karma: users.karma, role: users.role })
    .from(users)
    .where(eq(users.id, session.user.id))
    .limit(1);
  // Audit finding 3.4 — distinguish missing-user (auth desync) from locked.
  if (!voter) return { ok: false, reason: "unauth" };
  if (voter.role === "locked") return { ok: false, reason: "locked" };

  // Karma-gated downvote.
  if (
    value === -1 &&
    voter.role !== "staff" &&
    voter.karma < KARMA_DOWNVOTE_THRESHOLD
  ) {
    return { ok: false, reason: "karma_gate" };
  }

  // Defense in depth: vote UI is only rendered on approved + non-deleted
  // submissions, but the action can be invoked directly (stale client,
  // raw fetch, future PAT). Confirm the target is votable before
  // persisting so moderated content cannot keep accumulating signal.
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
    // Clear the vote (the trigger reverses score automatically).
    await db
      .delete(votes)
      .where(
        and(eq(votes.userId, session.user.id), eq(votes.submissionId, submissionId)),
      );
  } else {
    // Upsert; the trigger handles INSERT vs UPDATE deltas.
    await db
      .insert(votes)
      .values({ userId: session.user.id, submissionId, value })
      .onConflictDoUpdate({
        target: [votes.userId, votes.submissionId],
        set: { value, createdAt: new Date() },
      });
  }

  revalidatePath(`/post/${submissionId}`);
  return { ok: true, value };
}

/* ── save (private bookmark, orthogonal to vote) ──────────────── */

const saveInput = z.object({
  submissionId: z.uuid(),
  saved: z.boolean(),
});

export type SaveResult =
  | { ok: true; saved: boolean }
  | { ok: false; reason: "unauth" | "validation" | "not_found" };

export async function save(input: unknown): Promise<SaveResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = saveInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  if (parsed.data.saved) {
    // Confirm the target exists and is visible before inserting.
    // Without this check a crafted UUID would reach the FK-backed
    // insert and bubble a server error instead of a typed result;
    // and saves on rejected/deleted submissions are nonsense for
    // the same reason votes on them are (defense in depth).
    const [target] = await db
      .select({ id: submissions.id })
      .from(submissions)
      .where(
        and(
          eq(submissions.id, parsed.data.submissionId),
          eq(submissions.state, "approved"),
          isNull(submissions.deletedAt),
        ),
      )
      .limit(1);
    if (!target) return { ok: false, reason: "not_found" };

    await db
      .insert(saves)
      .values({ userId: session.user.id, submissionId: parsed.data.submissionId })
      .onConflictDoNothing();
  } else {
    await db
      .delete(saves)
      .where(
        and(
          eq(saves.userId, session.user.id),
          eq(saves.submissionId, parsed.data.submissionId),
        ),
      );
  }

  revalidatePath(`/post/${parsed.data.submissionId}`);
  revalidatePath("/saved");
  return { ok: true, saved: parsed.data.saved };
}
