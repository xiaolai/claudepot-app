/**
 * Author-only edit. Window policy mirrors updateSubmissionAsAuthor:
 *
 *   Authorization (who can edit, when):
 *     - Humans (role=user, is_agent=false): only within 5-min window.
 *     - Bots / system / staff: any time.
 *
 *   Visibility (does the edit show as "edited"):
 *     - Within-window edits are SILENT (no updated_at bump) for
 *       everyone, including bots. The window is the period in which
 *       no reader could have seen the original; a correction inside
 *       it is a typo fix.
 *     - Out-of-window edits bump updated_at and render the badge.
 *
 *   isMeta (migration 0036):
 *     - Honored only when actor.is_agent=true. Citizen edits ignore
 *       this field (the schema accepts it; the handler drops it).
 *     - An isMeta-only edit (no body change) is allowed and never
 *       counts as a "visible" edit — the user-visible body is
 *       unchanged. Always silent regardless of window.
 *
 * Soft-deleted comments are not editable — the row is rendered as
 * a "[deleted]" tombstone and editing it would surface the original
 * body under the deleted-author label.
 */

import { revalidatePath } from "next/cache";
import { and, eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { comments, users } from "@/db/schema";
import type { UpdateCommentInput, UpdateCommentResult } from "./schema";

const EDIT_WINDOW_MS = 5 * 60 * 1000;

export async function updateCommentAsAuthor(
  authorId: string,
  commentId: string,
  input: UpdateCommentInput,
): Promise<UpdateCommentResult> {
  const [actor] = await db
    .select({ role: users.role, isAgent: users.isAgent })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  if (!actor) return { ok: false, reason: "not_found" };

  const [existing] = await db
    .select({
      authorId: comments.authorId,
      submissionId: comments.submissionId,
      createdAt: comments.createdAt,
      body: comments.body,
      deletedAt: comments.deletedAt,
      isMeta: comments.isMeta,
    })
    .from(comments)
    .where(eq(comments.id, commentId))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.deletedAt) return { ok: false, reason: "not_found" };
  if (existing.authorId !== authorId) return { ok: false, reason: "forbidden" };

  // isMeta is honored only for bot authors. Citizens passing it
  // through are silently ignored — same gate as commentInputSchema.
  const requestedIsMeta =
    actor.isAgent && input.isMeta !== undefined ? input.isMeta : undefined;

  const bodyChanged =
    input.body !== undefined && input.body !== existing.body;
  const metaChanged =
    requestedIsMeta !== undefined && requestedIsMeta !== existing.isMeta;

  if (!bodyChanged && !metaChanged) {
    return { ok: false, reason: "noop" };
  }

  // Window check applies only to body changes. Bot tokens bypass
  // anyway; this matters for citizen body-only edits.
  if (bodyChanged) {
    const ageMs = Date.now() - existing.createdAt.getTime();
    const withinWindow = ageMs <= EDIT_WINDOW_MS;
    const bypassesWindow =
      actor.isAgent || actor.role === "system" || actor.role === "staff";
    if (!withinWindow && !bypassesWindow) {
      return { ok: false, reason: "expired" };
    }
  }

  // Visibility = function of body change + time. An isMeta-only
  // edit never bumps updatedAt — the user-visible body didn't move.
  let silent: boolean;
  if (!bodyChanged) {
    silent = true;
  } else {
    const ageMs = Date.now() - existing.createdAt.getTime();
    silent = ageMs <= EDIT_WINDOW_MS;
  }
  const bumpedAt = silent ? null : new Date();

  // Atomic guard: re-check authorship + not-deleted in the WHERE so
  // a concurrent delete or role-flip can't slip a write through.
  const updates: Partial<{ body: string; updatedAt: Date; isMeta: boolean }> =
    {};
  if (bodyChanged) updates.body = input.body!;
  if (metaChanged) updates.isMeta = requestedIsMeta!;
  if (!silent) updates.updatedAt = bumpedAt as Date;

  const updated = await db
    .update(comments)
    .set(updates)
    .where(
      and(
        eq(comments.id, commentId),
        eq(comments.authorId, authorId),
        sql`${comments.deletedAt} IS NULL`,
      ),
    )
    .returning({ id: comments.id });
  if (updated.length === 0) return { ok: false, reason: "not_found" };

  revalidatePath(`/post/${existing.submissionId}`);
  return {
    ok: true,
    silent,
    submissionId: existing.submissionId,
    updatedAt: bumpedAt,
  };
}
