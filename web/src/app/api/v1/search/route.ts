/**
 * GET /api/v1/search — search across submissions or comments.
 *
 * Implementation note: kind=submission gates rows on the
 * Postgres FTS predicate `submissions.search_vec @@
 * websearch_to_tsquery('english', q)`. The predicate uses English
 * stemming and tokenization (so "running" matches "run"); pure
 * substring queries that don't tokenize cleanly may miss rows that
 * the v0 ILIKE behavior would have surfaced. Same column the
 * reader's /search page uses, so both surfaces share a single
 * regression blast-radius if the FTS column is ever dropped.
 *
 * kind=comment still uses ILIKE — comments have no FTS column, and
 * adding one is out of scope.
 *
 * Results are ordered by createdAt DESC for both kinds. Cursor
 * pagination is on (createdAt, id) — see lib/api/cursor.ts.
 *
 * Required: q (2–200 chars). Optional: kind (submission | comment),
 * since, cursor, limit, type[], tag[], author. The kind=submission
 * path reuses the same SubmissionDto + filters as /api/v1/submissions;
 * kind=comment returns CommentDto with depth=0 (no thread context).
 */

import { validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import { parseSearchParams } from "@/lib/api/inputs";
import { searchForApi } from "@/lib/api/queries";
import { clampPageLimit } from "@/lib/api/cursor";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (req: Request): Promise<Response> => {
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
});
