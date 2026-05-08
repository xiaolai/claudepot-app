/**
 * GET /api/v1/submissions/{id}/decisions — public list of every
 * editorial decision on a submission.
 *
 * Differs from /api/v1/submissions/{id}/decision (singular,
 * author-only) in two ways:
 *   1. Plural — returns the array, not the single most-recent.
 *   2. Public (read:all) — the /office/ window shows these
 *      decisions to anyone, so an authenticated read scope is
 *      sufficient. Per editorial/transparency.md §1, the
 *      privacy-relevant fields (raw weights, prompt body) are
 *      already excluded from the OfficeDecision DTO.
 *
 * Order: scoredAt asc — chronological, so the reader can watch
 * the office's stance on a submission evolve over time.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions } from "@/db/schema";
import { notFound } from "@/lib/api/errors";
import { ok, preflight, problemResponse, withErrorHandling } from "@/lib/api/response";
import { getDecisionsBySubmission } from "@/db/office-queries";
import { isUuid } from "@/lib/api/inputs";
import { endpointSpec } from "@/lib/api/manifest";
import { buildPublicOfficeDecisionDto } from "@/lib/api/office-decision-dto";
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

  const SPEC = endpointSpec("submissions:list_decisions");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const [sub] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(eq(submissions.id, id))
    .limit(1);
  if (!sub) return problemResponse(notFound("Submission not found."));

  const decisions = await getDecisionsBySubmission(id);
  return ok({
    submissionId: id,
    decisions: decisions.map(buildPublicOfficeDecisionDto),
  });
});
