/**
 * GET /api/v1/search — substring search across submissions or comments.
 *
 * Implementation note (PRD): callers don't depend on ranking — we use
 * Postgres ILIKE (with escaped wildcards) without a new FTS index for
 * the v0 cut. Results are ordered by createdAt DESC. Switching to
 * pg_trgm or full FTS is a follow-up that doesn't change the contract.
 *
 * Required: q (2–200 chars). Optional: kind (submission | comment),
 * since, cursor, limit, type[], tag[], author. The kind=submission
 * path reuses the same SubmissionDto + filters as /api/v1/submissions;
 * kind=comment returns CommentDto with depth=0 (no thread context).
 */

import { validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { parseSearchParams } from "@/lib/api/inputs";
import { searchForApi } from "@/lib/api/queries";
import { clampPageLimit } from "@/lib/api/cursor";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(req: Request): Promise<Response> {
  const SPEC = endpointSpec("search");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const parsed = parseSearchParams(new URL(req.url));
  if (!parsed.ok) {
    return problemResponse(
      validation("Query validation failed.", parsed.errors),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await searchForApi({
    viewerId: auth.user.id,
    q: parsed.value.q,
    kind: parsed.value.kind,
    cursor: parsed.value.cursor,
    limit: clampPageLimit(parsed.value.limit),
    since: parsed.value.since,
    types: parsed.value.types,
    tagSlugs: parsed.value.tagSlugs,
    authorUsername: parsed.value.authorUsername,
  });

  return ok({ ...result.page, query: parsed.value.q });
}
