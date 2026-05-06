/**
 * Core submission-creation logic.
 *
 * Lives in lib/ (NOT lib/actions/) because both surfaces need it:
 *
 *   - Web UI server action (lib/actions/submission.ts:submitPost) calls
 *     this with the cookie-authenticated user id.
 *   - REST endpoint (app/api/v1/submissions/route.ts) calls this with
 *     the PAT-authenticated user id and { submitterKind: 'scout',
 *     sourceId: token.displayPrefix } for traceability.
 *   - MCP tool (lib/mcp/tools.ts:submit_link) calls the same.
 *
 * Auth happens at each surface's boundary; this function trusts the
 * authorId it's given.
 */

import { revalidatePath } from "next/cache";
import { and, count, desc, eq, gte, sql } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { submissions, submissionTags, users } from "@/db/schema";
import {
  moderate,
  writeModerationLogForReject,
  writeModerationNotification,
  writePolicyDecision,
  type ModerationAuthor,
} from "@/lib/moderation";

/* ── Schema (shared with lib/actions/submission.ts) ─────────────── */

export const SUBMISSION_TYPES = [
  "news",
  "tip",
  "tutorial",
  "course",
  "article",
  "podcast",
  "interview",
  "tool",
  "discussion",
  // editorial/rubric.yml v0.2.3 types — added in 0008_editorial_runtime
  "release",
  "paper",
  "workflow",
  "case_study",
  "prompt_pattern",
] as const;

export const submissionInputSchema = z
  .object({
    type: z.enum(SUBMISSION_TYPES),
    title: z.string().trim().min(3).max(120),
    url: z.url().or(z.literal("")).optional(),
    text: z.string().trim().max(40_000).optional(),
    tags: z.array(z.string()).max(5).optional(),
  })
  .refine((v) => Boolean(v.url) !== Boolean(v.text), {
    message: "Provide a URL or text body, not both.",
  });

export type SubmissionInput = z.infer<typeof submissionInputSchema>;

export type SubmitResult =
  | { ok: true; submissionId: string; pending: boolean }
  | { ok: false; reason: "validation" | "locked" | "rate"; detail?: string }
  | { ok: false; reason: "duplicate"; existingId: string };

/* ── State determination (auto-approve rules) ──────────────────── */
//
// The audit (slice-1 audit-fix report) flagged that API-submitted
// content from system-role users still auto-approves, which means a
// leaked PAT for an agent could flood the feed. The decision so far
// is to preserve that behavior + add traceability via submitterKind /
// sourceId so any abuse can be retroactively scoped to a token.
// Tightening (e.g., requiring a separate `submission:auto-publish`
// scope) is a follow-up policy call, not a slice-2 blocker.

const APPROVED_PAST_THRESHOLD = 2;
const KARMA_AUTO_APPROVE = 50;

interface AuthorContext {
  role: "user" | "staff" | "locked" | "system";
  karma: number;
  isAgent: boolean;
  botModerationExempt: boolean;
}

async function loadAuthorContext(
  authorId: string,
): Promise<AuthorContext | null> {
  const [row] = await db
    .select({
      role: users.role,
      karma: users.karma,
      isAgent: users.isAgent,
      botModerationExempt: users.botModerationExempt,
    })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  return row ?? null;
}

async function determineInitialState(
  authorId: string,
  ctx: AuthorContext,
): Promise<"pending" | "approved" | "locked"> {
  if (ctx.role === "locked") return "locked";
  if (ctx.role === "staff" || ctx.role === "system") return "approved";
  if (ctx.karma >= KARMA_AUTO_APPROVE) return "approved";

  const [c] = await db
    .select({ n: count() })
    .from(submissions)
    .where(
      and(eq(submissions.authorId, authorId), eq(submissions.state, "approved")),
    );
  return (c?.n ?? 0) >= APPROVED_PAST_THRESHOLD ? "approved" : "pending";
}

async function findRecentDuplicate(url: string): Promise<string | null> {
  const cutoff = new Date(Date.now() - 30 * 86_400_000);
  const [hit] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(
      and(
        eq(submissions.url, url),
        gte(submissions.createdAt, cutoff),
        sql`${submissions.deletedAt} IS NULL`,
      ),
    )
    .orderBy(desc(submissions.createdAt))
    .limit(1);
  return hit?.id ?? null;
}

/* ── createSubmission ───────────────────────────────────────────
 *
 * The single source of truth for "make a new submission". Auth must
 * be performed by the caller; pass the resolved authorId here.
 *
 * `via` describes the entry point. Web traffic passes { surface: 'web' };
 * PAT-auth API/MCP traffic passes { surface: 'api', tokenId, tokenPrefix }.
 * For API submissions we write submitterKind='scout' and store the FULL
 * api_tokens.id UUID in sourceId so a submission can be unambiguously
 * traced back to exactly one token (the 12-char displayPrefix is not
 * unique and would collide as token volume grows). tokenPrefix is kept
 * here only for human-readable log/error messages.
 */

export type SubmissionVia =
  | { surface: "web" }
  | { surface: "api"; tokenId: string; tokenPrefix: string };

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

/* ── deleteSubmissionAsAuthor ────────────────────────────────────
 *
 * Author-only soft delete (sets `deleted_at`). Used by both the web
 * server action and the REST DELETE endpoint. Auth happens at each
 * caller — this function only enforces "the supplied authorId owns
 * the row".
 */

export type DeleteSubmissionResult =
  | { ok: true }
  | { ok: false; reason: "not_found" | "forbidden" };

export async function deleteSubmissionAsAuthor(
  authorId: string,
  submissionId: string,
): Promise<DeleteSubmissionResult> {
  const [existing] = await db
    .select({ authorId: submissions.authorId })
    .from(submissions)
    .where(eq(submissions.id, submissionId))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== authorId) return { ok: false, reason: "forbidden" };

  await db
    .update(submissions)
    .set({ deletedAt: new Date() })
    .where(eq(submissions.id, submissionId));
  revalidatePath(`/post/${submissionId}`);
  revalidatePath("/");
  return { ok: true };
}

/* ── updateSubmissionAsAuthor ────────────────────────────────────
 *
 * Author-only edit. Editable fields mirror the existing web action:
 * `title` and `text`. URL is intentionally NOT editable (it carries
 * dedup identity); type and tags are out of scope here.
 *
 * Two orthogonal axes:
 *
 *   1. Authorization — who is allowed to edit, and when?
 *        Human users (role === "user", is_agent === false): only
 *        within EDIT_WINDOW_MS of post creation.
 *        Bots (is_agent === true) and platform users (role IN
 *        "system" / "staff"): any time.
 *
 *   2. Visibility — does this edit surface as "edited" to readers?
 *        Within-window edits stay SILENT (no updated_at bump, no
 *        badge) regardless of role. The window is the period in
 *        which a reader could not yet have read the original; a
 *        correction inside it is functionally a typo fix.
 *        Out-of-window edits bump updated_at and render the badge.
 *
 * Soft-deleted rows are not editable — the row is gone from the
 * public surface and cannot be resurrected via update.
 *
 * URL/text invariant: the row stays a link post (url set, text null)
 * or a self-post (url null, text set). An update that would break the
 * invariant — adding text to a link post, or clearing a self-post's
 * text — is rejected. Author should delete + re-create instead.
 */

const EDIT_WINDOW_MS = 5 * 60 * 1000;

export const updateSubmissionInputSchema = z
  .object({
    title: z.string().trim().min(3).max(120).optional(),
    text: z.string().trim().max(40_000).optional(),
  })
  .refine((v) => v.title !== undefined || v.text !== undefined, {
    message: "Provide at least one of: title, text.",
  });

export type UpdateSubmissionInput = z.infer<typeof updateSubmissionInputSchema>;

export type UpdateSubmissionResult =
  | { ok: true; silent: boolean; updatedAt: Date | null }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "expired" | "noop" | "invalid";
      detail?: string;
    };

export async function updateSubmissionAsAuthor(
  authorId: string,
  submissionId: string,
  input: UpdateSubmissionInput,
): Promise<UpdateSubmissionResult> {
  const [actor] = await db
    .select({ role: users.role, isAgent: users.isAgent })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  // Missing actor → unauth equivalent. The REST surface authenticated
  // already; this only fires if the user was deleted between auth and
  // here, which we treat as not_found (don't disclose existence).
  if (!actor) return { ok: false, reason: "not_found" };

  const [existing] = await db
    .select({
      authorId: submissions.authorId,
      createdAt: submissions.createdAt,
      title: submissions.title,
      text: submissions.text,
      url: submissions.url,
      deletedAt: submissions.deletedAt,
    })
    .from(submissions)
    .where(eq(submissions.id, submissionId))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  // Soft-deleted rows are gone from the public surface. Editing one
  // would resurrect content under deletedAt-IS-NOT-NULL — confusing
  // for the author and never visible to readers anyway.
  if (existing.deletedAt) return { ok: false, reason: "not_found" };
  if (existing.authorId !== authorId) return { ok: false, reason: "forbidden" };

  const ageMs = Date.now() - existing.createdAt.getTime();
  const withinWindow = ageMs <= EDIT_WINDOW_MS;
  const bypassesWindow =
    actor.isAgent || actor.role === "system" || actor.role === "staff";
  if (!withinWindow && !bypassesWindow) {
    return { ok: false, reason: "expired" };
  }

  // Compute the new title / text against `existing`. Detecting a
  // genuine change (rather than "field provided but identical")
  // turns redundant edits into a noop result and keeps RETURNING
  // honest about whether anything actually mutated.
  const newTitle =
    input.title !== undefined && input.title !== existing.title
      ? input.title
      : null;
  const newText: { value: string | null; changed: boolean } =
    input.text !== undefined && input.text !== (existing.text ?? "")
      ? { value: input.text === "" ? null : input.text, changed: true }
      : { value: existing.text, changed: false };

  if (newTitle === null && !newText.changed) {
    return { ok: false, reason: "noop" };
  }

  // URL XOR text invariant — existing.url is immutable here.
  const hasUrl = Boolean(existing.url);
  const hasText = Boolean(newText.value);
  if (hasUrl === hasText) {
    return {
      ok: false,
      reason: "invalid",
      detail: hasUrl
        ? "Cannot add text to a link post. Delete and repost as a discussion."
        : "Cannot clear text on a self-post. Delete it instead.",
    };
  }

  // Visibility is a function of time, not role. The 5-min window is
  // when the post hasn't been read yet; a correction inside it is a
  // typo fix and shouldn't render an "edited" badge — even for bots.
  // Role only determines whether the EDIT is allowed at all (above).
  const silent = withinWindow;
  const bumpedAt = silent ? null : new Date();

  const updates: Partial<{
    title: string;
    text: string | null;
    updatedAt: Date;
  }> = {};
  if (newTitle !== null) updates.title = newTitle;
  if (newText.changed) updates.text = newText.value;
  // Omit updatedAt from .set() when silent so a prior post-window
  // bump on the same row is preserved (cannot happen today — every
  // post-window edit creates a fresh bump — but defensive against
  // future code paths that might silently re-edit).
  if (!silent) updates.updatedAt = bumpedAt as Date;

  // Atomic guard: re-check authorship and not-deleted in the WHERE
  // clause so a concurrent delete or role-flip can't slip a write
  // through. RETURNING tells us whether the row actually moved.
  const result = await db
    .update(submissions)
    .set(updates)
    .where(
      and(
        eq(submissions.id, submissionId),
        eq(submissions.authorId, authorId),
        sql`${submissions.deletedAt} IS NULL`,
      ),
    )
    .returning({ id: submissions.id });
  if (result.length === 0) return { ok: false, reason: "not_found" };

  revalidatePath(`/post/${submissionId}`);
  // Title / text changes can affect the home feed and search snippets.
  revalidatePath("/");
  // /search reads fresh from the DB per-request today (dynamic route),
  // so this is a no-op now. Kept explicit so adding any caching layer
  // in /search later doesn't silently leak stale post titles.
  revalidatePath("/search");
  return { ok: true, silent, updatedAt: bumpedAt };
}
