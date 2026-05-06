/**
 * GET /api/v1/me/quota — daily-bucket introspection for the calling token.
 *
 * Returns the same counters that `checkAndIncrement` writes (see
 * api_token_usage), so the value is exactly what the next mutation
 * compares against. No scope required and no rate-limit charge:
 * blocking a token from reading its own quota would be hostile UX
 * — and the data leaked is the token's own usage, which it already
 * implicitly knows by counting its own requests.
 */

import { ok, preflight } from "@/lib/api/response";
import { readQuotaForToken } from "@/lib/api/quota";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(req: Request): Promise<Response> {
  const SPEC = endpointSpec("me:quota");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;

  // No-op for me:quota's null bucket but consistent with the manifest
  // invariant — see /me for the same comment.
  const charge = await chargeForSpec(SPEC, policy.auth.token.id);
  if (!charge.ok) return charge.response;

  const quota = await readQuotaForToken(policy.auth.token.id);
  return ok(quota);
}
