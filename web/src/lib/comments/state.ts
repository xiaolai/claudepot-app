/**
 * State helpers for the comment-create path.
 *
 * Mirror of submissions/state.ts: one fetch returns the user fields
 * the rest of createComment needs (role, karma, isAgent, exempt-flag),
 * and the karma gate is computed against that context. Comments use
 * a softer karma gate than submissions — any user with at least one
 * approved submission is past first-comment review.
 */

import { and, eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";

const KARMA_AUTO_APPROVE = 50;

export interface AuthorContext {
  role: "user" | "staff" | "locked" | "system";
  karma: number;
  isAgent: boolean;
  botModerationExempt: boolean;
}

export async function loadAuthorContext(
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

export async function determineInitialState(
  authorId: string,
  ctx: AuthorContext,
): Promise<"pending" | "approved" | "locked"> {
  // Mirror lib/submissions: locked accounts are rejected outright,
  // not silently routed to the moderation queue. PAT-driven flooding
  // would otherwise consume reviewer time + the daily comments bucket.
  if (ctx.role === "locked") return "locked";
  if (ctx.role === "staff" || ctx.role === "system") return "approved";
  if (ctx.karma >= KARMA_AUTO_APPROVE) return "approved";

  // Softer than submissions: any prior approved submission lifts the
  // first-comment-pending gate.
  const [hasApproved] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(
      and(eq(submissions.authorId, authorId), eq(submissions.state, "approved")),
    )
    .limit(1);
  return hasApproved ? "approved" : "pending";
}
