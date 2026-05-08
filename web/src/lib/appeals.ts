/**
 * Core appeal-submission logic.
 *
 * Lives in lib/ (not lib/actions/) so both surfaces share it:
 *
 *   - Web UI server action (lib/actions/appeals.ts:submitAppeal) calls
 *     this with the cookie-authenticated user id.
 *   - REST endpoint (app/api/v1/appeals/route.ts) calls it with the
 *     PAT-authenticated user id.
 *
 * Auth happens at each surface's boundary; this function trusts the
 * authorId it's given.
 *
 * Reuses the `flags` table with a tagged reason ("appeal: …") so
 * staff sees appeals in /admin/queue alongside community flags. The
 * email-verified gate that the public flag action enforces is
 * skipped here — a freshly-rejected user may not have verified yet
 * and denying them an appeal path defeats the moderator's
 * accountability.
 *
 * One-appeal-per-decision is enforced application-side: if an open
 * flag tagged "appeal:" already references the same target, the new
 * attempt is rejected.
 */

import { revalidatePath } from "next/cache";
import { and, eq, sql } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { comments, flags, policyDecisions, submissions } from "@/db/schema";

// flags.reason has a practical 500-char ceiling (set by callers via
// .slice(500) — see lib/moderation/persist.ts). The "appeal: "
// prefix takes 8 chars, leaving ~490 for user text. Cap at 480 to
// keep a margin for emoji byte expansion and any future prefix
// changes. Going higher would silently truncate user-submitted
// appeals, which is the bug Codex flagged.
export const appealInputSchema = z.object({
  decisionId: z.uuid(),
  text: z.string().trim().min(10).max(480),
});

export type AppealInput = z.infer<typeof appealInputSchema>;

export type AppealCoreResult =
  | { ok: true; flagId: string }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "duplicate" | "stale";
    };

export async function submitAppealAsAuthor(
  authorId: string,
  input: AppealInput,
): Promise<AppealCoreResult> {
  const { decisionId, text } = input;

  const [decision] = await db
    .select({
      id: policyDecisions.id,
      authorId: policyDecisions.authorId,
      targetType: policyDecisions.targetType,
      targetId: policyDecisions.targetId,
      verdict: policyDecisions.verdict,
    })
    .from(policyDecisions)
    .where(eq(policyDecisions.id, decisionId))
    .limit(1);

  if (!decision) return { ok: false, reason: "not_found" };
  if (decision.authorId !== authorId) {
    return { ok: false, reason: "forbidden" };
  }
  if (decision.verdict !== "reject") {
    // Appeals only make sense on rejects — a `pass` decision has
    // nothing to appeal. Surface as `stale` so callers can show a
    // useful message.
    return { ok: false, reason: "stale" };
  }
  if (!decision.targetId) {
    // illegal-comment block path: the comment was never inserted,
    // so there's no row for staff to act on. These appeals require
    // a different (manual / email) path; surface as stale.
    return { ok: false, reason: "stale" };
  }

  // Confirm the targeted content still exists and is in a
  // rejected/non-deleted state — if staff already approved or the
  // row was deleted, an appeal is moot.
  if (decision.targetType === "submission") {
    const [sub] = await db
      .select({ state: submissions.state, deletedAt: submissions.deletedAt })
      .from(submissions)
      .where(eq(submissions.id, decision.targetId))
      .limit(1);
    if (!sub || sub.deletedAt) return { ok: false, reason: "stale" };
    if (sub.state !== "rejected") return { ok: false, reason: "stale" };
  } else {
    // Comment targets: appeals make sense only when the comment is
    // currently 'rejected' (typically by the confirmation pass) and
    // not deleted. A pass-2-cleared or staff-approved comment has
    // nothing to appeal.
    const [c] = await db
      .select({ state: comments.state, deletedAt: comments.deletedAt })
      .from(comments)
      .where(eq(comments.id, decision.targetId))
      .limit(1);
    if (!c || c.deletedAt) return { ok: false, reason: "stale" };
    if (c.state !== "rejected") return { ok: false, reason: "stale" };
  }

  // Block duplicates: at most one open appeal flag per target. The
  // SQL filters specifically for appeal-tagged rows so an unrelated
  // open flag (community report) doesn't cause us to silently miss
  // an existing appeal. Race against concurrent inserts is bounded
  // and acceptable for v0 — staff dismisses dupes from /admin/queue
  // — but the prefix filter eliminates the more common false
  // negative path.
  const [existingAppeal] = await db
    .select({ id: flags.id })
    .from(flags)
    .where(
      and(
        eq(flags.targetType, decision.targetType),
        eq(flags.targetId, decision.targetId),
        eq(flags.status, "open"),
        sql`${flags.reason} LIKE 'appeal:%'`,
      ),
    )
    .limit(1);
  if (existingAppeal) {
    return { ok: false, reason: "duplicate" };
  }

  let row: { id: string } | undefined;
  try {
    [row] = await db
      .insert(flags)
      .values({
        reporterId: authorId,
        targetType: decision.targetType,
        targetId: decision.targetId,
        reason: `appeal: ${text}`.slice(0, 500),
      })
      .returning({ id: flags.id });
  } catch (err) {
    // Migration 0019 lays a partial unique index on (target_type,
    // target_id) WHERE status='open' AND reason LIKE 'appeal:%'. A
    // concurrent insert that lost the race lands here with a
    // unique-violation; translate to reason='duplicate' so the
    // caller maps to 409 / "already in queue".
    if (isUniqueViolation(err)) {
      return { ok: false, reason: "duplicate" };
    }
    throw err;
  }
  if (!row) {
    // Defensive: insert succeeded but RETURNING produced no row. Treat
    // as not_found so the caller surfaces a retryable error.
    return { ok: false, reason: "not_found" };
  }

  revalidatePath("/admin");
  revalidatePath("/admin/console/appeals");
  revalidatePath(`/appeal/${decisionId}`);

  return { ok: true, flagId: row.id };
}

// Postgres unique-violation is SQLSTATE 23505. Under
// `@neondatabase/serverless`'s `Pool`, errors surface as pg
// `DatabaseError` with `.code === "23505"`. Catch loosely (typed code
// + message fallback) so a future driver swap doesn't silently mask
// the constraint.
function isUniqueViolation(err: unknown): boolean {
  if (typeof err !== "object" || err === null) return false;
  const e = err as { code?: unknown; message?: unknown };
  if (e.code === "23505") return true;
  if (
    typeof e.message === "string" &&
    e.message.includes("idx_flags_open_appeal_per_target")
  ) {
    return true;
  }
  return false;
}
