/**
 * POST /api/v1/notifications/mark-read — mark notifications as read.
 *
 * Body (one of):
 *   { "ids": ["uuid", ...] }   mark exactly those (max 500 per call)
 *   { "all": true }            mark every unread row for this user
 *
 * Idempotent: rows already read are not double-counted. The response
 * `updated` is the number of rows that flipped from unread → read on
 * THIS call; clients can use it to detect "nothing changed" without a
 * second list call.
 *
 * Scope and bucket from the manifest — `notification:read` + reads.
 * Mark-read is treated as a read because consume == read.
 */

import { validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse , withErrorHandling } from "@/lib/api/response";
import {
  markNotificationsReadForUser,
  markReadInputSchema,
} from "@/lib/notifications";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("notifications:mark_read");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = markReadInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Mark-read validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join("."),
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await markNotificationsReadForUser(auth.user.id, parsed.data);
  return ok(result);
});
