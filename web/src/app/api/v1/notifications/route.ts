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
 * Scope and bucket policy come from the manifest — `notification:read`,
 * not `read:all`, because notifications are private per-recipient.
 */

import { validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import {
  listNotificationsForUser,
  listNotificationsInputSchema,
  NOTIFICATION_KINDS,
  type NotificationKind,
} from "@/lib/notifications";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(req: Request): Promise<Response> {
  const SPEC = endpointSpec("notifications:list");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

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

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const result = await listNotificationsForUser(auth.user.id, parsed.data);
  return ok(result);
}
