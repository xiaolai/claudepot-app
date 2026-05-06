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
 * Charged against the `reads` daily bucket (consume == read).
 *
 * Requires the notification:read scope.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import { rateLimited, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import {
  markNotificationsReadForUser,
  markReadInputSchema,
} from "@/lib/notifications";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "notification:read");
  if (denied) return problemResponse(denied.problem);

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

  const limit = await checkAndIncrement(auth.token.id, "reads");
  if (!limit.ok) {
    return problemResponse(
      rateLimited(
        `Daily read limit (${limit.limit}) exceeded for this token.`,
        limit.resetAt,
      ),
    );
  }

  const result = await markNotificationsReadForUser(auth.user.id, parsed.data);
  return ok(result);
}
