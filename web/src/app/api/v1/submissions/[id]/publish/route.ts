/**
 * POST /api/v1/submissions/{id}/publish — flip an office-controlled
 * submission between state='draft' and state='approved'.
 *
 * Body: { publish: boolean }
 *   publish=true  → draft → approved (sets publishedAt = now())
 *   publish=false → approved → draft (clears publishedAt)
 *
 * Idempotent: re-POSTing the current state is a noop with
 * outcome='unchanged'. Rejected: 'pending', 'rejected', 'locked'
 * states are not part of the office's draft↔approved cycle.
 *
 * Authorization:
 *   - Token holds scope `submission:publish` (granted to office
 *     bots only).
 *   - Calling user is is_agent=true. Without this, even a leaked
 *     token can't reach the primitive.
 *   - The submission's author is is_agent=true. Citizen submissions
 *     stay under Ada / staff control — bots can't toggle them.
 */

import { z } from "zod";

import { forbidden, notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse, withErrorHandling } from "@/lib/api/response";
import { isUuid } from "@/lib/api/inputs";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import { publishSubmission } from "@/lib/submissions";

const publishInputSchema = z.object({
  publish: z.boolean(),
});

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(
  async (
    req: Request,
    ctx: { params: Promise<{ id: string }> },
  ): Promise<Response> => {
    const SPEC = endpointSpec("submissions:publish");
    const policy = await checkAuthForSpec(req, SPEC);
    if (!policy.ok) return policy.response;
    const { auth } = policy;

    if (!auth.user.isAgent) {
      return problemResponse(
        forbidden(
          "submission:publish is callable only from bot accounts (users.is_agent=true).",
        ),
      );
    }

    const { id } = await ctx.params;
    if (!isUuid(id)) {
      return problemResponse(notFound("Invalid id."));
    }

    let body: unknown;
    try {
      body = await req.json();
    } catch {
      return problemResponse(validation("Request body must be valid JSON."));
    }

    const parsed = publishInputSchema.safeParse(body);
    if (!parsed.success) {
      return problemResponse(
        validation(
          "Publish input validation failed.",
          parsed.error.issues.map((i) => ({
            field: i.path.join(".") || "(root)",
            message: i.message,
          })),
        ),
      );
    }

    const charge = await chargeForSpec(SPEC, auth.token.id);
    if (!charge.ok) return charge.response;

    const result = await publishSubmission(id, parsed.data.publish);
    if (!result.ok) {
      if (result.reason === "submission_not_found") {
        return problemResponse(notFound("Submission not found."));
      }
      if (result.reason === "not_office_owned") {
        return problemResponse(
          forbidden(
            result.detail ??
              "Publish primitive is only valid on bot-authored submissions.",
          ),
        );
      }
      // wrong_state
      return problemResponse(
        validation(
          result.detail ??
            "Submission is not in a state the publish primitive can transition.",
        ),
      );
    }

    return ok({
      submissionId: id,
      outcome: result.outcome,
      state: result.state,
    });
  },
);
