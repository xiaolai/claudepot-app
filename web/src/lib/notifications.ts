/**
 * Core notifications inbox operations.
 *
 * Two surfaces consume these:
 *
 *   - Web UI (app/(reader)/notifications/page.tsx) reads the inbox
 *     server-side from the cookie session. Today it auto-marks
 *     unread on view; that path can keep its inline query — these
 *     cores exist for the API surface where "explicit consume"
 *     matters more than "auto-mark on visit".
 *   - REST GET /api/v1/notifications + POST .../mark-read for bots.
 *
 * Auth happens at each surface's boundary. These functions trust the
 * userId they're given.
 */

import { and, asc, count, desc, eq, gt, inArray, isNull } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { notifications } from "@/db/schema";

/* ── Notification kind enum (mirrors notificationKindEnum) ──────── */

export const NOTIFICATION_KINDS = [
  "comment_reply",
  "submission_reply",
  "moderation",
  "mention",
] as const;

export type NotificationKind = (typeof NOTIFICATION_KINDS)[number];

/* ── List ───────────────────────────────────────────────────────── */

const DEFAULT_LIMIT = 50;
const MAX_LIMIT = 200;

export const listNotificationsInputSchema = z.object({
  unreadOnly: z.boolean().optional(),
  // Exclusive lower bound on createdAt for incremental polling.
  // Bots persist the highest createdAt they've seen and pass it back
  // here; using `>` (not `>=`) means the boundary item is not
  // re-delivered. Same-millisecond ties are vanishingly rare for a
  // single user's inbox; if they ever matter, switch to a composite
  // (createdAt, id) cursor.
  since: z.iso.datetime().optional(),
  limit: z.coerce.number().int().min(1).max(MAX_LIMIT).optional(),
  // Filter to specific kinds; omitted = all kinds.
  kinds: z.array(z.enum(NOTIFICATION_KINDS)).max(NOTIFICATION_KINDS.length).optional(),
});

export type ListNotificationsInput = z.infer<typeof listNotificationsInputSchema>;

export type NotificationDto = {
  id: string;
  kind: NotificationKind;
  payload: unknown;
  createdAt: string;
  readAt: string | null;
};

export type ListNotificationsResult = {
  items: NotificationDto[];
  unreadCount: number;
  /**
   * True iff there are more rows beyond the returned page. When the
   * caller is polling with `since`, advance it to the newest returned
   * item's createdAt and re-poll until hasMore=false.
   */
  hasMore: boolean;
};

export async function listNotificationsForUser(
  userId: string,
  input: ListNotificationsInput,
): Promise<ListNotificationsResult> {
  const limit = input.limit ?? DEFAULT_LIMIT;
  const conditions = [eq(notifications.userId, userId)];
  if (input.unreadOnly) conditions.push(isNull(notifications.readAt));
  if (input.since) conditions.push(gt(notifications.createdAt, new Date(input.since)));
  if (input.kinds && input.kinds.length > 0) {
    conditions.push(inArray(notifications.kind, input.kinds));
  }

  // Two order modes:
  //   - Initial fetch (no `since`): newest first for inbox UX.
  //   - Incremental polling (`since` present): oldest first so a
  //     caller can advance `since` to the newest returned item and
  //     deterministically drain an overflowed window across polls.
  //
  // Both modes use LIMIT N+1 to surface `hasMore` without a second
  // count query.
  const rowsPlusOne = await db
    .select({
      id: notifications.id,
      kind: notifications.kind,
      payload: notifications.payload,
      createdAt: notifications.createdAt,
      readAt: notifications.readAt,
    })
    .from(notifications)
    .where(and(...conditions))
    .orderBy(
      ...(input.since
        ? [asc(notifications.createdAt), asc(notifications.id)]
        : [desc(notifications.createdAt), desc(notifications.id)]),
    )
    .limit(limit + 1);
  const hasMore = rowsPlusOne.length > limit;
  const rows = hasMore ? rowsPlusOne.slice(0, limit) : rowsPlusOne;

  // Unread count is over the FULL inbox, not the filtered slice — bots
  // need the inbox-wide unread count to decide whether to keep polling
  // even if their current filter is empty. Aggregate count() so a
  // 10k-unread inbox doesn't materialize 10k rows just to .length them.
  const [{ n: unreadCount } = { n: 0 }] = await db
    .select({ n: count() })
    .from(notifications)
    .where(and(eq(notifications.userId, userId), isNull(notifications.readAt)));

  return {
    items: rows.map((r) => ({
      id: r.id,
      kind: r.kind,
      payload: r.payload,
      createdAt: r.createdAt.toISOString(),
      readAt: r.readAt?.toISOString() ?? null,
    })),
    unreadCount,
    hasMore,
  };
}

/* ── Mark read ──────────────────────────────────────────────────── */

// True XOR: exactly one of {ids non-empty, all === true}. The previous
// shape allowed { all: true, ids: [...] } and silently marked the whole
// inbox while looking like an id-targeted call — that's a footgun for
// any bot that builds the request dynamically.
export const markReadInputSchema = z
  .object({
    ids: z.array(z.uuid()).max(500).optional(),
    all: z.boolean().optional(),
  })
  .refine(
    (v) => {
      const hasIds = v.ids !== undefined && v.ids.length > 0;
      const hasAll = v.all === true;
      return hasIds !== hasAll;
    },
    {
      message: "Provide exactly one of: ids[] (non-empty) OR all=true.",
    },
  );

export type MarkReadInput = z.infer<typeof markReadInputSchema>;

export type MarkReadResult = { updated: number };

export async function markNotificationsReadForUser(
  userId: string,
  input: MarkReadInput,
): Promise<MarkReadResult> {
  // Set readAt only on currently-unread rows so RETURNING reports the
  // true number of newly-marked rows, not "rows that match the
  // selector". Lets API clients show "0 / N already read" cleanly.
  const baseCond = and(
    eq(notifications.userId, userId),
    isNull(notifications.readAt),
  );
  const where = input.all
    ? baseCond
    : and(baseCond, inArray(notifications.id, input.ids!));

  const updated = await db
    .update(notifications)
    .set({ readAt: new Date() })
    .where(where)
    .returning({ id: notifications.id });

  return { updated: updated.length };
}

/* ── Helper retained for the existing web page ────────────────── */

/** Mark every unread notification for a user as read. Thin shim over
 * `markNotificationsReadForUser({ all: true })` for callers that
 * don't care about the count, used by the web notifications page on
 * view. New API/MCP callers go through the typed core. */
export async function markAllReadForUser(userId: string): Promise<number> {
  const { updated } = await markNotificationsReadForUser(userId, { all: true });
  return updated;
}
