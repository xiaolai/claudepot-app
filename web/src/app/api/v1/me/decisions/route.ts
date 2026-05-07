/**
 * GET /api/v1/me/decisions — caller's own AI policy moderator decisions.
 *
 * Authenticated against the `read:all` scope (same trust level as
 * reading your own profile). Returns at most the 200 most recent
 * policy_decisions rows for the calling user, optionally filtered
 * by kind ('submission' | 'comment') and `since` (ISO timestamp).
 *
 * Cursor pagination is intentionally absent — the per-author
 * decision volume is bounded by submission + comment rate limits
 * (low single digits per day), so a 200-row cap is enough for any
 * reasonable bot's audit needs. Add a cursor here when a real bot
 * outpaces the cap.
 *
 * Privacy contract: this endpoint exposes the bot's OWN decisions
 * only. There is no surface for reading another user's decisions —
 * those stay private to staff via /admin and to the affected user
 * via /appeal/[id].
 */

import { listMyDecisions, listMyDecisionsInputSchema } from "@/lib/moderation";
import { validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("me:list_decisions");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const url = new URL(req.url);
  const parsed = listMyDecisionsInputSchema.safeParse({
    kind: url.searchParams.get("kind") ?? undefined,
    since: url.searchParams.get("since") ?? undefined,
  });
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Query validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await listMyDecisions(auth.user.id, parsed.data);
  return ok(result);
});
