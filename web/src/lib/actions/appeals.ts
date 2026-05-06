"use server";

import { auth } from "@/lib/auth";
import {
  appealInputSchema,
  submitAppealAsAuthor,
  type AppealCoreResult,
} from "@/lib/appeals";

/**
 * Web UI server action — cookie-authenticated thin wrapper over
 * lib/appeals.ts:submitAppealAsAuthor. The REST endpoint at
 * /api/v1/appeals calls the same core with a PAT-authenticated
 * userId.
 *
 * Adds the 'unauth' arm (the REST surface authenticates at its
 * boundary, so the core never returns it).
 */

export type AppealResult =
  | AppealCoreResult
  | { ok: false; reason: "unauth" | "validation" };

export async function submitAppeal(input: unknown): Promise<AppealResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = appealInputSchema.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  return submitAppealAsAuthor(session.user.id, parsed.data);
}
