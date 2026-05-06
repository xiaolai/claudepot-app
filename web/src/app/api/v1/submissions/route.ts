/**
 * /api/v1/submissions
 *
 *   GET  — list approved submissions (cursor-paginated, filterable).
 *   POST — create a submission via PAT. Mirrors the web UI's
 *          submitPost server action; the resulting row is marked
 *          submitterKind='scout' with sourceId=token.id so admins
 *          can trace the submission back to the token that minted it.
 *
 * Scope and rate-limit policy are declared in lib/api/manifest.ts
 * and consumed via endpointSpec() — string literals for scope/bucket
 * are deliberately absent from this file.
 */

import { validation } from "@/lib/api/errors";
import { created, ok, preflight, problemResponse } from "@/lib/api/response";
import {
  createSubmission,
  submissionInputSchema,
} from "@/lib/submissions";
import { parseSubmissionListParams } from "@/lib/api/inputs";
import { listSubmissions } from "@/lib/api/queries";
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

export async function GET(req: Request): Promise<Response> {
  const SPEC = endpointSpec("submissions:list");
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

  // Citizens cannot see pending content. The route silently clamps
  // — the PRD calls this out explicitly so a forward-looking client
  // doesn't break when the param shows up in docs.
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
    authorUsername: parsed.value.authorUsername,
    state: effectiveState,
  });

  return ok(page);
}

export async function POST(req: Request): Promise<Response> {
  const SPEC = endpointSpec("submissions:create");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  // Parse + validate BEFORE incrementing the rate-limit bucket.
  // Malformed/invalid requests must not consume daily quota — that
  // would let buggy clients DoS themselves out of their own write
  // budget.
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = submissionInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Submission validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await createSubmission(auth.user.id, parsed.data, {
    surface: "api",
    tokenId: auth.token.id,
    tokenPrefix: auth.token.displayPrefix,
  });

  if (!result.ok) {
    if (result.reason === "duplicate") {
      // 200 + payload pointing at the existing row is friendlier than 409
      // for clients that don't care about the distinction. Includes the
      // existing id so they can navigate to it.
      return ok({
        duplicate: true,
        existingId: result.existingId,
        message: "A submission with this URL was created in the last 30 days.",
      });
    }
    if (result.reason === "locked") {
      return problemResponse({
        type: "https://claudepot.com/api/errors/forbidden",
        title: "Forbidden",
        status: 403,
        detail: result.detail ?? "Account is locked.",
      });
    }
    if (result.reason === "rate") {
      // Rung 3 of the ban ladder — the author's daily cap dropped
      // after recent moderation rejects. Reset is at UTC midnight.
      const utcMidnight = new Date();
      utcMidnight.setUTCHours(24, 0, 0, 0);
      return problemResponse({
        type: "https://claudepot.com/api/errors/rate-limit",
        title: "Daily cap reached",
        status: 429,
        detail: result.detail ?? "Daily cap reached.",
      });
    }
    return problemResponse(validation(result.detail ?? "Submission failed."));
  }

  return created(
    {
      id: result.submissionId,
      pending: result.pending,
      url: `https://claudepot.com/post/${result.submissionId}`,
    },
    `https://claudepot.com/post/${result.submissionId}`,
  );
}
