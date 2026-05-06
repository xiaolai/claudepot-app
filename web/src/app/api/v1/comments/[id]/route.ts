/**
 * Author-only mutations on a single comment via PAT.
 *
 *   PATCH  /api/v1/comments/[id]  → update body
 *   DELETE /api/v1/comments/[id]  → soft (with replies) or hard delete
 *
 * Both mirror the equivalent web UI server actions. Author + window
 * checks happen inside the cores; PAT scopes are enforced here. Both
 * verbs charge against the `comments` daily bucket.
 *
 * The PATCH path: human users still hit the 5-minute window the web
 * action enforces; bots (is_agent OR role IN system/staff) bypass it.
 * Out-of-window edits set comments.updated_at so the UI can render
 * an "edited" badge.
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
import {
  deleteCommentAsAuthor,
  updateCommentAsAuthor,
  updateCommentInputSchema,
} from "@/lib/comments";

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

export async function PATCH(
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const { id } = await params;
  if (!UUID_RE.test(id)) return problemResponse(notFound("Invalid id."));

  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "comment:update");
  if (denied) return problemResponse(denied.problem);

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = updateCommentInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Update validation failed.",
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
        `Daily comment-write limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await updateCommentAsAuthor(auth.user.id, id, parsed.data);
  if (!result.ok) {
    if (result.reason === "forbidden") {
      return problemResponse(
        forbidden("You can only edit your own comments."),
      );
    }
    if (result.reason === "expired") {
      return problemResponse(
        forbidden(
          "Edit window expired. Edits are accepted within 5 minutes of posting; bot tokens (is_agent / system / staff) bypass the window.",
        ),
      );
    }
    if (result.reason === "noop") {
      return ok({ id, edited: false, silent: true, updatedAt: null });
    }
    return problemResponse(notFound("Comment not found."));
  }

  return ok({
    id,
    edited: true,
    silent: result.silent,
    submissionId: result.submissionId,
    updatedAt: result.updatedAt?.toISOString() ?? null,
  });
}
