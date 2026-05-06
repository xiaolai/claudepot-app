"use server";

import { auth } from "@/lib/auth";
import {
  castVote,
  saveInputSchema,
  setSave,
  voteInputSchema,
  type SaveResult as CoreSaveResult,
  type VoteResult as CoreVoteResult,
} from "@/lib/votes";

export type VoteResult =
  | CoreVoteResult
  | { ok: false; reason: "unauth" | "validation" };

export async function vote(input: unknown): Promise<VoteResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = voteInputSchema.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const result = await castVote(session.user.id, parsed.data);
  // The web action's legacy contract treats "missing user" as
  // "unauth" — the cookie session is stale. Translate at the boundary.
  if (!result.ok && result.reason === "missing_user") {
    return { ok: false, reason: "unauth" };
  }
  return result;
}

/* ── save ──────────────────────────────────────────────────────── */

export type SaveResult =
  | CoreSaveResult
  | { ok: false; reason: "unauth" | "validation" };

export async function save(input: unknown): Promise<SaveResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = saveInputSchema.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  return setSave(session.user.id, parsed.data);
}
