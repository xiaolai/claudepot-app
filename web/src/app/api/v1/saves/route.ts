/**
 * POST /api/v1/saves — toggle a private bookmark via PAT.
 *
 * Body: { submissionId: uuid, saved: boolean }
 *
 *   saved = true  → insert (idempotent — duplicate inserts are absorbed)
 *   saved = false → delete (idempotent — missing rows are absorbed)
 *
 * Bookmarks are private to the token's owning user; nothing in this
 * surface is publicly observable.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import { notFound, rateLimited, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { saveInputSchema, setSave } from "@/lib/votes";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "save:write");
  if (denied) return problemResponse(denied.problem);

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = saveInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Save validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const limit = await checkAndIncrement(auth.token.id, "saves");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily save limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await setSave(auth.user.id, parsed.data);

  if (!result.ok) {
    return problemResponse(
      notFound("Submission not found, or not in a saveable state."),
    );
  }

  return ok({ submissionId: parsed.data.submissionId, saved: result.saved });
}
