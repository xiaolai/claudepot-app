"use server";

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { comments, notifications, submissions, users } from "@/db/schema";

const submitInput = z.object({
  submissionId: z.uuid(),
  parentId: z.uuid().nullable().optional(),
  body: z.string().trim().min(2).max(40_000),
});

export type CommentResult =
  | { ok: true; commentId: string; pending: boolean }
  | { ok: false; reason: "unauth" | "validation" | "not_found" | "locked" };

async function determineInitialState(
  authorId: string,
): Promise<"pending" | "approved"> {
  const [u] = await db
    .select({ role: users.role, karma: users.karma })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  if (!u || u.role === "locked") return "pending";
  if (u.role === "staff" || u.role === "system") return "approved";
  if (u.karma >= 50) return "approved";
  // For comments we use a softer gate than submissions: any user with at
  // least one approved submission is past first-comment review.
  const [hasApproved] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(
      and(eq(submissions.authorId, authorId), eq(submissions.state, "approved")),
    )
    .limit(1);
  return hasApproved ? "approved" : "pending";
}

export async function submitComment(input: unknown): Promise<CommentResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = submitInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const initialState = await determineInitialState(session.user.id);

  // The submission state read, the comment insert, and the notification
  // insert run in one transaction. Without this, a concurrent staff
  // lock/reject/delete could land between the read and the insert,
  // and a notification-write failure would leave the comment with no
  // recipient surface. SELECT ... FOR SHARE on the submission row
  // serializes against the staff UPDATEs in moderationAction, which
  // now share a transaction.
  type Outcome =
    | { kind: "ok"; commentId: string }
    | { kind: "not_found" }
    | { kind: "locked" };

  const outcome = await db.transaction(async (tx): Promise<Outcome> => {
    const [target] = await tx
      .select({
        id: submissions.id,
        authorId: submissions.authorId,
        lockedAt: submissions.lockedAt,
        state: submissions.state,
        deletedAt: submissions.deletedAt,
      })
      .from(submissions)
      .where(eq(submissions.id, parsed.data.submissionId))
      .limit(1)
      .for("share");
    if (!target) return { kind: "not_found" };
    // Reject + delete pretend the row isn't there for non-author/non-
    // staff viewers; mirror that here so a reply cannot be posted onto
    // either.
    if (target.deletedAt || target.state === "rejected") {
      return { kind: "not_found" };
    }
    // Audit finding 3.3 — lock blocks new comments.
    if (target.lockedAt) return { kind: "locked" };

    let parentAuthor: string | null = null;
    if (parsed.data.parentId) {
      const [parent] = await tx
        .select({
          authorId: comments.authorId,
          submissionId: comments.submissionId,
        })
        .from(comments)
        .where(eq(comments.id, parsed.data.parentId))
        .limit(1);
      if (!parent || parent.submissionId !== parsed.data.submissionId) {
        return { kind: "not_found" };
      }
      parentAuthor = parent.authorId;
    }

    const [row] = await tx
      .insert(comments)
      .values({
        authorId: session.user.id,
        submissionId: parsed.data.submissionId,
        parentId: parsed.data.parentId ?? null,
        body: parsed.data.body,
        state: initialState,
      })
      .returning({ id: comments.id });

    // Notify the parent comment's author or, for top-level, the
    // submission's author. Skip self-notifications.
    const notifyTarget = parentAuthor ?? target.authorId;
    if (notifyTarget && notifyTarget !== session.user.id) {
      await tx.insert(notifications).values({
        userId: notifyTarget,
        kind: parsed.data.parentId ? "comment_reply" : "submission_reply",
        payload: {
          commentId: row.id,
          submissionId: parsed.data.submissionId,
        },
      });
    }

    return { kind: "ok", commentId: row.id };
  });

  if (outcome.kind === "not_found") return { ok: false, reason: "not_found" };
  if (outcome.kind === "locked") return { ok: false, reason: "locked" };

  revalidatePath(`/post/${parsed.data.submissionId}`);
  return {
    ok: true,
    commentId: outcome.commentId,
    pending: initialState === "pending",
  };
}

/* ── Edit (5-minute window) ────────────────────────────────────── */

const EDIT_WINDOW_MS = 5 * 60 * 1000;

const editInput = z.object({
  id: z.uuid(),
  body: z.string().trim().min(2).max(40_000),
});

export async function editComment(
  input: unknown,
): Promise<
  | { ok: true }
  | { ok: false; reason: "unauth" | "not_found" | "forbidden" | "expired" | "validation" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = editInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const [existing] = await db
    .select({
      authorId: comments.authorId,
      submissionId: comments.submissionId,
      createdAt: comments.createdAt,
    })
    .from(comments)
    .where(eq(comments.id, parsed.data.id))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== session.user.id) return { ok: false, reason: "forbidden" };
  if (Date.now() - existing.createdAt.getTime() > EDIT_WINDOW_MS)
    return { ok: false, reason: "expired" };

  await db
    .update(comments)
    .set({ body: parsed.data.body })
    .where(eq(comments.id, parsed.data.id));
  revalidatePath(`/post/${existing.submissionId}`);
  return { ok: true };
}

/* ── Delete (soft if has replies, else hard) ───────────────────── */

export async function deleteComment(
  id: string,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "not_found" | "forbidden" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const [existing] = await db
    .select({
      authorId: comments.authorId,
      submissionId: comments.submissionId,
    })
    .from(comments)
    .where(eq(comments.id, id))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== session.user.id) return { ok: false, reason: "forbidden" };

  const [reply] = await db
    .select({ id: comments.id })
    .from(comments)
    .where(eq(comments.parentId, id))
    .limit(1);

  if (reply) {
    // Replies exist — soft delete (tombstone preserved).
    await db
      .update(comments)
      .set({ deletedAt: new Date() })
      .where(eq(comments.id, id));
  } else {
    // No replies — hard delete.
    await db.delete(comments).where(eq(comments.id, id));
  }

  revalidatePath(`/post/${existing.submissionId}`);
  return { ok: true };
}
