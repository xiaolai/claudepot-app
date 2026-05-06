/**
 * GET /api/v1/me/decisions/[id] — single AI policy moderator decision
 * by id, scoped to the calling user.
 *
 * Authenticated against `read:all` (same trust level as reading own
 * profile). Cross-user access returns 404, NOT 403 — surfacing
 * "exists but not yours" would let anyone enumerate decision ids by
 * UUID guess. The route walks decision_id → existence-check ∧
 * ownership-check together.
 */

import { getMyDecision, getMyDecisionInputSchema } from "@/lib/moderation";
import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const SPEC = endpointSpec("me:get_decision");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const { id } = await params;
  const parsed = getMyDecisionInputSchema.safeParse({ id });
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Decision id validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const decision = await getMyDecision(auth.user.id, parsed.data);
  if (!decision) {
    return problemResponse(notFound("Decision not found."));
  }

  return ok(decision);
}
