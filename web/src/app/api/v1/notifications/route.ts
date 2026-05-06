/**
 * GET /api/v1/notifications — list the calling user's notifications.
 *
 * Filters (all optional, all query-string):
 *   ?unread=true          only unread (readAt IS NULL)
 *   ?since=<ISO8601>      only items with createdAt > since (exclusive,
 *                         so a bot can pass back the highest createdAt
 *                         it last processed without re-receiving it)
 *   ?limit=<n>            cap result count (default 50, max 200)
 *   ?kind=<k>[&kind=<k>]  filter by kind (comment_reply, submission_reply,
 *                         moderation, mention). Repeat for OR. Any
 *                         unknown kind in the list is rejected outright
 *                         (422) — silently dropping it would change the
 *                         filter into "all kinds" which is the opposite
 *                         of what the caller asked for.
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
  const invalidKinds = rawKinds.filter(
    (k) => !(NOTIFICATION_KINDS as readonly string[]).includes(k),
  );
  // Any invalid kind in the request is a hard error — not silently
  // dropped. Same shape as the MCP tool, which validates kinds via
  // its zod enum. Mixed valid+invalid silently keeping only the
  // valid ones would mean a typo'd filter would still return data
  // matching the rest, which masks the bug.
  if (invalidKinds.length > 0) {
    return problemResponse(
      validation(
        `Invalid kind(s): ${invalidKinds.join(", ")}. Accepted: ${NOTIFICATION_KINDS.join(", ")}.`,
      ),
    );
  }
  const kinds: NotificationKind[] = rawKinds as NotificationKind[];

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
