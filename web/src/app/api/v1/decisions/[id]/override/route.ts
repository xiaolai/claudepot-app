/**
 * POST /api/v1/decisions/{id}/override — office-bot override of an
 * existing decision_records row.
 *
 * The endpoint is PAT-authenticated and requires `decision:override`.
 * reviewer_kind is forced to 'bot' here — the human-staff override
 * flow stays in /admin/console (it doesn't share this endpoint).
 *
 * This endpoint NEVER touches submissions.state. If the office wants
 * an override to publish (or unpublish) a submission, it must call
 * POST /api/v1/submissions/{id}/publish separately. Decoupling the
 * decision record from the visibility flip keeps editorial policy
 * out of the polity.
 */

import { notFound, validation } from "@/lib/api/errors";
import {
  created,
  preflight,
  problemResponse,
  withErrorHandling,
} from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import {
  overrideInputSchema,
  persistOverride,
} from "@/lib/editorial-writes";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(
  async (
    req: Request,
    ctx: { params: Promise<{ id: string }> },
  ): Promise<Response> => {
    const SPEC = endpointSpec("decisions:override");
    const policy = await checkAuthForSpec(req, SPEC);
    if (!policy.ok) return policy.response;
    const { auth } = policy;

    const { id } = await ctx.params;
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id)) {
      return problemResponse(
        validation("Path parameter 'id' must be a UUID."),
      );
    }

    let body: unknown;
    try {
      body = await req.json();
    } catch {
      return problemResponse(validation("Request body must be valid JSON."));
    }

    const parsed = overrideInputSchema.safeParse(body);
    if (!parsed.success) {
      return problemResponse(
        validation(
          "Override validation failed.",
          parsed.error.issues.map((i) => ({
            field: i.path.join(".") || "(root)",
            message: i.message,
          })),
        ),
      );
    }

    const charge = await chargeForSpec(SPEC, auth.token.id);
    if (!charge.ok) return charge.response;

    const result = await persistOverride(id, auth.user.id, parsed.data);
    if (!result.ok) {
      if (result.reason === "decision_not_found") {
        return problemResponse(
          notFound("No decision_records row found for the supplied id."),
        );
      }
      return problemResponse(validation(result.detail));
    }

    return created({ id: result.overrideId });
  },
);
