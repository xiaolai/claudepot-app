/**
 * GET /api/v1/tags — list all tags with submission counts.
 *
 * Tag set is bounded (≤ a few hundred slugs) so no pagination — the
 * full list ships in one payload. Sort by count (default) or alpha.
 */

import { validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import { parseTagListParams } from "@/lib/api/inputs";
import { listTagsForApi } from "@/lib/api/queries";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("tags:list");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const parsed = parseTagListParams(new URL(req.url));
  if (!parsed.ok) {
    return problemResponse(
      validation("Query validation failed.", parsed.errors),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const items = await listTagsForApi(parsed.value.sort);
  return ok({ items });
});
