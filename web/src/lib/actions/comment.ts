"use server";

import { revalidatePath } from "next/cache";
import { eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { comments } from "@/db/schema";
import {
  commentInputSchema,
  createComment,
  deleteCommentAsAuthor,
  type CommentResult as CoreCommentResult,
} from "@/lib/comments";

// Web action result includes "unauth" — REST/MCP surfaces handle auth
// before they call the core, so they never see it.
export type CommentResult =
  | CoreCommentResult
  | { ok: false; reason: "unauth" };

export async function submitComment(input: unknown): Promise<CommentResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = commentInputSchema.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  return createComment(session.user.id, parsed.data);
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

  const result = await deleteCommentAsAuthor(session.user.id, id);
  if (!result.ok) return result;
  return { ok: true };
}
