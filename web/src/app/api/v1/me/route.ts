/**
 * GET /api/v1/me — token introspection.
 *
 * Returns the authenticated user and the token metadata. This is the
 * entry-point clients use to verify a token works before doing real
 * work, so it deliberately:
 *
 *   - requires no scope (any active token can call it)
 *   - is exempt from rate limits (getting blocked here is hostile UX)
 *   - never returns secrets — only the displayPrefix that's safe to log
 *
 * The `last_used_at` bump happens inside `authenticate()`, so the very
 * first /me call will already register the token as "used".
 *
 * The manifest spec confirms `auth: "any"` and `bucket: null` — the
 * helper handles both shapes; chargeForSpec is a no-op here.
 */

import { ok, preflight , withErrorHandling } from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("me:identify");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { user, token } = policy.auth;

  // chargeForSpec is a no-op when spec.bucket is null (as it is for
  // /me), but the manifest invariant says every authed route runs
  // both halves of the policy contract. Keeps this route honest if a
  // future manifest change wires /me to a bucket.
  const charge = await chargeForSpec(SPEC, token.id);
  if (!charge.ok) return charge.response;

  return ok({
    user: {
      id: user.id,
      username: user.username,
      name: user.name,
      avatarUrl: user.avatarUrl,
      role: user.role,
      isAgent: user.isAgent,
      karma: user.karma,
    },
    token: {
      id: token.id,
      name: token.name,
      displayPrefix: token.displayPrefix,
      scopes: token.scopes,
      lastUsedAt: token.lastUsedAt,
      expiresAt: token.expiresAt,
      createdAt: token.createdAt,
    },
  });
});
