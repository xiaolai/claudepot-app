/**
 * createComment — single source of truth for "post a new comment".
 *
 * Three surfaces call it:
 *   - Web UI server actions (lib/actions/comment.ts)
 *   - REST endpoint (app/api/v1/comments/*)
 *   - MCP tool (lib/mcp/tools.ts)
 *
 * Auth happens at the boundary; this function trusts the authorId.
 *
 * Pipeline:
 *   1. Karma gate (locked → reject; staff/system/karma → approved).
 *   2. Ladder rate-limit check (rung 3 — drops daily cap after recent rejects).
 *   3. Moderate() — runs OUTSIDE the FOR-SHARE transaction so a
 *      1500ms model call doesn't hold row locks.
 *   4. Branch on verdict:
 *        - illegal → hard block, no insert. Audit row written with
 *          target_id=NULL, staff flag inserted, return error.
 *        - non-illegal reject → optimistic publish, schedule
 *          confirmation pass via after(). State stays approved
 *          unless the confirmation pass also rejects.
 *        - pass → existing flow.
 *   5. Insert + notify inside a single transaction with FOR SHARE
 *      locks on the target submission and (if a reply) the parent
 *      comment, matching the original locking discipline.
 *   6. Audit-row writes (policy_decisions, ban-candidate flag for
 *      illegal) happen sequentially after the transaction commits.
 *
 * See dev-docs/policy-moderator-plan.md §7.2 and §3.
 */

import { revalidatePath } from "next/cache";
import { after } from "next/server";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { comments, notifications, submissions } from "@/db/schema";
import {
  checkBanCandidate,
  checkLadderRateLimit,
  moderate,
  writeModerationNotification,
  writePolicyDecision,
  type ModerationAuthor,
} from "@/lib/moderation";

import { runCommentConfirmation } from "./confirm";
import type { CommentInput, CommentResult } from "./schema";
import {
  determineInitialState,
  loadAuthorContext,
  type AuthorContext,
} from "./state";

export async function createComment(
  authorId: string,
  input: CommentInput,
): Promise<CommentResult> {
  const ctx = await loadAuthorContext(authorId);
  if (!ctx) return { ok: false, reason: "not_found" };

  const initialState = await determineInitialState(authorId, ctx);
  if (initialState === "locked") return { ok: false, reason: "locked" };

  if (!isExempt(ctx)) {
    const rate = await checkLadderRateLimit(authorId);
    if (rate.rateLimited) {
      return {
        ok: false,
        reason: "rate",
        detail: rate.reason ?? "Rate limit reached.",
      };
    }
  }

  // Run the moderator BEFORE the FOR-SHARE transaction. A 1500ms
  // model call inside the transaction would hold parent locks and
  // serialize concurrent comment writes on hot threads.
  const author: ModerationAuthor = {
    id: authorId,
    role: ctx.role,
    isAgent: ctx.isAgent,
    botModerationExempt: ctx.botModerationExempt,
  };
  const verdict = await moderate(
    { kind: "comment", title: "", body: input.body },
    author,
  );

  // Hard block on 'illegal'. Do not insert. Write the policy_decisions
  // row with target_id=NULL — the comment never existed — and let
  // checkBanCandidate file the flag (it inserts a ban_candidate flag
  // on any 'illegal' verdict regardless of count, dedup'd by user).
  // The flag's targetType points at the parent submission since we
  // have no comment row to target.
  if (verdict.verdict === "reject" && verdict.category === "illegal") {
    if (!verdict.synthetic) {
      try {
        const decisionId = await writePolicyDecision({
          authorId,
          targetType: "comment",
          targetId: null,
          verdict,
        });
        await writeModerationNotification({
          recipientId: authorId,
          targetType: "comment",
          targetId: null,
          targetTitle: null,
          category: "illegal",
          oneLineWhy: verdict.oneLineWhy,
          decisionId,
        });
        await checkBanCandidate(
          authorId,
          verdict,
          "submission",
          input.submissionId,
        );
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        console.warn(
          `[moderation] illegal-block persist failed for ${authorId}: ${msg}`,
        );
      }
    }
    return {
      ok: false,
      reason: "illegal",
      detail: verdict.oneLineWhy,
    };
  }

  // Non-illegal rejects fall through to optimistic publish. Pass
  // verdicts also fall through. The transaction handles insert +
  // notification atomically.
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

  // Persist the first-pass verdict outside the transaction. A failure
  // here doesn't roll back the comment insert — the row is already
  // user-visible and recoverable via the confirmation pass below or
  // staff intervention.
  if (!verdict.synthetic) {
    try {
      await writePolicyDecision({
        authorId,
        targetType: "comment",
        targetId: outcome.commentId,
        verdict,
        passNumber: 1,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.warn(
        `[moderation] pass-1 persist failed for comment ${outcome.commentId}: ${msg}`,
      );
    }
  }

  // Schedule the confirmation pass for non-illegal rejects. after()
  // runs after the response is sent to the client, so the original
  // POST is not blocked by a second model call.
  if (
    verdict.verdict === "reject" &&
    verdict.category !== "illegal" &&
    !verdict.synthetic
  ) {
    after(() =>
      runCommentConfirmation({
        commentId: outcome.commentId,
        body: input.body,
        author,
        submissionId: input.submissionId,
      }),
    );
  }

  revalidatePath(`/post/${input.submissionId}`);
  return {
    ok: true,
    commentId: outcome.commentId,
    pending: initialState === "pending",
  };
}

function isExempt(ctx: AuthorContext): boolean {
  if (ctx.role === "staff" || ctx.role === "system") return true;
  if (ctx.isAgent && ctx.botModerationExempt) return true;
  return false;
}
