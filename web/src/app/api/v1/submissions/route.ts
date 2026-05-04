/**
 * POST /api/v1/submissions — create a submission via PAT.
 *
 * Mirrors the web UI's submitPost server action but takes the author
 * identity from a Bearer-token-authenticated request instead of a
 * cookie session. The resulting row is marked submitterKind='scout'
 * with sourceId=token.displayPrefix so admins can trace the
 * submission back to the token that minted it.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import { rateLimited, validation } from "@/lib/api/errors";
import { created, ok, preflight, problemResponse } from "@/lib/api/response";
import {
  createSubmission,
  submissionInputSchema,
} from "@/lib/submissions";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "submission:write");
  if (denied) return problemResponse(denied.problem);

  // Parse + validate the body BEFORE incrementing the rate-limit bucket.
  // Otherwise malformed/invalid requests consume daily quota even though
  // they can never produce a submission — letting buggy clients DoS
  // themselves out of their own write budget.
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

  const limit = await checkAndIncrement(auth.token.id, "submissions");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily submission limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

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
