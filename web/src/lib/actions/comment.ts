"use server";

import { auth } from "@/lib/auth";
import {
  commentInputSchema,
  createComment,
  deleteCommentAsAuthor,
  updateCommentAsAuthor,
  updateCommentInputSchema,
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

/* ── Edit — thin wrapper over updateCommentAsAuthor ──────────── */

export async function editComment(
  input: unknown,
): Promise<
  | { ok: true }
  | { ok: false; reason: "unauth" | "not_found" | "forbidden" | "expired" | "validation" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  if (typeof input !== "object" || input === null || !("id" in input)) {
    return { ok: false, reason: "validation" };
  }
  const { id, ...rest } = input as { id: unknown } & Record<string, unknown>;
  if (typeof id !== "string") return { ok: false, reason: "validation" };

  const parsed = updateCommentInputSchema.safeParse(rest);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const result = await updateCommentAsAuthor(session.user.id, id, parsed.data);
  if (!result.ok) {
    if (result.reason === "noop") return { ok: true };
    return { ok: false, reason: result.reason };
  }
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
