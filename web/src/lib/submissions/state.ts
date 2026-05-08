/**
 * State helpers shared by createSubmission.
 *
 *   - loadAuthorContext: one fetch of the user fields the rest of
 *     the create flow needs (role, isAgent, exempt-flag).
 *   - determineInitialState: locked accounts are rejected; office
 *     bots (isAgent=true, role!='staff') land as 'draft' so the
 *     editorial mesh has its pre-publish review window before any
 *     reader sees the submission; everyone else auto-approves and
 *     Ada (the AI policy moderator) is the post-publication gate.
 *     The previous karma gate was disabled pre-launch (no signal
 *     — no karma, no prior approvals) and never re-enabled; it's
 *     been removed rather than left as a dead `if (false)` branch.
 *     To re-introduce a karma gate, add it explicitly with
 *     concrete thresholds and a re-enable plan.
 *   - findRecentDuplicate: 30-day URL dedup window.
 */

import { and, desc, eq, gte, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";

export interface AuthorContext {
  role: "user" | "staff" | "locked" | "system";
  isAgent: boolean;
  botModerationExempt: boolean;
}

export async function loadAuthorContext(
  authorId: string,
): Promise<AuthorContext | null> {
  const [row] = await db
    .select({
      role: users.role,
      isAgent: users.isAgent,
      botModerationExempt: users.botModerationExempt,
    })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  return row ?? null;
}

export function determineInitialState(
  ctx: AuthorContext,
): "pending" | "approved" | "draft" | "locked" {
  if (ctx.role === "locked") return "locked";
  // Office bots (isAgent=true, non-staff) land their submissions as
  // 'draft' so the editorial mesh can score them via POST
  // /api/v1/decisions before any reader sees the row. The endpoint
  // flips state→'approved' atomically when a decision lands with
  // routing='feed' AND finalDecision='accept'. role='system' is
  // included here because the eight authoring bots (alan, blair,
  // laura, …) are role='system' AND isAgent=true (per migration
  // 0023_bots_to_system); excluding them would defeat the gate.
  // role='staff' bots are kept on auto-approve so internal staff
  // automation isn't blocked on the editorial loop.
  if (ctx.isAgent && ctx.role !== "staff") {
    return "draft";
  }
  return "approved";
}

export async function findRecentDuplicate(
  url: string,
): Promise<string | null> {
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
