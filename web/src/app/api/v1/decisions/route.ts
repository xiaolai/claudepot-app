/**
 * POST /api/v1/decisions — office-bot scoring decision write.
 *
 * Idempotent on (submissionId, appliedPersona, modelId, promptHash):
 * a re-POST of the same tuple returns 200 with the existing id and
 * created=false. First write returns 201 with created=true.
 *
 * This endpoint NEVER touches submissions.state. Publishing a draft
 * is a separate primitive (POST /api/v1/submissions/{id}/publish,
 * scope submission:publish) so the office decides when its policy is
 * satisfied. The polity stops encoding "one accept = publish."
 */

import { notFound, validation } from "@/lib/api/errors";
import {
  created,
  ok,
  preflight,
  problemResponse,
  withErrorHandling,
} from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import {
  decisionInputSchema,
  persistDecision,
} from "@/lib/editorial-writes";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("decisions:create");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = decisionInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Decision validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join(".") || "(root)",
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await persistDecision(parsed.data);

  if (!result.ok) {
    if (result.reason === "submission_not_found") {
      return problemResponse(
        notFound("No submission found for the supplied submissionId."),
      );
    }
    return problemResponse(validation(result.detail));
  }

  const payload = {
    id: result.decisionId,
    created: result.created,
  };
  return result.created ? created(payload) : ok(payload);
});
