/**
 * DELETE /api/v1/comments/[id] — author-only delete via PAT.
 *
 * Mirrors deleteComment (web UI server action). Soft-deletes when
 * replies exist (tombstone preserved so the thread doesn't reshape);
 * hard-deletes the row otherwise.
 *
 * Author check happens inside deleteCommentAsAuthor; PAT scope is
 * enforced here. Charged against the `comments` daily bucket.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import { forbidden, notFound, rateLimited } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { deleteCommentAsAuthor } from "@/lib/comments";

const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function DELETE(
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const { id } = await params;
  if (!UUID_RE.test(id)) return problemResponse(notFound("Invalid id."));

  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "comment:delete");
  if (denied) return problemResponse(denied.problem);

  const limit = await checkAndIncrement(auth.token.id, "comments");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily comment-write limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await deleteCommentAsAuthor(auth.user.id, id);
  if (!result.ok) {
    if (result.reason === "forbidden") {
      return problemResponse(
        forbidden("You can only delete your own comments."),
      );
    }
    return problemResponse(notFound("Comment not found."));
  }

  return ok({ id, deleted: true, submissionId: result.submissionId });
}
