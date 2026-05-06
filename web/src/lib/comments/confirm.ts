/**
 * Comment confirmation pass — second moderate() call after an
 * optimistic publish.
 *
 * Per dev-docs/policy-moderator-plan.md §3.2 and §7.2: when the
 * first-pass moderate() returns a non-illegal reject on a comment,
 * createComment publishes the row anyway (state='approved') and
 * schedules this confirmation pass via Vercel `after()`. The
 * confirmation re-runs moderate() against the freshly-inserted
 * row; if it still rejects, we flip state='rejected', write a
 * pass=2 policy_decisions row, the moderation_log row, and the
 * notification.
 *
 * Why a second pass at all: gpt-4o-mini class models false-positive
 * on conversation-style text at a meaningful rate. A single-pass
 * decision retracts comments that read fine on a second look.
 * Two passes absorb that variance — at the cost of a brief window
 * where the comment is publicly visible, which is exactly the
 * tradeoff §3.1 names.
 *
 * If the comment was deleted by the author or another path before
 * the confirmation lands, we skip the retract gracefully.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { comments } from "@/db/schema";
import {
  checkBanCandidate,
  moderate,
  writeModerationLogForReject,
  writeModerationNotification,
  writePolicyDecision,
  type ModerationAuthor,
} from "@/lib/moderation";

export interface ConfirmCommentParams {
  commentId: string;
  body: string;
  author: ModerationAuthor;
  /** The body of the parent submission, used for context only — not
   *  passed to the moderator (which scores the comment text alone). */
  submissionId: string;
}

/**
 * Run the second-pass moderation on a comment that was published
 * optimistically. Idempotent in spirit — safe to invoke twice; the
 * worst case is a duplicate policy_decisions row with pass=2.
 *
 * Designed to be invoked from `after()` so the original POST returns
 * fast. Errors are logged, never thrown — this runs after the
 * response is on the wire.
 */
export async function runCommentConfirmation(
  params: ConfirmCommentParams,
): Promise<void> {
  try {
    // Re-fetch the comment to confirm it still exists and isn't
    // already rejected/deleted. If the author deleted it or staff
    // rejected it manually, skip — confirming a moot row would
    // double-write log + notification.
    const [row] = await db
      .select({
        id: comments.id,
        body: comments.body,
        state: comments.state,
        deletedAt: comments.deletedAt,
      })
      .from(comments)
      .where(eq(comments.id, params.commentId))
      .limit(1);
    if (!row || row.deletedAt || row.state !== "approved") return;

    const verdict = await moderate(
      { kind: "comment", title: "", body: row.body },
      params.author,
    );

    // Synthetic verdicts (MODERATION_ENABLED=0 or model error) leave
    // the optimistic publish standing. The first pass's decision row
    // is the authoritative audit record; a synthetic second pass
    // doesn't add information.
    if (verdict.synthetic) return;

    // Pass-2 row regardless of verdict — the audit trail wants both
    // passes recorded so calibration can compute the FP-confirmation
    // rate.
    const decisionId = await writePolicyDecision({
      authorId: params.author.id,
      targetType: "comment",
      targetId: row.id,
      verdict,
      passNumber: 2,
    });

    if (verdict.verdict !== "reject" || !verdict.category) {
      // Pass-2 cleared it — leave the comment up. The discrepancy
      // between pass-1 and pass-2 is a calibration signal; the audit
      // trail captures it via the two policy_decisions rows.
      return;
    }

    // Pass-2 still rejects — retract.
    await db
      .update(comments)
      .set({ state: "rejected" })
      .where(eq(comments.id, row.id));

    await writeModerationLogForReject({
      targetType: "comment",
      targetId: row.id,
      category: verdict.category,
      oneLineWhy: verdict.oneLineWhy,
      passNumber: 2,
    });
    await writeModerationNotification({
      recipientId: params.author.id,
      targetType: "comment",
      targetId: row.id,
      targetTitle: null,
      category: verdict.category,
      oneLineWhy: verdict.oneLineWhy,
      decisionId,
    });
    await checkBanCandidate(params.author.id, verdict, "comment", row.id);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.warn(
      `[moderation] comment confirmation failed for ${params.commentId}: ${msg}`,
    );
  }
}
