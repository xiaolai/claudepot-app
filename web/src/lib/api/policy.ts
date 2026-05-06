/**
 * Manifest-driven policy enforcement for /api/v1/* routes.
 *
 * Every route handler reads its scope and rate-limit bucket from
 * lib/api/manifest.ts via these helpers — string literals for
 * scope/bucket are not allowed in route files anymore. The drift
 * test (tests/api-manifest.test.ts) enforces that every route
 * file's `endpointSpec("...")` reference resolves to a real
 * manifest entry.
 *
 * The helpers are intentionally small and composable so handlers
 * can interleave validation, conditional bucket charging (304s,
 * authorship checks), or per-resource auth (decision-record
 * authorship) between the auth and charge steps.
 */

import {
  type AuthSuccess,
  authenticate,
  requireScope,
} from "./auth";
import { rateLimited } from "./errors";
import { problemResponse } from "./response";
import { checkAndIncrement, type LimitCategory } from "./rate-limit";
import type { EndpointSpec } from "./manifest";

/**
 * Authenticate + requireScope per the spec's `auth` field. Does
 * NOT charge the rate-limit bucket — call `chargeForSpec()` at the
 * point the handler is ready (usually after path/query validation
 * so a 422 doesn't consume budget).
 *
 * Returns the authenticated user on success, or a ready-to-return
 * Problem response on auth/scope failure. Calling this on a "public"
 * spec throws — public endpoints don't authenticate, and reaching
 * this function for one indicates a route-level bug.
 */
export async function checkAuthForSpec(
  req: Request,
  spec: EndpointSpec,
): Promise<{ ok: true; auth: AuthSuccess } | { ok: false; response: Response }> {
  if (spec.auth === "public") {
    throw new Error(
      `checkAuthForSpec invoked for public endpoint "${spec.id}". ` +
        `Public endpoints must not authenticate.`,
    );
  }
  const auth = await authenticate(req);
  if (!auth.ok) {
    return { ok: false, response: problemResponse(auth.problem) };
  }
  if (spec.auth !== "any") {
    // spec.auth is narrowed to Scope here — "public" excluded above
    // and "any" excluded by this branch, leaving only Scope.
    const denied = requireScope(auth.token, spec.auth);
    if (denied) {
      return { ok: false, response: problemResponse(denied.problem) };
    }
  }
  return { ok: true, auth };
}

/**
 * Bucket name → singular noun for user-facing 429 messages. Keeps
 * "Daily comment-write limit (...)" consistent across all endpoints
 * that hit the comments bucket, regardless of which one tripped it.
 */
const RATE_LIMIT_NOUN: Record<LimitCategory, string> = {
  reads: "read",
  submissions: "submission-write",
  comments: "comment-write",
  votes: "vote",
  saves: "save",
};

/**
 * Charge the spec's rate-limit bucket. No-op when `spec.bucket` is
 * null (public endpoints, /me, /me/quota). Returns either ok or a
 * 429 Problem response carrying the next-reset timestamp.
 */
export async function chargeForSpec(
  spec: EndpointSpec,
  tokenId: string,
): Promise<{ ok: true } | { ok: false; response: Response }> {
  if (spec.bucket === null) return { ok: true };
  const limit = await checkAndIncrement(tokenId, spec.bucket);
  if (limit.ok) return { ok: true };
  return {
    ok: false,
    response: problemResponse(
      rateLimited(
        `Daily ${RATE_LIMIT_NOUN[spec.bucket]} limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    ),
  };
}

/**
 * For staff-only branches (e.g. clamping `state=pending` to approved
 * for non-staff). The role check is duplicated across several routes;
 * keeping it here ensures everyone agrees what "staff" means.
 */
export function isStaffAuth(auth: AuthSuccess): boolean {
  return auth.user.role === "staff";
}
