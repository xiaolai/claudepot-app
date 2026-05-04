/**
 * GET /api/v1/me — token introspection.
 *
 * Returns the authenticated user and the token metadata. This is the
 * entry-point clients use to verify a token works before doing real work,
 * so it deliberately:
 *
 *   - requires no scope (any active token can call it)
 *   - is exempt from rate limits (getting blocked here is hostile UX)
 *   - never returns secrets — only the displayPrefix that's safe to log
 *
 * The `last_used_at` bump happens inside `authenticate()`, so the very
 * first /me call will already register the token as "used".
 */

import { authenticate } from "@/lib/api/auth";
import { ok, preflight, problemResponse } from "@/lib/api/response";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const { user, token } = auth;

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
}
