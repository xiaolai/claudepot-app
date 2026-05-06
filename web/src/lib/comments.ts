/**
 * Core comment operations.
 *
 * Lives in lib/ (not lib/actions/) because three surfaces need them:
 *
 *   - Web UI server actions (lib/actions/comment.ts) call these with the
 *     cookie-authenticated user id.
 *   - REST endpoints (app/api/v1/comments/*) call these with the
 *     PAT-authenticated user id.
 *   - MCP tools (lib/mcp/tools.ts) call these with the same.
 *
 * Auth happens at each surface's boundary; these functions trust the
 * authorId / userId they're given.
 *
 * Mirrors the lib/submissions.ts split — the web action retains its
 * legacy result shape ("unauth"), while the cores return the
 * surface-agnostic outcomes the API/MCP can translate to HTTP/text.
 */

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { comments, notifications, submissions, users } from "@/db/schema";

/* ── Schema ─────────────────────────────────────────────────────── */

export const commentInputSchema = z.object({
  submissionId: z.uuid(),
  parentId: z.uuid().nullable().optional(),
  body: z.string().trim().min(2).max(40_000),
});

export type CommentInput = z.infer<typeof commentInputSchema>;

export type CommentResult =
  | { ok: true; commentId: string; pending: boolean }
  | { ok: false; reason: "validation" | "not_found" | "locked" };

/* ── createComment ──────────────────────────────────────────────── */

async function determineInitialState(
  authorId: string,
): Promise<"pending" | "approved" | "locked"> {
  const [u] = await db
    .select({ role: users.role, karma: users.karma })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  if (!u) return "pending";
  // Mirror lib/submissions.ts: locked accounts are rejected outright,
  // not silently routed to the moderation queue. PAT-driven flooding
  // would otherwise consume reviewer time + the daily comments bucket.
  if (u.role === "locked") return "locked";
  if (u.role === "staff" || u.role === "system") return "approved";
  if (u.karma >= 50) return "approved";
  // Softer gate than submissions: any user with at least one approved
  // submission is past first-comment review.
  const [hasApproved] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(
      and(eq(submissions.authorId, authorId), eq(submissions.state, "approved")),
    )
    .limit(1);
  return hasApproved ? "approved" : "pending";
}

export async function createComment(
  authorId: string,
  input: CommentInput,
): Promise<CommentResult> {
  const initialState = await determineInitialState(authorId);
  if (initialState === "locked") return { ok: false, reason: "locked" };

  type Outcome =
    | { kind: "ok"; commentId: string }
    | { kind: "not_found" }
    | { kind: "locked" };

  // Single transaction with FOR SHARE on the target submission to
  // serialize against staff lock/reject UPDATEs and to keep the
  // notification insert atomic with the comment insert. The parent
  // comment (if any) is also locked FOR SHARE so deleteCommentAsAuthor's
  // FOR UPDATE serializes against an in-flight reply — without this,
  // a hard delete + reply race could leave an orphaned parent_id
  // (parent_id has no FK constraint on purpose).
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
      .where(eq(submissions.id, input.submissionId))
      .limit(1)
      .for("share");
    if (!target) return { kind: "not_found" };
    if (target.deletedAt || target.state === "rejected") {
      return { kind: "not_found" };
    }
    if (target.lockedAt) return { kind: "locked" };

    let parentAuthor: string | null = null;
    if (input.parentId) {
      const [parent] = await tx
        .select({
          authorId: comments.authorId,
          submissionId: comments.submissionId,
          state: comments.state,
          deletedAt: comments.deletedAt,
        })
        .from(comments)
        .where(eq(comments.id, input.parentId))
        .limit(1)
        .for("share");
      // Reject replies to non-visible parents (deleted, rejected, or
      // pending). The web UI hides reply links for those, but the
      // REST/MCP surface can be invoked directly.
      if (
        !parent ||
        parent.submissionId !== input.submissionId ||
        parent.deletedAt ||
        parent.state !== "approved"
      ) {
        return { kind: "not_found" };
      }
      parentAuthor = parent.authorId;
    }

    const [row] = await tx
      .insert(comments)
      .values({
        authorId,
        submissionId: input.submissionId,
        parentId: input.parentId ?? null,
        body: input.body,
        state: initialState,
      })
      .returning({ id: comments.id });

    // Only notify when the comment is immediately visible. Pending
    // comments would otherwise produce a "reply" alert linking to a
    // tombstone the recipient cannot read. If/when moderation grows a
    // deferred-notify hook on approval, the gate moves there.
    const notifyTarget = parentAuthor ?? target.authorId;
    if (
      initialState === "approved" &&
      notifyTarget &&
      notifyTarget !== authorId
    ) {
      await tx.insert(notifications).values({
        userId: notifyTarget,
        kind: input.parentId ? "comment_reply" : "submission_reply",
        payload: {
          commentId: row.id,
          submissionId: input.submissionId,
        },
      });
    }

    return { kind: "ok", commentId: row.id };
  });

  if (outcome.kind === "not_found") return { ok: false, reason: "not_found" };
  if (outcome.kind === "locked") return { ok: false, reason: "locked" };

  revalidatePath(`/post/${input.submissionId}`);
  return {
    ok: true,
    commentId: outcome.commentId,
    pending: initialState === "pending",
  };
}

/* ── deleteCommentAsAuthor ──────────────────────────────────────── */

export type DeleteCommentResult =
  | { ok: true; submissionId: string }
  | { ok: false; reason: "not_found" | "forbidden" };

export async function deleteCommentAsAuthor(
  authorId: string,
  commentId: string,
): Promise<DeleteCommentResult> {
  // Atomic: lock the row being deleted FOR UPDATE, then check for
  // children, then act. createComment locks the parent FOR SHARE
  // when inserting a reply, so an in-flight reply blocks until we
  // commit (and vice versa). Without this serialization, a hard
  // delete + concurrent reply could orphan the parent_id — there is
  // no FK on comments.parent_id, so the orphan would not be caught.
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

/* ── updateCommentAsAuthor ──────────────────────────────────────
 *
 * Author-only edit. Same window policy as updateSubmissionAsAuthor:
 * 5-min window for human users, no window for bots / system / staff.
 * Within-window human edits stay silent; out-of-window edits bump
 * updated_at so the UI can render an "edited" badge.
 */

const EDIT_WINDOW_MS = 5 * 60 * 1000;

export const updateCommentInputSchema = z.object({
  body: z.string().trim().min(2).max(40_000),
});

export type UpdateCommentInput = z.infer<typeof updateCommentInputSchema>;

export type UpdateCommentResult =
  | { ok: true; silent: boolean; submissionId: string; updatedAt: Date | null }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "expired" | "noop";
    };

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
    })
    .from(comments)
    .where(eq(comments.id, commentId))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  // Soft-deleted comments stay readable as tombstones; editing one
  // would surface the original body as if undeleted. Treat as gone.
  if (existing.deletedAt) return { ok: false, reason: "not_found" };
  if (existing.authorId !== authorId) return { ok: false, reason: "forbidden" };

  const ageMs = Date.now() - existing.createdAt.getTime();
  const withinWindow = ageMs <= EDIT_WINDOW_MS;
  const bypassesWindow =
    actor.isAgent || actor.role === "system" || actor.role === "staff";
  if (!withinWindow && !bypassesWindow) {
    return { ok: false, reason: "expired" };
  }
  if (input.body === existing.body) return { ok: false, reason: "noop" };

  const silent = withinWindow && !bypassesWindow;
  const bumpedAt = silent ? null : new Date();

  await db
    .update(comments)
    .set({ body: input.body, updatedAt: bumpedAt })
    .where(eq(comments.id, commentId));
  revalidatePath(`/post/${existing.submissionId}`);
  return {
    ok: true,
    silent,
    submissionId: existing.submissionId,
    updatedAt: bumpedAt,
  };
}
