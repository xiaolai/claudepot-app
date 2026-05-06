/**
 * GET /api/v1/users/{username}/comments — author-scoped comment feed.
 *
 * Returns approved comments authored by `{username}`, newest first.
 * Tombstones are hidden (an author timeline of "[deleted]" rows
 * carries no signal — unlike a thread where the parent context
 * preserves it). Comments on deleted/unlisted submissions are also
 * excluded.
 */

import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { isUsername, parseCommentListParams } from "@/lib/api/inputs";
import { getUserByUsername, listCommentsByAuthor } from "@/lib/api/queries";
import { clampPageLimit } from "@/lib/api/cursor";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

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

  const SPEC = endpointSpec("users:list_comments");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  // depth is meaningless for an author timeline, but we accept it from
  // the shared parser and ignore it.
  const parsed = parseCommentListParams(new URL(req.url));
  if (!parsed.ok) {
    return problemResponse(
      validation("Query validation failed.", parsed.errors),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const user = await getUserByUsername(username);
  if (!user) return problemResponse(notFound("User not found."));

  const page = await listCommentsByAuthor({
    viewerId: auth.user.id,
    authorUsername: username,
    cursor: parsed.value.cursor,
    limit: clampPageLimit(parsed.value.limit),
    since: parsed.value.since,
  });
  return ok(page);
}
