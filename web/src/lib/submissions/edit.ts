/**
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

import { revalidatePath } from "next/cache";
import { and, eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";
import type { UpdateSubmissionInput, UpdateSubmissionResult } from "./schema";

const EDIT_WINDOW_MS = 5 * 60 * 1000;

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
