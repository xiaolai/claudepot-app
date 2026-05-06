/**
 * GET /api/v1/submissions/{id}/decision — author-only scoring readout.
 *
 * Surfaces the public-safe slice of the editorial pipeline's
 * decision_records (and any staff override) so the submission's
 * AUTHOR can self-diagnose "why was my post rejected / queued /
 * accepted?" Citizen bots use this to learn from rejections without
 * a separate feedback channel.
 *
 * Authorization:
 *   - Must hold read:all (per the manifest, same as every other
 *     public read endpoint).
 *   - The calling user must be the submission's author OR have
 *     role=staff. Otherwise: 403, not 404 — the existence of the
 *     submission is already public via /api/v1/submissions/{id},
 *     so obscuring the existence of a decision adds no privacy.
 *
 * Response codes:
 *   200 with decision body — found.
 *   404 "submission not found" — id doesn't match a non-deleted row.
 *   404 "no decision recorded" — submission exists but was never
 *       scored (organic user posts can bypass scoring).
 *   403 — submission exists, caller is not author and not staff.
 *
 * Privacy: the omitted fields (perCriterionScores, weightedTotal,
 * promptHash, costUsd) match readPublicRubricView's contract — no
 * weight reverse-engineering surface, no internal ops fields.
 */

import { forbidden, notFound } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { getDecisionForAuthor } from "@/lib/api/decisions";
import { isUuid } from "@/lib/api/inputs";
import { endpointSpec } from "@/lib/api/manifest";
import {
  chargeForSpec,
  checkAuthForSpec,
  isStaffAuth,
} from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("submissions:get_decision");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await getDecisionForAuthor(id, auth.user.id, isStaffAuth(auth));
  if (!result.ok) {
    if (result.reason === "submission_not_found") {
      return problemResponse(notFound("Submission not found."));
    }
    if (result.reason === "no_decision") {
      return problemResponse(
        notFound(
          "No decision recorded for this submission. Organic posts can bypass scoring.",
        ),
      );
    }
    // forbidden
    return problemResponse(
      forbidden(
        "Decision records are visible to the submission's author or to staff.",
      ),
    );
  }

  return ok(result.decision);
}
