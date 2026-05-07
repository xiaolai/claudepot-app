/**
 * GET /api/v1/tags/{slug} — tag-scoped feed.
 *
 * Returns the same shape as /api/v1/submissions but filtered to
 * submissions tagged with `{slug}`. The `tag` query param is silently
 * overridden — the path is the truth — and the response carries the
 * resolved tag at the top level for convenience.
 */

import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import { parseSubmissionListParams } from "@/lib/api/inputs";
import { getTagBySlugForApi, listSubmissions } from "@/lib/api/queries";
import { clampPageLimit } from "@/lib/api/cursor";
import { endpointSpec } from "@/lib/api/manifest";
import {
  chargeForSpec,
  checkAuthForSpec,
  isStaffAuth,
} from "@/lib/api/policy";

const SLUG_RE = /^[a-z0-9-]{1,40}$/;

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ slug: string }> },
): Promise<Response> => {
  const { slug } = await params;
  if (!SLUG_RE.test(slug)) {
    return problemResponse(notFound("Invalid tag slug."));
  }

  const SPEC = endpointSpec("tags:get");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const parsed = parseSubmissionListParams(new URL(req.url));
  if (!parsed.ok) {
    return problemResponse(
      validation("Query validation failed.", parsed.errors),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const tag = await getTagBySlugForApi(slug);
  if (!tag) return problemResponse(notFound("Tag not found."));

  const viewerIsStaff = isStaffAuth(auth);
  const effectiveState =
    parsed.value.state === "pending" && !viewerIsStaff
      ? "approved"
      : parsed.value.state;

  const page = await listSubmissions({
    viewerId: auth.user.id,
    sort: parsed.value.sort,
    cursor: parsed.value.cursor,
    limit: clampPageLimit(parsed.value.limit),
    since: parsed.value.since,
    types: parsed.value.types,
    // Force the path slug; ignore any tag= override.
    tagSlugs: [slug],
    authorUsername: parsed.value.authorUsername,
    state: effectiveState,
  });

  return ok({ tag, ...page });
});
