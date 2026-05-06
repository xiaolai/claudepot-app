/**
 * GET /api/v1/notifications — list the calling user's notifications.
 *
 * Filters (all optional, all query-string):
 *   ?unread=true          only unread (readAt IS NULL)
 *   ?since=<ISO8601>      only items with createdAt >= since (inclusive)
 *   ?limit=<n>            cap result count (default 50, max 200)
 *   ?kind=<k>[&kind=<k>]  filter by kind (comment_reply, submission_reply,
 *                         moderation, mention). Repeat for OR.
 *
 * Always includes `unreadCount` over the FULL inbox so polling clients
 * can decide whether to keep polling even when their filtered slice
 * comes back empty.
 *
 * Charged against the `reads` daily bucket (10000/day default — large
 * enough for poll-every-minute bot loops).
 *
 * Requires the notification:read scope. NOT covered by read:all
 * because notifications are private per-recipient, not the public
 * surface read:all unlocks.
 */

import { authenticate, requireScope } from "@/lib/api/auth";
import { checkAndIncrement } from "@/lib/api/rate-limit";
import { rateLimited, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import {
  listNotificationsForUser,
  listNotificationsInputSchema,
  NOTIFICATION_KINDS,
  type NotificationKind,
} from "@/lib/notifications";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(req: Request): Promise<Response> {
  const auth = await authenticate(req);
  if (!auth.ok) return problemResponse(auth.problem);

  const denied = requireScope(auth.token, "notification:read");
  if (denied) return problemResponse(denied.problem);

  const url = new URL(req.url);
  const rawKinds = url.searchParams.getAll("kind");
  // Filter unknown kinds out at the boundary so the schema's z.enum
  // sees only valid values; bots passing an obsolete kind shouldn't
  // produce a 422 — they should just see no results for that kind.
  const kinds: NotificationKind[] = rawKinds.filter(
    (k): k is NotificationKind =>
      (NOTIFICATION_KINDS as readonly string[]).includes(k),
  );

  const parsed = listNotificationsInputSchema.safeParse({
    unreadOnly: url.searchParams.get("unread") === "true",
    since: url.searchParams.get("since") ?? undefined,
    limit: url.searchParams.get("limit") ?? undefined,
    kinds: kinds.length > 0 ? kinds : undefined,
  });
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Query validation failed.",
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

  const result = await listNotificationsForUser(auth.user.id, parsed.data);
  return ok(result);
}
