/**
 * createSubmission — single source of truth for "make a new submission".
 *
 * Auth happens at each surface's boundary; this function trusts the
 * authorId it's given. Three surfaces call it:
 *
 *   - Web UI server action (lib/actions/submission.ts:submitPost)
 *   - REST endpoint (app/api/v1/submissions/route.ts)
 *   - MCP tool (lib/mcp/tools.ts:submit_link)
 *
 * Pipeline: dup check → karma gate → AI policy moderator → insert.
 * Policy reject overrides the karma-gate state with state='rejected'.
 * The row is inserted either way so an appeal has a target.
 */

import { revalidatePath } from "next/cache";

import { db } from "@/db/client";
import { submissions, submissionTags } from "@/db/schema";
import {
  checkBanCandidate,
  checkLadderRateLimit,
  moderate,
  writeModerationLogForReject,
  writeModerationNotification,
  writePolicyDecision,
  type ModerationAuthor,
} from "@/lib/moderation";

import {
  determineInitialState,
  findRecentDuplicate,
  loadAuthorContext,
} from "./state";
import type { SubmissionInput, SubmissionVia, SubmitResult } from "./schema";

export async function createSubmission(
  authorId: string,
  input: SubmissionInput,
  via: SubmissionVia = { surface: "web" },
): Promise<SubmitResult> {
  const { type, title, url, text, tags = [] } = input;
  const normalizedUrl = url ? url.trim() : null;
  const normalizedText = text ? text.trim() : null;

  if (normalizedUrl) {
    const dup = await findRecentDuplicate(normalizedUrl);
    if (dup) return { ok: false, reason: "duplicate", existingId: dup };
  }

  const ctx = await loadAuthorContext(authorId);
  if (!ctx) return { ok: false, reason: "validation", detail: "Author not found." };

  const karmaState = await determineInitialState(authorId, ctx);
  if (karmaState === "locked") {
    return { ok: false, reason: "locked", detail: "Account is locked." };
  }

  // Ban-ladder rung 3: if the author has accumulated rejects
  // recently, their daily content cap drops. Skip the cap for
  // exempt users (staff / system / allowlisted bots) — the
  // moderator is the source of truth on "should this rung apply"
  // and exempt users don't generate the underlying rejects anyway.
  if (
    ctx.role !== "staff" &&
    ctx.role !== "system" &&
    !(ctx.isAgent && ctx.botModerationExempt)
  ) {
    const rate = await checkLadderRateLimit(authorId);
    if (rate.rateLimited) {
      return {
        ok: false,
        reason: "rate",
        detail: rate.reason ?? "Rate limit reached.",
      };
    }
  }

  // Run the policy moderator BEFORE inserting. Synchronous by
  // design: the row's initial state must reflect the verdict so
  // a rejected submission never enters the public feed even
  // briefly. See dev-docs/policy-moderator-plan.md §7.1.
  const author: ModerationAuthor = {
    id: authorId,
    role: ctx.role,
    isAgent: ctx.isAgent,
    botModerationExempt: ctx.botModerationExempt,
  };
  const verdict = await moderate(
    {
      kind: "submission",
      title,
      // Body for the moderator is whatever the user actually wrote —
      // text body or the URL itself if URL-only. The model needs at
      // least one of them; the input schema already enforces XOR.
      body: normalizedText ?? normalizedUrl ?? "",
    },
    author,
  );

  const moderatorRejected =
    verdict.verdict === "reject" && verdict.category !== null;
  const initialState = moderatorRejected ? "rejected" : karmaState;

  const now = new Date();
  const [row] = await db
    .insert(submissions)
    .values({
      authorId,
      type,
      title,
      url: normalizedUrl,
      text: normalizedText,
      state: initialState,
      publishedAt: initialState === "approved" ? now : null,
      submitterKind: via.surface === "api" ? "scout" : "user",
      sourceId: via.surface === "api" ? via.tokenId : null,
    })
    .returning({ id: submissions.id });

  if (tags.length > 0) {
    await db.insert(submissionTags).values(
      tags.map((tagSlug) => ({ submissionId: row.id, tagSlug })),
    );
  }

  // Record the verdict + (on reject) the audit log + the user
  // notification. Sequential not transactional — createSubmission
  // is non-transactional today; an audit-row failure here logs but
  // does not roll back the submission insert. Acceptable because
  // (a) the row is already user-visible, (b) the user will retry
  // on appeal, (c) Slice 1b will reconcile if needed.
  if (!verdict.synthetic) {
    try {
      const decisionId = await writePolicyDecision({
        authorId,
        targetType: "submission",
        targetId: row.id,
        verdict,
      });
      if (moderatorRejected && verdict.category) {
        await writeModerationLogForReject({
          targetType: "submission",
          targetId: row.id,
          category: verdict.category,
          oneLineWhy: verdict.oneLineWhy,
        });
        await writeModerationNotification({
          recipientId: authorId,
          targetType: "submission",
          targetId: row.id,
          targetTitle: title,
          category: verdict.category,
          oneLineWhy: verdict.oneLineWhy,
          decisionId,
        });
        // Rung 4: file a ban-candidate flag if thresholds are
        // tripped. Runs AFTER policy_decisions has been written so
        // recentRejects includes the just-now reject. Idempotent —
        // returns early if an open ban-candidate flag already
        // exists for this user.
        await checkBanCandidate(authorId, verdict, "submission", row.id);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.warn(`[moderation] persist failed for submission ${row.id}: ${msg}`);
    }
  }

  if (initialState === "approved") revalidatePath("/");
  return {
    ok: true,
    submissionId: row.id,
    pending: initialState === "pending",
  };
}
