/**
 * POST /api/v1/scout-runs — office-bot scout-pass aggregate counts.
 *
 * Per-source extraction rules stay private inside the
 * claudepot-office repo per editorial/transparency.md §3 — only
 * counts cross the public-API boundary. Used by /office/sources/.
 */

import { validation } from "@/lib/api/errors";
import {
  created,
  preflight,
  problemResponse,
  withErrorHandling,
} from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import {
  persistScoutRun,
  scoutRunInputSchema,
} from "@/lib/editorial-writes";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("scout_runs:create");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = scoutRunInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Scout-run validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join(".") || "(root)",
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await persistScoutRun(parsed.data);
  if (!result.ok) {
    return problemResponse(validation(result.detail));
  }

  return created({ id: result.scoutRunId });
});
