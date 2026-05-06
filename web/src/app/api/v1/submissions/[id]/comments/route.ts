/**
 * GET /api/v1/submissions/{id}/comments — flat comment listing for a
 * submission. Tombstones included with body=null. Pending and rejected
 * comments are hidden, as are comments whose parent submission is not
 * publicly visible (deleted, unlisted, or unapproved).
 *
 * Ordering: (createdAt ASC, id ASC). Clients reconstruct the tree from
 * parentId. The order tuple matches the cursor tuple, which is the
 * keyset-pagination correctness contract. Replies past `depth`
 * (default 5, max 20) are trimmed and the parent gets
 * `hasMoreReplies: true`.
 */

import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { parseCommentListParams, isUuid } from "@/lib/api/inputs";
import { listSubmissionComments } from "@/lib/api/queries";
import { clampPageLimit } from "@/lib/api/cursor";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const { id } = await params;
  if (!isUuid(id)) {
    return problemResponse(notFound("Invalid submission id."));
  }

  const SPEC = endpointSpec("submissions:list_comments");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const parsed = parseCommentListParams(new URL(req.url));
  if (!parsed.ok) {
    return problemResponse(
      validation("Query validation failed.", parsed.errors),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const page = await listSubmissionComments({
    viewerId: auth.user.id,
    submissionId: id,
    cursor: parsed.value.cursor,
    limit: clampPageLimit(parsed.value.limit),
    since: parsed.value.since,
    maxDepth: parsed.value.depth,
  });

  return ok(page);
}
