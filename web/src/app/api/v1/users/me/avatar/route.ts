/**
 * POST   /api/v1/users/me/avatar — upload + set the caller's avatar.
 * DELETE /api/v1/users/me/avatar — clear the caller's avatar.
 *
 * Body for POST: multipart/form-data with field `avatar` (the file).
 * Allowed types: image/png, image/jpeg, image/webp. Max 2 MB. The
 * server verifies magic bytes against the declared content-type.
 *
 * Authorization: PAT scope `avatar:write` AND the route always
 * targets `auth.user.id` — there is no `target_user_id` field, so a
 * leaked token can change exactly one avatar (its own).
 */

import { forbidden, validation } from "@/lib/api/errors";
import {
  noContent,
  ok,
  preflight,
  problemResponse,
  withErrorHandling,
} from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import {
  ALLOWED_AVATAR_TYPES,
  clearAvatar,
  MAX_AVATAR_BYTES,
  setAvatar,
} from "@/lib/avatars";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("users:set_avatar");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  // Multipart parsing happens via the standard Web FormData API.
  // Next.js's App Router request supports req.formData() for any
  // multipart/form-data body. Non-multipart bodies throw — surface
  // as 422 instead of 500.
  let formData: FormData;
  try {
    formData = await req.formData();
  } catch {
    return problemResponse(
      validation(
        "Request body must be multipart/form-data with field 'avatar'.",
      ),
    );
  }

  const file = formData.get("avatar");
  if (!(file instanceof File)) {
    return problemResponse(
      validation("Missing form field 'avatar' (must be a file)."),
    );
  }

  // Pre-check size before reading the bytes — a 50 MB upload would
  // otherwise pull the whole stream into memory before we reject it.
  if (file.size > MAX_AVATAR_BYTES) {
    return problemResponse(
      validation(
        `Avatar must be ≤ ${MAX_AVATAR_BYTES} bytes. Got ${file.size}.`,
      ),
    );
  }
  if (!ALLOWED_AVATAR_TYPES.includes(file.type as (typeof ALLOWED_AVATAR_TYPES)[number])) {
    return problemResponse(
      validation(
        `Content type must be one of: ${ALLOWED_AVATAR_TYPES.join(", ")}. Got ${file.type || "(none)"}.`,
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const bytes = new Uint8Array(await file.arrayBuffer());
  const result = await setAvatar(auth.user.id, bytes, file.type);
  if (!result.ok) {
    if (result.reason === "user_not_found") {
      // The token references a user that's been deleted between
      // auth and the avatar write. Surface as 401 — the token's
      // identity is invalid going forward.
      return problemResponse(
        forbidden("Token references a deleted user."),
      );
    }
    return problemResponse(
      validation(result.detail ?? "Avatar validation failed."),
    );
  }

  return ok({ avatarUrl: result.url });
});

export const DELETE = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("users:clear_avatar");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await clearAvatar(auth.user.id);
  if (!result.ok) {
    return problemResponse(forbidden("Token references a deleted user."));
  }
  return noContent();
});
