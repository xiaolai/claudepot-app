/**
 * Bearer token authentication for /api/v1/* routes.
 *
 * Usage in a route handler:
 *
 *   const auth = await authenticate(req);
 *   if (!auth.ok) return problemResponse(auth.problem);
 *
 *   const scopeCheck = requireScope(auth.token, "submission:write");
 *   if (scopeCheck) return problemResponse(scopeCheck.problem);
 *
 *   const limit = await checkAndIncrement(auth.token.id, "submissions");
 *   if (!limit.ok) return problemResponse(rateLimited(...));
 *
 * Token last_used_at is bumped on every successful auth.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { users } from "@/db/schema";
import {
  findActiveTokenByPlaintext,
  markTokenUsed,
  TOKEN_FORMAT_RE,
  type ApiToken,
} from "./tokens";
import {
  unauthorized,
  forbidden,
  serviceUnavailable,
  type Problem,
} from "./errors";
import type { Scope } from "./scopes";

export type AuthUser = typeof users.$inferSelect;

export type AuthSuccess = { ok: true; token: ApiToken; user: AuthUser };
export type AuthFailure = { ok: false; problem: Problem };

// Tight upper bound on the bearer value length. Format is
// `shn_pat_<28 base64url>` (36 chars); cap a touch higher to reject
// pathological inputs before doing any work but still allow future
// prefix changes without code edits.
const MAX_BEARER_LEN = 64;

export async function authenticate(
  req: Request,
): Promise<AuthSuccess | AuthFailure> {
  const header = req.headers.get("authorization") ?? "";
  const match = /^Bearer\s+(\S+)$/i.exec(header);
  if (!match || match[1].length > MAX_BEARER_LEN || !TOKEN_FORMAT_RE.test(match[1])) {
    return {
      ok: false,
      problem: unauthorized(
        "Missing or malformed Authorization header. Expected: Authorization: Bearer shn_pat_<28 url-safe-base64 chars>",
      ),
    };
  }

  // DB calls can transiently fail (cold-start connect timeout, etc.). Trap
  // those here and surface a 503 problem instead of bubbling an opaque 500.
  // The detail is intentionally generic — raw exception messages can leak
  // hostnames, SQL fragments, and other ops information; full diagnostics
  // are logged server-side instead.
  let token: ApiToken | null;
  let user: AuthUser | undefined;
  try {
    token = await findActiveTokenByPlaintext(match[1]);
    if (!token) {
      return {
        ok: false,
        problem: unauthorized("Token is invalid, expired, or revoked"),
      };
    }
    const rows = await db
      .select()
      .from(users)
      .where(eq(users.id, token.userId))
      .limit(1);
    user = rows[0];
  } catch (err) {
    console.error("[api/auth] DB error during authenticate:", err);
    return { ok: false, problem: serviceUnavailable() };
  }

  if (!user) {
    return {
      ok: false,
      problem: unauthorized("Token references a deleted user"),
    };
  }
  if (user.role === "locked") {
    return {
      ok: false,
      problem: forbidden("This account is locked. The token cannot be used."),
    };
  }

  // Bump last_used_at in DB AND mirror onto the in-memory token so callers
  // (e.g. /api/v1/me) see the up-to-date timestamp without a refetch.
  const usedAt = new Date();
  try {
    await markTokenUsed(token.id);
  } catch {
    // last_used_at is observability, not security — skip on transient failure.
  }
  token.lastUsedAt = usedAt;

  return { ok: true, token, user };
}

export function hasScope(token: ApiToken, scope: Scope): boolean {
  return token.scopes.includes(scope);
}

/**
 * Returns null if the token holds the scope, or an AuthFailure to short-circuit.
 *
 *   const denied = requireScope(token, "submission:write");
 *   if (denied) return problemResponse(denied.problem);
 */
export function requireScope(token: ApiToken, scope: Scope): AuthFailure | null {
  if (hasScope(token, scope)) return null;
  return {
    ok: false,
    problem: forbidden(
      `This token is missing the required scope: ${scope}. Mint a new token at /settings/tokens with this scope enabled.`,
    ),
  };
}
