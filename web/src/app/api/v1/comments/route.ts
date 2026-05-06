/**
 * POST /api/v1/comments — post a comment or reply via PAT.
 *
 * Mirrors submitComment (web UI server action) but takes the author
 * identity from a Bearer token instead of a cookie session. Replies
 * are created by passing parentId; the parent must belong to the same
 * submissionId.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import {
  forbidden,
  notFound,
  rateLimited,
  validation,
} from "@/lib/api/errors";
import { created, preflight, problemResponse } from "@/lib/api/response";
import { commentInputSchema, createComment } from "@/lib/comments";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "comment:write");
  if (denied) return problemResponse(denied.problem);

  // Parse + validate before bumping the rate-limit bucket — see the
  // submissions route for the rationale.
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = commentInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Comment validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const limit = await checkAndIncrement(auth.token.id, "comments");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily comment limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await createComment(auth.user.id, parsed.data);

  if (!result.ok) {
    if (result.reason === "not_found") {
      return problemResponse(
        notFound("Submission or parent comment not found."),
      );
    }
    if (result.reason === "locked") {
      // The "locked" outcome covers both an account lock (the user's
      // role is "locked") and a submission lock (the thread is closed
      // to new comments). Keep the message neutral so it covers both.
      return problemResponse(
        forbidden(
          "Your account is locked, or this submission is closed to new comments.",
        ),
      );
    }
    return problemResponse(validation("Comment failed."));
  }

  return created(
    {
      id: result.commentId,
      pending: result.pending,
      url: `https://claudepot.com/post/${parsed.data.submissionId}#comment-${result.commentId}`,
    },
    `https://claudepot.com/post/${parsed.data.submissionId}#comment-${result.commentId}`,
  );
}
