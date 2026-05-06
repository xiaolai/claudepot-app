/**
 * DELETE /api/v1/submissions/[id] — author-only soft delete via PAT.
 *
 * Mirrors deleteSubmission (web UI server action). The row stays in
 * the DB with `deleted_at` set; reads filter it out everywhere except
 * the staff queue.
 *
 * Author check happens inside deleteSubmissionAsAuthor; PAT scope is
 * enforced here. We charge against the `submissions` daily bucket so
 * a leaked token can't mass-delete past the daily limit.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import { forbidden, notFound, rateLimited } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { deleteSubmissionAsAuthor } from "@/lib/submissions";

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

  const denied = requireScope(auth.token, "submission:delete");
  if (denied) return problemResponse(denied.problem);

  const limit = await checkAndIncrement(auth.token.id, "submissions");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily submission-write limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await deleteSubmissionAsAuthor(auth.user.id, id);
  if (!result.ok) {
    if (result.reason === "forbidden") {
      return problemResponse(
        forbidden("You can only delete your own submissions."),
      );
    }
    return problemResponse(notFound("Submission not found."));
  }

  return ok({ id, deleted: true });
}
