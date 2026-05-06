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

import { and, eq, gte, inArray, isNull } from "drizzle-orm";
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
  // Inclusive lower bound on createdAt for incremental polling.
  // Bots persist the highest createdAt they've seen and pass it back
  // here so they don't re-pull the whole inbox.
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
};

export async function listNotificationsForUser(
  userId: string,
  input: ListNotificationsInput,
): Promise<ListNotificationsResult> {
  const limit = input.limit ?? DEFAULT_LIMIT;
  const conditions = [eq(notifications.userId, userId)];
  if (input.unreadOnly) conditions.push(isNull(notifications.readAt));
  if (input.since) conditions.push(gte(notifications.createdAt, new Date(input.since)));
  if (input.kinds && input.kinds.length > 0) {
    conditions.push(inArray(notifications.kind, input.kinds));
  }

  const rows = await db
    .select({
      id: notifications.id,
      kind: notifications.kind,
      payload: notifications.payload,
      createdAt: notifications.createdAt,
      readAt: notifications.readAt,
    })
    .from(notifications)
    .where(and(...conditions))
    .orderBy(notifications.createdAt)
    .limit(limit);

  // Reverse to newest-first AFTER the SQL — ordering ASC keeps
  // pagination stable when callers iterate with `since`.
  rows.reverse();

  // Unread count is over the FULL inbox, not the filtered slice — bots
  // need the inbox-wide unread count to decide whether to keep polling
  // even if their current filter is empty.
  const unreadRows = await db
    .select({ id: notifications.id })
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
    unreadCount: unreadRows.length,
  };
}

/* ── Mark read ──────────────────────────────────────────────────── */

export const markReadInputSchema = z
  .object({
    ids: z.array(z.uuid()).max(500).optional(),
    all: z.boolean().optional(),
  })
  .refine((v) => v.all === true || (v.ids !== undefined && v.ids.length > 0), {
    message: "Provide either ids[] or all=true.",
  });

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

/* ── Helper retained for compatibility with the existing web page ── */

/** Fully-typed: mark all unread as read for a single user. Used by
 * the web notifications page on view. The API surface uses
 * markNotificationsReadForUser({all:true}) instead. */
export async function markAllReadForUser(userId: string): Promise<number> {
  const updated = await db
    .update(notifications)
    .set({ readAt: new Date() })
    .where(and(eq(notifications.userId, userId), isNull(notifications.readAt)))
    .returning({ id: notifications.id });
  return updated.length;
}

