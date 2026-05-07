/**
 * State helpers shared by createSubmission.
 *
 *   - loadAuthorContext: one fetch of the user fields the rest of
 *     the create flow needs (role, karma, isAgent, exempt-flag).
 *   - determineInitialState: the existing karma gate, kept untouched.
 *     The audit (slice-1 audit-fix report) flagged that API-submitted
 *     content from system-role users still auto-approves, which means
 *     a leaked PAT for an agent could flood the feed. The decision
 *     so far is to preserve that behavior + add traceability via
 *     submitterKind / sourceId so any abuse can be retroactively
 *     scoped to a token. Tightening (e.g., a separate
 *     'submission:auto-publish' scope) is a follow-up policy call.
 *   - findRecentDuplicate: 30-day URL dedup window.
 */

import { and, count, desc, eq, gte, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";

// Launch-mode flag. Pre-launch the karma gate provides no signal —
// no user has karma, no user has past approvals, so every real
// submission lands in 'pending' and the user sees a 404 on their
// own permalink. Ada is the sole gate while this is false. Re-enable
// once karma signal accumulates (vote volume, repeat contributors)
// or when spam volume justifies the friction.
const KARMA_GATE_ENABLED = false;
const APPROVED_PAST_THRESHOLD = 2;
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
  if (ctx.role === "locked") return "locked";
  if (ctx.role === "staff" || ctx.role === "system") return "approved";
  if (!KARMA_GATE_ENABLED) return "approved";
  if (ctx.karma >= KARMA_AUTO_APPROVE) return "approved";

  const [c] = await db
    .select({ n: count() })
    .from(submissions)
    .where(
      and(eq(submissions.authorId, authorId), eq(submissions.state, "approved")),
    );
  return (c?.n ?? 0) >= APPROVED_PAST_THRESHOLD ? "approved" : "pending";
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
