/**
 * Pull-based notification inbox + per-user preference and visibility
 * lists. The `notifications.payload` jsonb shape varies by `kind`;
 * per-kind payload schemas live in lib/notifications.ts and the
 * moderator's lib/moderation/notify.ts.
 */

import {
  boolean,
  index,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";

import { notificationKindEnum } from "./enums";
import { submissions, tags } from "./content";
import { users } from "./users";

export const notifications = pgTable(
  "notifications",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    kind: notificationKindEnum("kind").notNull(),
    payload: jsonb("payload").notNull(),
    readAt: timestamp("read_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_notifications_user_unread").on(t.userId, t.createdAt.desc()),
  ],
);

/**
 * Per-user filters used as feed exclusions.
 */
export const userHiddenSubmissions = pgTable(
  "user_hidden_submissions",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    hiddenAt: timestamp("hidden_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.submissionId] })],
);

export const userTagMutes = pgTable(
  "user_tag_mutes",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    tagSlug: text("tag_slug")
      .notNull()
      .references(() => tags.slug, { onDelete: "cascade" }),
    mutedAt: timestamp("muted_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.tagSlug] })],
);

export const userEmailPrefs = pgTable("user_email_prefs", {
  userId: uuid("user_id")
    .primaryKey()
    .references(() => users.id, { onDelete: "cascade" }),
  digestWeekly: boolean("digest_weekly").notNull().default(true),
  notifyReplies: boolean("notify_replies").notNull().default(true),
  updatedAt: timestamp("updated_at", { withTimezone: true }).notNull().defaultNow(),
});

/**
 * Idempotency guard for the weekly digest cron. One row per
 * (user, ISO-week). The cron does INSERT ... ON CONFLICT DO NOTHING
 * RETURNING user_id and only emails recipients whose insert produced
 * a row. This makes the cron safe to retry: a rerun in the same week
 * cannot deliver duplicate digests.
 */
export const digestSends = pgTable(
  "digest_sends",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    // ISO-8601 week key, e.g. "2026-W18". Text not date so retries
    // across the Sun→Mon midnight boundary still collapse onto the
    // same row.
    weekKey: text("week_key").notNull(),
    sentAt: timestamp("sent_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.weekKey] })],
);
