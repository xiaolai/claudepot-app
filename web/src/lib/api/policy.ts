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
import { forbidden, rateLimited } from "./errors";
import { problemResponse } from "./response";
import { checkAndIncrement, type LimitCategory } from "./rate-limit";
import type { EndpointSpec } from "./manifest";
// CITIZEN_BOT_DENIED_SCOPES + the privileged-scope pair live in
// scopes.ts (pure, DB-free) so unit tests and the MCP twin
// (lib/mcp/policy.ts) share the exact same constants.
import {
  canHoldPrivilegedScopes,
  CITIZEN_BOT_DENIED_SCOPES,
  PRIVILEGED_SCOPES,
} from "./scopes";

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
    // Citizen-bot defense-in-depth: refuse to act on the deny list
    // even if the PAT carries the scope. Belt-and-braces with the
    // CITIZEN_SCOPES filter at PAT mint time.
    if (
      auth.user.botKind === "citizen" &&
      CITIZEN_BOT_DENIED_SCOPES.has(spec.auth)
    ) {
      return {
        ok: false,
        response: problemResponse(
          forbidden(
            `Citizen bots cannot perform "${spec.auth}". This action is for human users and operator-owned bots only. See web/dev-docs/citizen-bots.md.`,
          ),
        ),
      };
    }
    // Privileged-scope defense-in-depth: the office-only scopes
    // (editorial writes, bot self-reporting) require the token's
    // OWNER to be entitled, not just the token to carry the scope.
    // Mirrors the publish route's is_agent gate and the mint-time
    // check in lib/actions/api-tokens.ts — a token minted before
    // that gate existed (or via a future mint bug) is still refused
    // here.
    if (
      PRIVILEGED_SCOPES.has(spec.auth) &&
      !canHoldPrivilegedScopes(auth.user)
    ) {
      return {
        ok: false,
        response: problemResponse(
          forbidden(
            `"${spec.auth}" is reserved for staff and bot accounts. This token's owner is not entitled to it.`,
          ),
        ),
      };
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
  bots: "bot-report",
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
 *
 * Both `staff` (humans) and `system` (Ada and other agent accounts)
 * are admitted — same definition the web shell uses in
 * `lib/staff.ts:requireStaffId` and `lib/staff-gate.tsx:staffGate`.
 * Without this symmetry, the same `system` account sees different
 * pending-content visibility through a PAT than through a web
 * session.
 */
export function isStaffAuth(auth: AuthSuccess): boolean {
  const role = auth.user.role;
  return role === "staff" || role === "system";
}
