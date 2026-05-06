/**
 * Author-only comment delete.
 *
 * Atomic: lock the row being deleted FOR UPDATE, then check for
 * children, then act. createComment locks the parent FOR SHARE
 * when inserting a reply, so an in-flight reply blocks until we
 * commit (and vice versa). Without this serialization, a hard
 * delete + concurrent reply could orphan the parent_id — there is
 * no FK on comments.parent_id, so the orphan would not be caught.
 */

import { revalidatePath } from "next/cache";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { comments } from "@/db/schema";
import type { DeleteCommentResult } from "./schema";

export async function deleteCommentAsAuthor(
  authorId: string,
  commentId: string,
): Promise<DeleteCommentResult> {
  type Outcome =
    | { kind: "ok"; submissionId: string }
    | { kind: "not_found" }
    | { kind: "forbidden" };

  const outcome = await db.transaction(async (tx): Promise<Outcome> => {
    const [existing] = await tx
      .select({
        authorId: comments.authorId,
        submissionId: comments.submissionId,
      })
      .from(comments)
      .where(eq(comments.id, commentId))
      .limit(1)
      .for("update");
    if (!existing) return { kind: "not_found" };
    if (existing.authorId !== authorId) return { kind: "forbidden" };

    const [reply] = await tx
      .select({ id: comments.id })
      .from(comments)
      .where(eq(comments.parentId, commentId))
      .limit(1);

    if (reply) {
      // Replies exist — soft delete (tombstone preserved).
      await tx
        .update(comments)
        .set({ deletedAt: new Date() })
        .where(eq(comments.id, commentId));
    } else {
      // No replies — hard delete.
      await tx.delete(comments).where(eq(comments.id, commentId));
    }
    return { kind: "ok", submissionId: existing.submissionId };
  });

  if (outcome.kind === "not_found") return { ok: false, reason: "not_found" };
  if (outcome.kind === "forbidden") return { ok: false, reason: "forbidden" };

  revalidatePath(`/post/${outcome.submissionId}`);
  return { ok: true, submissionId: outcome.submissionId };
}
