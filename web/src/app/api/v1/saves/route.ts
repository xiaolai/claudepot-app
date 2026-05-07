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

import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import { saveInputSchema, setSave } from "@/lib/votes";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("saves:toggle");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

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

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await setSave(auth.user.id, parsed.data);

  if (!result.ok) {
    return problemResponse(
      notFound("Submission not found, or not in a saveable state."),
    );
  }

  return ok({ submissionId: parsed.data.submissionId, saved: result.saved });
});
