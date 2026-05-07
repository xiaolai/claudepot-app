/**
 * POST /api/v1/appeals — appeal a policy_decisions reject.
 *
 * Authenticates against any active token (auth: "any"). The caller
 * must own the decision; cross-user appeals return 403. Server-
 * enforced one-open-appeal-per-target — duplicates return 409.
 *
 * Mirrors the web UI's submitAppeal server action via the shared
 * core in lib/appeals.ts:submitAppealAsAuthor. The flag row that
 * the call inserts shows up in /admin/queue alongside community
 * flags, tagged 'appeal: <text>' so staff can distinguish.
 *
 * Bucket: null. Rate-limiting comes from the server-side dedup
 * check (one open appeal per target) — a bot rapid-firing the
 * same decisionId gets `duplicate` after the first success.
 */

import { conflict, forbidden, notFound, validation } from "@/lib/api/errors";
import { created, preflight, problemResponse } from "@/lib/api/response";
import { appealInputSchema, submitAppealAsAuthor } from "@/lib/appeals";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const SPEC = endpointSpec("appeals:create");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = appealInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Appeal validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  // bucket: null on this spec, so chargeForSpec is a no-op. Kept
  // for symmetry with the rest of the policy contract.
  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await submitAppealAsAuthor(auth.user.id, parsed.data);

  if (!result.ok) {
    if (result.reason === "not_found") {
      return problemResponse(notFound("Decision not found."));
    }
    if (result.reason === "forbidden") {
      return problemResponse(
        forbidden("You can only appeal your own decisions."),
      );
    }
    if (result.reason === "duplicate") {
      return problemResponse(
        conflict("An open appeal for this decision already exists."),
      );
    }
    if (result.reason === "stale") {
      return problemResponse({
        type: "https://claudepot.com/api/errors/stale",
        title: "Decision no longer appealable",
        status: 410,
        detail:
          "This decision has been resolved already, or the targeted content was deleted.",
      });
    }
    return problemResponse(validation("Appeal failed."));
  }

  return created(
    {
      flagId: result.flagId,
      decisionId: parsed.data.decisionId,
    },
    `https://claudepot.com/admin`,
  );
}
