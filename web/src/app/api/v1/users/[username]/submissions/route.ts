/**
 * GET /api/v1/users/{username}/submissions — author-scoped feed.
 *
 * Same shape and filters as GET /api/v1/submissions, with the
 * `author` filter forced to the path username. A user-supplied
 * ?author= in the query string is silently overridden — the path
 * is the truth.
 *
 * Pending submissions are not returned (citizens see approved-only,
 * including for their own user — same policy as the public feed).
 */

import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { isUsername, parseSubmissionListParams } from "@/lib/api/inputs";
import { getUserByUsername, listSubmissions } from "@/lib/api/queries";
import { clampPageLimit } from "@/lib/api/cursor";
import { endpointSpec } from "@/lib/api/manifest";
import {
  chargeForSpec,
  checkAuthForSpec,
  isStaffAuth,
} from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(
  req: Request,
  { params }: { params: Promise<{ username: string }> },
): Promise<Response> {
  const { username } = await params;
  if (!isUsername(username)) {
    return problemResponse(notFound("Invalid username."));
  }

  const SPEC = endpointSpec("users:list_submissions");
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

  // Resolve user up front so a missing username returns 404 rather
  // than an empty page (which is the API contract for "no results").
  const user = await getUserByUsername(username);
  if (!user) return problemResponse(notFound("User not found."));

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
    tagSlugs: parsed.value.tagSlugs,
    authorUsername: username,
    state: effectiveState,
  });

  return ok(page);
}
