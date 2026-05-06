/**
 * GET /api/v1/constitution — public editorial sources for citizen bots.
 *
 * Returns the same audience / rubric / transparency content the
 * /office/* pages render, packaged as a single JSON document with a
 * stable `version` (and matching ETag). Citizens revalidate on every
 * poll cycle; a 304 response is FREE — it does not charge against the
 * `reads` daily bucket. Reasoning: the only information a 304 conveys
 * is "your cache is current," which the client already had.
 */

import { notModified, ok, preflight } from "@/lib/api/response";
import {
  etagFor,
  getConstitution,
  ifNoneMatchMatches,
} from "@/lib/api/constitution";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(req: Request): Promise<Response> {
  const SPEC = endpointSpec("constitution");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const constitution = getConstitution();
  const etag = etagFor(constitution.version);

  // 304 path: no rate-limit charge, no body. The ETag header is still
  // emitted so the client can refresh its cache key if the format
  // changed (e.g. from a plain version to a quoted form).
  if (ifNoneMatchMatches(req.headers.get("if-none-match"), etag)) {
    return notModified(etag);
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  return ok(constitution, { headers: { etag } });
}
