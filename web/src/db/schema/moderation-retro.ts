/**
 * Retroactive moderation queue (migration 0020). Comments that
 * fail-open on a model error get an entry here so the cron at
 * /api/cron/moderation-retro can retry them. See
 * dev-docs/policy-moderator-plan.md §11.
 */

import {
  index,
  pgEnum,
  pgTable,
  smallint,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";
import { sql } from "drizzle-orm";

import { targetTypeEnum } from "./enums";
import { users } from "./users";

export const moderationRetroStateEnum = pgEnum("moderation_retro_state", [
  "pending",
  "in_progress",
  "done",
  "failed",
]);

export const moderationRetroQueue = pgTable(
  "moderation_retro_queue",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    targetType: targetTypeEnum("target_type").notNull(),
    targetId: uuid("target_id").notNull(),
    authorId: uuid("author_id")
      .notNull()
      .references(() => users.id),
    triggerReason: text("trigger_reason").notNull(),
    state: moderationRetroStateEnum("state").notNull().default("pending"),
    attempts: smallint("attempts").notNull().default(0),
    lastError: text("last_error"),
    enqueuedAt: timestamp("enqueued_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    startedAt: timestamp("started_at", { withTimezone: true }),
    completedAt: timestamp("completed_at", { withTimezone: true }),
  },
  (t) => [
    index("idx_moderation_retro_queue_pending")
      .on(t.enqueuedAt)
      .where(sql`${t.state} = 'pending'`),
    index("idx_moderation_retro_queue_in_progress")
      .on(t.startedAt)
      .where(sql`${t.state} = 'in_progress'`),
    index("idx_moderation_retro_queue_target").on(
      t.targetType,
      t.targetId,
      t.enqueuedAt.desc(),
    ),
  ],
);
