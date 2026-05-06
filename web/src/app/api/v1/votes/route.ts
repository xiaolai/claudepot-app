/**
 * POST /api/v1/votes — cast / change / clear a vote via PAT.
 *
 * Body: { submissionId: uuid, value: 1 | -1 | 0 }
 *
 *   value =  1 → upvote
 *   value = -1 → downvote (gated on karma >= 100, like the web UI)
 *   value =  0 → clear the existing vote
 *
 * The DB trigger handles score deltas; the action upserts on
 * (user_id, submission_id) so flipping is one row, not two.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import {
  forbidden,
  notFound,
  rateLimited,
  validation,
} from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { castVote, voteInputSchema } from "@/lib/votes";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "vote:write");
  if (denied) return problemResponse(denied.problem);

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = voteInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Vote validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const limit = await checkAndIncrement(auth.token.id, "votes");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily vote limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await castVote(auth.user.id, parsed.data);

  if (!result.ok) {
    if (result.reason === "missing_user") {
      // Token references a deleted user — already handled in
      // authenticate(), but the core may still detect this if the user
      // was deleted between auth and the vote. Surface as 401.
      return problemResponse({
        type: "https://claudepot.com/api/errors/unauthorized",
        title: "Unauthorized",
        status: 401,
        detail: "Token references a deleted user.",
      });
    }
    if (result.reason === "locked") {
      return problemResponse(forbidden("Account is locked."));
    }
    if (result.reason === "karma_gate") {
      return problemResponse(
        forbidden(
          "Downvotes require at least 100 karma. Your account is below the threshold.",
        ),
      );
    }
    return problemResponse(
      notFound("Submission not found, or not in a votable state."),
    );
  }

  return ok({ submissionId: parsed.data.submissionId, value: result.value });
}
