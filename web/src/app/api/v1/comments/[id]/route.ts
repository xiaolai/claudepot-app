/**
 * /api/v1/comments/[id] — single-comment verbs.
 *
 *   GET    — public read. Returns CommentDetailDto including a
 *            compact reference to the parent submission.
 *   PATCH  — author-only edit. Bots bypass the 5-minute window.
 *   DELETE — author-only delete. Soft-delete with replies, hard-delete
 *            without.
 */

import { forbidden, notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import {
  deleteCommentAsAuthor,
  updateCommentAsAuthor,
  updateCommentInputSchema,
} from "@/lib/comments";
import { getCommentByIdForApi } from "@/lib/api/queries";
import { isUuid } from "@/lib/api/inputs";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> => {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("comments:get");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const dto = await getCommentByIdForApi(auth.user.id, id);
  if (!dto) return problemResponse(notFound("Comment not found."));
  return ok(dto);
});

export const DELETE = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> => {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("comments:delete");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

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
});

export const PATCH = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> => {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("comments:update");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

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

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

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
});
