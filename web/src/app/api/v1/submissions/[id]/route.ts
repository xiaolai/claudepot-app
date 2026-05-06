/**
 * Author-only mutations on a single submission via PAT.
 *
 *   PATCH  /api/v1/submissions/[id]  → update title / text
 *   DELETE /api/v1/submissions/[id]  → soft delete
 *
 * Both mirror the equivalent web UI server actions. Author + window
 * checks happen inside the cores; PAT scopes are enforced here. Both
 * verbs charge against the `submissions` daily bucket so a leaked
 * token can't drain mutations past the daily limit.
 *
 * The PATCH path is the bot edit story: human users still hit the
 * 5-minute window the web action has always enforced, but bots
 * (is_agent OR role IN system/staff) bypass the window. Out-of-window
 * edits set submissions.updated_at so the UI can render an "edited"
 * badge.
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
  deleteSubmissionAsAuthor,
  updateSubmissionAsAuthor,
  updateSubmissionInputSchema,
} from "@/lib/submissions";

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

export async function PATCH(
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const { id } = await params;
  if (!UUID_RE.test(id)) return problemResponse(notFound("Invalid id."));

  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "submission:update");
  if (denied) return problemResponse(denied.problem);

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = updateSubmissionInputSchema.safeParse(body);
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

  const limit = await checkAndIncrement(auth.token.id, "submissions");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily submission-write limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await updateSubmissionAsAuthor(auth.user.id, id, parsed.data);
  if (!result.ok) {
    if (result.reason === "forbidden") {
      return problemResponse(
        forbidden("You can only edit your own submissions."),
      );
    }
    if (result.reason === "expired") {
      return problemResponse(
        forbidden(
          "Edit window expired. Edits are accepted within 5 minutes of posting; bot tokens (is_agent / system / staff) bypass the window.",
        ),
      );
    }
    if (result.reason === "invalid") {
      return problemResponse(
        validation(result.detail ?? "Update would violate the URL/text invariant."),
      );
    }
    if (result.reason === "noop") {
      // Treat as success — no fields changed, idempotent.
      return ok({ id, edited: false, silent: true, updatedAt: null });
    }
    return problemResponse(notFound("Submission not found."));
  }

  return ok({
    id,
    edited: true,
    silent: result.silent,
    updatedAt: result.updatedAt?.toISOString() ?? null,
  });
}
