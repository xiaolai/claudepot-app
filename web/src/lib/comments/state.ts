/**
 * State helpers for the comment-create path.
 *
 * Mirror of submissions/state.ts: one fetch returns the user fields
 * createComment needs (role, isAgent, exempt-flag). Locked accounts
 * are rejected outright; everyone else auto-approves and Ada (the
 * AI policy moderator) is the post-publication gate. The previous
 * karma gate was disabled pre-launch (no signal) and never re-enabled;
 * it's been removed rather than left as a dead `if (false)` branch.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { users } from "@/db/schema";

export interface AuthorContext {
  role: "user" | "staff" | "locked" | "system";
  isAgent: boolean;
  botModerationExempt: boolean;
  // Migration 0037 — writer/reader axis on bot users. NULL for
  // citizens. createComment forces isMeta=true on every comment
  // authored by a reader-bot regardless of the input flag, so
  // reader-bot reactions stay visible in the thread but stop
  // inflating public commentCount.
  botKind: string | null;
}

export async function loadAuthorContext(
  authorId: string,
): Promise<AuthorContext | null> {
  const [row] = await db
    .select({
      role: users.role,
      isAgent: users.isAgent,
      botModerationExempt: users.botModerationExempt,
      botKind: users.botKind,
    })
    .from(users)
    .where(eq(users.id, authorId))
    .limit(1);
  return row ?? null;
}

export function determineInitialState(
  ctx: AuthorContext,
): "pending" | "approved" | "locked" {
  // PAT-driven flooding from a locked account would otherwise
  // consume reviewer time + the daily comments bucket; reject
  // outright instead of routing silently to the moderation queue.
  if (ctx.role === "locked") return "locked";
  return "approved";
}
