/**
 * /api/v1/submissions/[id] — single-submission verbs.
 *
 *   GET    — public read.
 *   PATCH  — author-only edit (title / text). Bots bypass the 5-min
 *            window humans hit.
 *   DELETE — author-only soft delete.
 *
 * Scope and rate-limit policy are sourced from lib/api/manifest.ts.
 * Author + window checks live inside the cores in lib/submissions.ts;
 * this file only enforces the API-edge concerns (path validation,
 * auth gate, bucket charge, response shaping).
 */

import { forbidden, notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import {
  deleteSubmissionAsAuthor,
  updateSubmissionAsAuthor,
  updateSubmissionInputSchema,
} from "@/lib/submissions";
import { getSubmissionByIdForApi } from "@/lib/api/queries";
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

  const SPEC = endpointSpec("submissions:get");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const dto = await getSubmissionByIdForApi(auth.user.id, id);
  if (!dto) return problemResponse(notFound("Submission not found."));
  return ok(dto);
});

export const DELETE = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> => {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("submissions:delete");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

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
});

export const PATCH = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> => {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("submissions:update");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

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

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

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
});
