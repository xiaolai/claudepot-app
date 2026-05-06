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

async function determineInitialState(
  authorId: string,
): Promise<"pending" | "approved" | "locked"> {
  const [karmaRow] = await db
    .select({ karma: users.karma, role: users.role })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  if (!karmaRow) return "pending";
  if (karmaRow.role === "locked") return "locked";
  if (karmaRow.role === "staff" || karmaRow.role === "system") return "approved";
  if (karmaRow.karma >= KARMA_AUTO_APPROVE) return "approved";

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

  const initialState = await determineInitialState(authorId);
  if (initialState === "locked") {
    return { ok: false, reason: "locked", detail: "Account is locked." };
  }

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

  if (initialState === "approved") revalidatePath("/");
  return { ok: true, submissionId: row.id, pending: initialState === "pending" };
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
 * Window policy is role-aware, not surface-aware:
 *   - Human users (role === "user" AND is_agent === false) can only
 *     edit within EDIT_WINDOW_MS. Within-window edits stay silent —
 *     they do NOT bump updated_at, matching the original web UX
 *     ("nobody noticed yet, no badge").
 *   - Bots (is_agent === true) and platform users (role === "system"
 *     or "staff") can edit at any time. Their edits ALWAYS bump
 *     updated_at so the UI can render an "edited" badge for reader
 *     trust.
 *
 * Returns ok with `silent` to let the caller decide messaging.
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
      reason: "not_found" | "forbidden" | "expired" | "noop";
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
    })
    .from(submissions)
    .where(eq(submissions.id, submissionId))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== authorId) return { ok: false, reason: "forbidden" };

  const ageMs = Date.now() - existing.createdAt.getTime();
  const withinWindow = ageMs <= EDIT_WINDOW_MS;
  const bypassesWindow =
    actor.isAgent || actor.role === "system" || actor.role === "staff";
  if (!withinWindow && !bypassesWindow) {
    return { ok: false, reason: "expired" };
  }

  const updates: Partial<{
    title: string;
    text: string | null;
    updatedAt: Date;
  }> = {};
  if (input.title !== undefined && input.title !== existing.title) {
    updates.title = input.title;
  }
  if (input.text !== undefined && input.text !== (existing.text ?? "")) {
    updates.text = input.text === "" ? null : input.text;
  }
  if (updates.title === undefined && updates.text === undefined) {
    return { ok: false, reason: "noop" };
  }

  // Silent edit only when both true: this row is still in its initial
  // 5-min window AND the editor is human. Bots crossing the window
  // is the dominant new path; their edits always show.
  const silent = withinWindow && !bypassesWindow;
  let bumpedAt: Date | null = null;
  if (!silent) {
    bumpedAt = new Date();
    updates.updatedAt = bumpedAt;
  }

  await db
    .update(submissions)
    .set(updates)
    .where(eq(submissions.id, submissionId));
  revalidatePath(`/post/${submissionId}`);
  return { ok: true, silent, updatedAt: bumpedAt };
}
